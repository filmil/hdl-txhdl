// SPDX-License-Identifier: Apache-2.0
//! The reference: RV32IMAC as a program, one `step` per instruction,
//! written against the decoder and nothing else. The core is checked
//! against it, in lockstep, every cycle.
use crate::core::{DRAM_BASE, DRAM_BYTES, IMEM_BYTES};
use crate::isa::{
    compressed, decode, is_compressed, Kind, CAUSE_BREAKPOINT,
    CAUSE_FETCH_ACCESS, CAUSE_FETCH_PAGE, CAUSE_ILLEGAL, CAUSE_LOAD_ACCESS,
    CAUSE_LOAD_MISALIGNED, CAUSE_LOAD_PAGE, CAUSE_MEXT, CAUSE_MSOFT,
    CAUSE_MTIMER, CAUSE_SEXT, CAUSE_SSOFT, CAUSE_STIMER, CAUSE_STORE_ACCESS,
    CAUSE_STORE_MISALIGNED, CAUSE_STORE_PAGE, CLINT_BASE, CLINT_MASK,
    CSR_CYCLE, CSR_CYCLEH, CSR_DCSR, CSR_DPC, CSR_INSTRET, CSR_INSTRETH,
    CSR_MARCHID, CSR_MBUSQUIET, CSR_MCAUSE, CSR_MCOUNTEREN, CSR_MCYCLE,
    CSR_MCYCLEH, CSR_MEDELEG, CSR_MENVCFG, CSR_MENVCFGH, CSR_MEPC, CSR_MHALT,
    CSR_MHARTID, CSR_MIDELEG, CSR_MIE, CSR_MIMPID, CSR_MINSTRET, CSR_MINSTRETH,
    CSR_MIP, CSR_MISA, CSR_MSCRATCH, CSR_MSTATUS, CSR_MSTATUSH, CSR_MTVAL,
    CSR_MTVEC, CSR_MVENDORID, CSR_SATP, CSR_SCAUSE, CSR_SCOUNTEREN,
    CSR_SENVCFG, CSR_SEPC, CSR_SIE, CSR_SIP, CSR_SSCRATCH, CSR_SSTATUS,
    CSR_STVAL, CSR_STVEC, CSR_TIME, CSR_TIMEH, MEXT, MISA, MSOFT, MTIMECMP_OFF,
    MTIMER, SEXT, SSOFT, STIMER, UART_BASE,
};
use txhdl_parts::mmu::{translate, Access, Fault, Mode};

/// Where data memory begins and how much there is, in bytes. The
/// program lives at zero, in the boot memory; the two do not overlap,
/// and a load from the boot memory reads it, since it is on the bus
/// (issue 268).
pub const DATA_BASE: u32 = 0x1000;
pub const DATA_BYTES: u32 = 4096;

/// What stops the model: the program writing `mhalt`, or a fault. An
/// illegal instruction, `ecall` and `ebreak` do not stop it: they
/// trap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Halt {
    Break,
    Fault(u32),
}

/// The machine-mode control and status registers the core keeps, and
/// `mbusquiet`. mstatus holds two bits, MIE and MPIE; mtvec is a
/// direct-mode base.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Csr {
    pub mstatus: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mie: u32,
    pub mip: u32,
    pub mtval: u32,
    /// `mbusquiet`, bit 0 (issue 417).
    pub busquiet: bool,
    /// User and supervisor mode (issue 1012): the delegations, the
    /// supervisor's pending bits that software sets, its registers,
    /// and the counter enables.
    pub medeleg: u32,
    pub mideleg: u32,
    pub mip_sw: u32,
    pub stvec: u32,
    pub sscratch: u32,
    /// `menvcfg` and `senvcfg`: `FIOM` alone (issue 1348).
    pub menvcfg: u32,
    pub senvcfg: u32,
    pub sepc: u32,
    pub scause: u32,
    pub stval: u32,
    pub satp: u32,
    pub mcounteren: u32,
    pub scounteren: u32,
}

/// A machine around the model: the memories and devices its fetches,
/// loads and stores reach, by word address (issue 1016). The model does
/// its sub-word merging for loads; a store goes out as a word and a mask
/// of the bits it writes, so a device is not read on the way to being
/// written. `&self`, since a device keeps its state behind a cell.
pub trait Bus: std::fmt::Debug {
    /// The word at `addr`, word-aligned, or `None` where nothing
    /// answers.
    fn load(&self, addr: u32) -> Option<u32>;
    /// The bits of `v` that `mask` names, written at `addr`,
    /// word-aligned; `false` where nothing answers.
    fn store(&self, addr: u32, v: u32, mask: u32) -> bool;
}

/// Translations the machine has made, for the fast machine alone (issue
/// 1132): 256 entries, direct mapped by the virtual page and the access,
/// each tagged with everything a translation depends on, so a hit is the
/// walk's own answer. Only a translation that succeeded is kept, so a
/// fault always walks. A write of `satp`, an `sfence.vma` and a reset
/// empty it, as they would a hart's. The model in lockstep with the core
/// runs without a bus and so without this: there the reference walks
/// every access.
#[derive(Clone, Debug, Default)]
pub struct Tlb {
    entries: Vec<Option<TlbEntry>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TlbEntry {
    vpn: u32,
    satp: u32,
    prv: u8,
    sum: bool,
    mxr: bool,
    access: u8,
    ppn: u32,
}

impl Tlb {
    const SIZE: usize = 256;

    fn key(vpn: u32, access: Access) -> usize {
        (vpn as usize ^ (access as usize * 0x55)) & (Self::SIZE - 1)
    }

    fn get(&self, vpn: u32, m: &Mode, access: Access) -> Option<u32> {
        let e = (*self.entries.get(Self::key(vpn, access))?)?;
        let hit = e.vpn == vpn
            && e.satp == m.satp
            && e.prv == m.prv
            && e.sum == m.sum
            && e.mxr == m.mxr
            && e.access == access as u8;
        hit.then_some(e.ppn)
    }

    fn put(&mut self, vpn: u32, m: &Mode, access: Access, ppn: u32) {
        if self.entries.is_empty() {
            self.entries = vec![None; Self::SIZE];
        }
        self.entries[Self::key(vpn, access)] = Some(TlbEntry {
            vpn,
            satp: m.satp,
            prv: m.prv,
            sum: m.sum,
            mxr: m.mxr,
            access: access as u8,
            ppn,
        });
    }

    /// Every translation forgotten.
    pub fn flush(&mut self) {
        self.entries.clear();
    }
}

/// The hart's own translation buffers as the timing mode counts them
/// (issue 1561): the RTL's `Mmu8` (`txhdl_parts::mmu`), one for fetches
/// and one for loads and stores, each of [`HwTlb::ENTRIES`] entries,
/// fully associative and filled in turn, an entry a 4 KiB page or a 4
/// MiB megapage. [`Tlb`] answers the translations and is far larger, so
/// a walk the hart makes is one these miss, not one it misses. They
/// keep where a page is, not what it allows, since the RTL checks that
/// on every hit.
#[derive(Clone, Debug, Default)]
pub struct HwTlb {
    /// Per port, fetches first: each entry's page or megapage number,
    /// and whether it is a megapage.
    entries: [[Option<(u32, bool)>; HwTlb::ENTRIES]; 2],
    /// Per port, the entry to fill next.
    next: [usize; 2],
    /// The page the fetch last had translated, which the core keeps
    /// beside the buffers (`ft_vpn`): a fetch from another page asks the
    /// unit again, and waits for its answer.
    fetch_page: Option<u32>,
}

impl HwTlb {
    pub const ENTRIES: usize = 8;

    fn port(access: Access) -> usize {
        !matches!(access, Access::Fetch) as usize
    }

    /// Whether the buffer for `access` holds `va`'s page.
    pub fn holds(&self, va: u32, access: Access) -> bool {
        self.entries[Self::port(access)]
            .iter()
            .flatten()
            .any(|&(n, mega)| n == if mega { va >> 22 } else { va >> 12 })
    }

    /// `va`'s page, or its megapage, into the next entry for `access`.
    pub fn fill(&mut self, va: u32, mega: bool, access: Access) {
        let p = Self::port(access);
        let n = if mega { va >> 22 } else { va >> 12 };
        self.entries[p][self.next[p]] = Some((n, mega));
        self.next[p] = (self.next[p] + 1) % Self::ENTRIES;
    }

    /// Both emptied, as a write of `satp` or an `sfence.vma` empties them.
    pub fn flush(&mut self) {
        *self = HwTlb::default();
    }

    /// Whether a fetch at `va` moves to a page other than the one the
    /// fetch last had translated, which it then has.
    pub fn fetch_moves(&mut self, va: u32) -> bool {
        let moved = self.fetch_page != Some(va >> 12);
        self.fetch_page = Some(va >> 12);
        moved
    }

    /// The fetch's page forgotten, as the core forgets it on a trap, a
    /// return and a `fence.i` (`vctx`), so the next fetch asks again.
    pub fn forget_fetch(&mut self) {
        self.fetch_page = None;
    }
}

/// The architectural state, and only that.
#[derive(Clone, Debug)]
pub struct Model {
    /// The machine the model runs in, when it is one: every fetch, load
    /// and store goes to it. `None` is the board as the lockstep test
    /// holds the core to it, its memories the model's own.
    pub bus: Option<std::rc::Rc<dyn Bus>>,
    /// The fast machine's translations; empty and unused without a bus.
    pub tlb: std::cell::RefCell<Tlb>,
    /// The hart's translation buffers, which say what the timing mode
    /// charges for walks.
    pub hw_tlb: std::cell::RefCell<HwTlb>,
    pub pc: u32,
    pub x: [u32; 32],
    pub mem: Vec<u32>,
    /// The data RAM on the core's own port (issue 1275), 64 KiB at
    /// DRAM_BASE: loads and stores reach it, a fetch and the page
    /// walker do not, as in the core.
    pub dram: Vec<u32>,
    pub csr: Csr,
    /// What the bus answered the core's last load from a device: the
    /// model has neither a clock nor a bus, so the caller sets it
    /// before the step of a device load. The timer's compare and the
    /// bytes the serial port was given are the model's own.
    pub dev_word: u32,
    /// Whether the bus refused that load: the answer came back as an
    /// error, and the load traps instead of writing (issue 417).
    pub dev_err: bool,
    /// The hart's number, which `mhartid` reads (issue 1408).
    pub hartid: u32,
    pub mtimecmp: u64,
    /// Instructions retired. The core counts the same thing in
    /// `minstret`, and this model counts it by stepping, so the two
    /// can be compared; the cycles beside it cannot, which
    /// `csr_read` says.
    pub minstret: u64,
    pub uart: Vec<u8>,
    /// The timer's line as the core saw it, set by the caller with the
    /// count; the timer is a device on the bus, so its pending bit is
    /// what the line says, not what the model could compute.
    pub tirq: bool,
    /// The software interrupt the program raised for itself, which is
    /// bit 0 of `msip` in the interrupt controller.
    pub msip: bool,
    pub halted: Option<Halt>,
    /// Debug mode (issue 154): halted by a debugger and resumable. The
    /// core enters it before an instruction, on a halt request, on the
    /// instruction after a single step, or on an `ebreak` that
    /// `dcsr.ebreakm` sends here; the harness tells the model when the
    /// core has, since the model steps only on a retirement and an
    /// entry retires nothing.
    pub debug: bool,
    /// The instruction to execute on resume.
    pub dpc: u32,
    /// `dcsr` as read: version 4, `ebreakm`, the cause, `step`, privilege 3.
    pub dcsr: u32,
    /// A single step: armed by a resume with `dcsr.step`, and once one
    /// instruction has run, stepped, which asks to enter again.
    pub step_armed: bool,
    pub stepped: bool,
    /// The reservation `lr.w` makes and `sc.w` uses up: the word's byte
    /// address, or none (issue 1010).
    pub rsv: Option<u32>,
    /// The privilege the hart runs at: 3 machine, 1 supervisor, 0 user
    /// (issue 1012).
    pub prv: u32,
    /// The timer's count as the core saw it, which `time` reads: the
    /// caller sets it, as it sets the device word.
    pub time: u64,
    /// The exceptions taken, counted by cause, which a test reads to
    /// say what its programs exercised (issue 1014).
    pub causes: [u32; 16],
    /// The costs a step is charged in the timing mode (issue 1392), or
    /// none, when a step is an instruction and `mcycle` reads zero.
    pub timing: Option<Timing>,
    /// The cycles charged so far in the timing mode, which `mcycle`
    /// and `cycle` read there.
    pub cycles: u64,
    /// The step's data access, if it made one: its address, and
    /// whether it was a store.
    pub access: std::cell::Cell<Option<(u32, bool)>>,
    /// The physical word the last step wrote, a store, `sc.w` or an AMO,
    /// and the reservation's physical address: what a machine of two
    /// harts compares to take one hart's reservation when the other
    /// writes its word (issue 1408).
    pub wrote: Option<u32>,
    pub rsv_pa: Option<u32>,
    /// A window of the DDR3 the model keeps itself, its base and its
    /// words, for a harness with no bus (issue 1443): a load there is
    /// the model's own word, not what the core said the bus answered,
    /// so a wrong word from the core's data cache disagrees with it.
    pub ddr: Option<(u32, Vec<u32>)>,
    /// The step's exclusive access, `lr.w` 1, `sc.w` 2 or an AMO 3, and its
    /// physical word, for the timing mode (issue 1554).
    pub excl: std::cell::Cell<Option<(u8, u32)>>,
    /// The data cache's lines in the timing mode: each the line's address
    /// plus one, or zero when empty (issue 1554).
    pub dc: Vec<u32>,
    /// Whether the step was a `wfi`, and the cycle count at the last one;
    /// and whether the other hart runs beside this one, which the
    /// machine sets (issue 1554).
    pub wfi_now: std::cell::Cell<bool>,
    pub last_wfi: u64,
    pub contended: bool,
    /// Whether the scanout shows a frame from the DDR3 (issue 1561),
    /// which the machine sets before each step, as `contended`.
    pub scan_on: bool,
    /// The instruction cache's lines in the timing mode, each the line's
    /// address plus one, the line the last instruction was in, and
    /// whether control fell through to this one (issue 1561).
    pub ic: Vec<u32>,
    pub last_line: u32,
    pub fell: bool,
    /// The lines the timing mode filled, each one it charged a miss or
    /// the word after a jump's fill for: what the core's
    /// `mhpmcounter6`, its fills kept, counts (#1760).
    pub ic_fills: u64,
    /// What the step was for the timing mode: a multiply 1 or a division
    /// 2; the registers it reads; the register a load of it wrote; the
    /// page table entries its walks read; whether it was `fence.i` (issue 1561); its
    /// length in bytes (#1679).
    pub mkind: std::cell::Cell<u8>,
    pub ilen: std::cell::Cell<u32>,
    /// Whether the step was a backward branch (#1752).
    pub back: std::cell::Cell<bool>,
    pub reads: std::cell::Cell<(u32, u32)>,
    pub loaded: std::cell::Cell<u32>,
    pub prev_loaded: u32,
    pub walks: std::cell::Cell<u32>,
    /// The waits for a translation the step made (issue 1561), but a
    /// store's: a load translated, and a translated fetch from a new page.
    pub xlates: std::cell::Cell<u32>,
    /// A store's wait for a translation (issue 1561), which
    /// `Timing::xlate_store` charges, or `xlate_amo` when the step was an
    /// AMO; `sc.w` and each AMO are translated once, as a store.
    pub xlate_stores: std::cell::Cell<u32>,
    pub ifence: std::cell::Cell<bool>,
    /// Whether the step was a `fence`, and the cycle count at the last
    /// store into the DDR3 (issue 1561).
    pub fenced: std::cell::Cell<bool>,
    pub stored_at: u64,
}

/// What the instruction cache costs in the timing mode, in cycles more
/// than a step (#1679).
#[derive(Clone, Copy, Debug)]
pub struct Icache {
    /// A line the cache does not hold, which a jump or a taken branch
    /// went to.
    pub jumped: u64,
    /// A line it does not hold, which the line before ran on into.
    pub ran_on: u64,
    /// Running on into a line it holds.
    pub step: u64,
    /// The word after a jump or a taken branch, when it lies in another
    /// line than the jump's: the core fetches it as the jump executes and
    /// redirects a cycle later, so the line is looked up, and the target's
    /// fetch waits behind it. The first is a line the cache holds; the
    /// second one it does not, which is filled and kept, since only a
    /// flush, `fence.i` or a change of translation drops a fill.
    pub after: (u64, u64),
    /// More a fill while the other hart runs, which shares the DDR3's path
    /// with this one (#1752).
    pub shared: u64,
}

/// What a step costs in the timing mode (issue 1392): a base an
/// instruction, a penalty when control does not fall through, and a
/// cost for a data access by where it goes. A fetch is charged nothing
/// more, as a hit in the instruction cache. [`Timing::board`] is
/// calibrated to the board's own figures.
#[derive(Clone, Debug)]
pub struct Timing {
    /// Cycles an instruction.
    pub base: u64,
    /// More when control goes anywhere but the next instruction.
    pub taken: u64,
    /// More for a backward branch that falls through (#1752): the core
    /// guesses a backward branch taken, and a wrong guess is corrected a
    /// cycle later (issue 1300). A forward branch taken, the other wrong
    /// guess, costs what a jump does from the cache, which `taken` is.
    pub guessed: u64,
    /// A data access's extra cycles by region: from, to, load, store.
    pub regions: Vec<(u32, u32, u64, u64)>,
    /// A load or a store anywhere else, the peripherals among it.
    pub other: (u64, u64),
    /// What `lr.w`, `sc.w` and an AMO cost, more than a step, on a word
    /// the data cache would hold, where each is an exclusive pair on the
    /// bus (issue 1554); elsewhere each is a load or a store.
    pub excl: (u64, u64, u64),
    /// A load that hits the data cache, more than a step (issue 1554):
    /// the core's, direct-mapped, 256 lines of 16 bytes, over the DDR3.
    /// A load that misses costs its region's and fills the line. None
    /// models no cache, every load its region's.
    pub hit: Option<u64>,
    /// What a store, an `sc.w` and an AMO cost more while the other hart
    /// runs and shares the DDR3's path with it (issue 1554).
    pub contention: (u64, u64, u64),
    /// What a multiply and a division or remainder cost more than a
    /// step, and an instruction that reads the register the load before
    /// it wrote (issue 1561).
    pub mul: u64,
    pub div: u64,
    pub load_use: u64,
    /// The instruction cache (issue 1561): the core's, direct-mapped,
    /// 1024 lines of 16 bytes. None models no cache, as `taken` alone
    /// did.
    pub icache: Option<Icache>,
    /// A translation the hart's buffers do not hold: a page's walk, two
    /// reads of the DDR3 (issue 1561). A megapage's walk reads one entry
    /// and is charged half, as is an entry that is not valid. The 86 is
    /// from the `board_test` netlist's simulation until the board's
    /// `costs walk-page` and `walk-mega` say otherwise.
    pub walk: u64,
    /// What the core waits for the unit's answer under translation
    /// (issue 1561): once for each load, store or AMO translated
    /// (`stall_mem`'s `dvm & !x_done`), and once each time a translated
    /// fetch moves to another page (`i_need`). Machine mode never
    /// translates a fetch, nor a load or a store unless `MPRV` sends it
    /// through `MPP`'s tables. 4, as
    /// `translation_costs_the_core_what_the_timing_mode_charges` times the
    /// RTL: a page move exactly, a load and a store 4 together with the
    /// fetch's overlap a cycle either way. The board's `costs xlate-load`
    /// and `xlate-move` say 4 too (96910caf); a store and `sc.w` are
    /// `xlate_store`, and an AMO `xlate_amo`. `lr.w` is a load.
    pub xlate: u64,
    /// What a store or `sc.w` waits for the unit's answer under translation
    /// (issue 1561). The board's `costs xlate-store` is 2 (96910caf): the
    /// store is posted, and the core goes on while part of the wait
    /// passes. The RTL's own is 3, into a memory on the core's own path
    /// (`lockstep_test`'s translation test).
    pub xlate_store: u64,
    /// What an AMO waits for the unit's answer under translation (issue
    /// 1561): nothing more, in the RTL, whose AMO stalls longer than the
    /// wait and so hides it (`lockstep_test`'s translation loops with
    /// `amoadd.w`, 12.0 cycles a round in supervisor mode against 11.9 in
    /// machine mode). `sc.w` waits as a store does, `xlate_store`: an
    /// `lr.w` and `sc.w` pair there waits 6.1 more, a load's 4 and a
    /// store's 2. The board has timed neither.
    pub xlate_amo: u64,
    /// What a load that fills a line of the data cache costs more while
    /// the scanout shows a frame (issue 1561), which reads the frame
    /// from the same DDR3 controller: the board's `costs scan-bulk-read`
    /// against `bulk-read` in the same run, 17.15 against 15.41 a word
    /// (96910caf), 1.74 a word and so about 7 a line of four. Stores are
    /// 3% dearer there, and are not charged. Nor yet is an instruction
    /// fill, nor a walk's table read: the board has timed neither with
    /// the scanout showing, and `costs scan-icache-miss` asks of the
    /// first.
    pub scan_fill: u64,
    /// What a `fence` waits, at most, for a store into the DDR3 still on
    /// its way (issue 1561): the most, right after the store, less the
    /// cycles since.
    pub fence: u64,
    /// What a `fence` costs more than a step with nothing on its way.
    pub fence_alone: u64,
}

impl Timing {
    /// The board's costs, from its own timings: `cpi.rs`'s
    /// three-instruction loop, 3004 instructions in 7036 cycles (a base
    /// of one and four for the taken branch); `ethperf.rs`'s copies,
    /// a word from or into the DDR3 in about 53 cycles with the loop
    /// round it (a DDR3 load and store about 43 together), and its
    /// note of a received frame, two DDR3 loads and the registers in
    /// 128 (a load 40, a store, posted, 3); the memories on the bus about nine
    /// a load (`board_test`'s stack load before #1275, 9.14), and the
    /// core's own data memory one (#1275).
    pub fn board() -> Self {
        Timing {
            base: 1,
            taken: 4,
            // The core on its own memories (`icache_test`'s loop with a
            // backward branch never taken), from the boot memory and from
            // the cache alike.
            guessed: 2,
            regions: vec![
                // The boot memory and the data memory, on the bus.
                (0x0000_0000, 0x0000_2000, 9, 2),
                // The core's own data memory (#1275).
                (0x0001_0000, 0x0002_0000, 1, 0),
                // The DDR3: loads wait for the controller, stores are
                // posted.
                (0x4000_0000, 0x8000_0000, 40, 3),
            ],
            other: (8, 2),
            // `board_test`'s `exclusive_costs_on_the_ddr3` (issue 1554):
            // `lr.w` 34 cycles, `lr.w` and `sc.w` 51, `amoadd.w` 50 and a
            // load the cache holds 2, a step included; with the other
            // hart storing all the while a store 0.9 more, the pair 11
            // and the AMO 8.
            excl: (33, 16, 49),
            // The cache is not charged by default: without the costs it
            // hides, a missed instruction line, a page walk, a division,
            // the board's cycles an instruction come out well under
            // (issue 1554); `machine --dcache` asks for it.
            hit: None,
            contention: (1, 11, 8),
            mul: 0,
            div: 0,
            load_use: 0,
            icache: None,
            walk: 0,
            xlate: 0,
            xlate_store: 0,
            xlate_amo: 0,
            scan_fill: 0,
            fence: 0,
            fence_alone: 0,
        }
    }

    /// The board's costs with the core's caches and its slower steps
    /// charged (issue 1561). From `costs.rs` on the board (main
    /// 0d73a9f8's flagship, the scanout running), in cycles an
    /// instruction with the step: a load the data cache holds 3.2, a
    /// store 3.4, a load that misses 43.8, `lr.w` 40.6, the pair with
    /// `sc.w` 56.1, `amoadd.w` 56.4, a `fence` 2.2 alone and 13.8 more after a
    /// store, a multiply 3.4 and a division 35.4. From `board_test`'s
    /// `core_costs_for_the_machine`, which the board has not measured: a
    /// load's word used at once 1 more. From `costs.rs`'s instruction-cache
    /// blocks on the board (#1679, flagships 6238e0c8 and 63ba1e5f), in
    /// cycles a line: a line not held that a jump went to 83.96 more than
    /// one held when the word after the jump starts a line not held, and
    /// 42.16 when that word is in the jump's own line; one the line before
    /// ran on into 42.0 more (41.4 after a `jal` to the next line);
    /// running on into a line held 3 more than four steps (straight lines
    /// 7.0). So a jumped-to line is 42, and the word after the jump costs
    /// its line's fill, 42, when that line is not held; that is charged
    /// with the line's lookup, 2 (below), so its figure is 44. A line the line
    /// before ran on into is charged its miss in place of the running on,
    /// so its figure is 42 and 3. Warm, a line of three steps whose `jal`
    /// is not its last word costs 7.0, and a line of four whose last word
    /// is a taken jump or branch 10.0: a taken transfer is the 4 of
    /// `cpi.rs`'s loop, and 2 more when the word after it is in another
    /// line, which is looked up. A `jal` to the next instruction, which
    /// the board charges as a jump, the model sees as running on. From the
    /// GUESS and CONTEND blocks on the board (#1752, flagship f40c115d): a
    /// backward branch that falls through 5 more, 4.92 a line of four, where
    /// the core on its own memories takes 2, and a fill while the other hart
    /// streams loads from the DDR3 2.57 more, charged as 3. With these the
    /// teapot's list on the model is 35.69M against the board's 37.18M, 4.0%
    /// under, and linked hot first 30.34M against 30.36M (#1752). A page walk
    /// is two loads that miss. From the translation and scanout blocks on
    /// the board (issue 1561, costs image 96910caf, flagship 93a1e1c6): a
    /// translated store waits 2, and a load that fills a data line while
    /// the scanout shows costs 7 more, 1.74 a word.
    pub fn board_cached() -> Self {
        let mut t = Timing {
            hit: Some(2),
            excl: (40, 15, 55),
            mul: 2,
            div: 34,
            load_use: 1,
            // The board's GUESS block: 4.92 a line, 5.0 a round (#1752), where
            // the core on its own memories takes 2.
            guessed: 5,
            icache: Some(Icache {
                jumped: 42,
                ran_on: 45,
                step: 3,
                after: (2, 44),
                shared: 3,
            }),
            walk: 86,
            xlate: 4,
            xlate_store: 2,
            xlate_amo: 0,
            scan_fill: 7,
            fence: 14,
            fence_alone: 1,
            ..Self::board()
        };
        for r in &mut t.regions {
            if r.0 == 0x4000_0000 {
                (r.2, r.3) = (37, 2);
            }
        }
        t
    }

    /// The extra cycles of a data access at `addr`.
    fn access(&self, addr: u32, store: bool) -> u64 {
        let (load, st) = self
            .regions
            .iter()
            .find(|(lo, hi, _, _)| (*lo..*hi).contains(&addr))
            .map(|(_, _, l, s)| (*l, *s))
            .unwrap_or(self.other);
        if store {
            st
        } else {
            load
        }
    }
}

impl Default for Model {
    fn default() -> Self {
        Model {
            bus: None,
            tlb: Default::default(),
            hw_tlb: Default::default(),
            pc: 0,
            x: [0; 32],
            mem: vec![0; DATA_BYTES as usize / 4],
            dram: vec![0; DRAM_BYTES as usize / 4],
            csr: Csr::default(),
            dev_word: 0,
            dev_err: false,
            hartid: 0,
            // All ones, as the timer's reset leaves it: a compare of zero
            // beside a count of zero is an interrupt pending from the
            // first cycle (issue 419).
            mtimecmp: u64::MAX,
            minstret: 0,
            uart: Vec::new(),
            tirq: false,
            msip: false,
            halted: None,
            debug: false,
            dpc: 0,
            timing: None,
            cycles: 0,
            access: std::cell::Cell::new(None),
            wrote: None,
            rsv_pa: None,
            ddr: None,
            excl: std::cell::Cell::new(None),
            dc: Vec::new(),
            wfi_now: std::cell::Cell::new(false),
            last_wfi: 0,
            contended: false,
            scan_on: false,
            ic: Vec::new(),
            last_line: u32::MAX,
            fell: false,
            ic_fills: 0,
            mkind: std::cell::Cell::new(0),
            ilen: std::cell::Cell::new(4),
            back: std::cell::Cell::new(false),
            reads: std::cell::Cell::new((0, 0)),
            loaded: std::cell::Cell::new(0),
            prev_loaded: 0,
            walks: std::cell::Cell::new(0),
            xlates: std::cell::Cell::new(0),
            xlate_stores: std::cell::Cell::new(0),
            ifence: std::cell::Cell::new(false),
            fenced: std::cell::Cell::new(false),
            stored_at: 0,
            dcsr: 0x4000_0003,
            step_armed: false,
            stepped: false,
            rsv: None,
            prv: 3,
            time: 0,
            causes: [0; 16],
        }
    }
}

const MIE: u32 = 1 << 3;
const MPIE: u32 = 1 << 7;
/// The rest of `mstatus` that user and supervisor mode bring (issue
/// 1012), what of it is writable, and what `sstatus` shows of it.
const SIE: u32 = 1 << 1;
const SPIE: u32 = 1 << 5;
const SPP: u32 = 1 << 8;
const MPP: u32 = 3 << 11;
const SUM: u32 = 1 << 18;
const MXR: u32 = 1 << 19;
/// Loads and stores in machine mode as the mode `MPP` names (issue
/// 1105).
const MPRV: u32 = 1 << 17;
/// The traps machine mode sets on supervisor mode's virtual memory,
/// `wfi` and `sret` (issue 1347).
const TVM: u32 = 1 << 20;
const TW: u32 = 1 << 21;
const TSR: u32 = 1 << 22;
const MSTATUS_W: u32 =
    SIE | MIE | SPIE | MPIE | SPP | MPP | MPRV | SUM | MXR | TVM | TW | TSR;
const SSTATUS_W: u32 = SIE | SPIE | SPP | SUM | MXR;
/// The exceptions machine mode may delegate: all but an environment
/// call from machine mode and the reserved causes.
const MEDELEG_W: u32 = 0xb3ff;
/// What a write leaves of `mstatus`: the writable fields, and `MPP`
/// as one of the three modes there are, a 2 read as user mode.
fn mstatus_w(v: u32) -> u32 {
    let v = v & MSTATUS_W;
    if v & MPP == 2 << 11 {
        v & !MPP
    } else {
        v
    }
}

/// The instruction at `pc` in the instruction memory, as thirty-two
/// bits, and its length in bytes; `None` past the memory's end. The
/// words are little-endian, so a halfword at an address with bit 1 set
/// is the upper half of its word, and a thirty-two bit instruction
/// there takes its upper half from the next word. A compressed
/// instruction reads as the one it stands for, and a halfword that is
/// none as itself, which decodes as illegal and is the trap value an
/// illegal instruction leaves.
pub fn fetch(imem: &[u32], pc: u32) -> Option<(u32, u32)> {
    let half = |at: u32| {
        imem.get((at / 4) as usize)
            .map(|w| (w >> (8 * (at & 2))) as u16)
    };
    let lo = half(pc)?;
    if is_compressed(lo) {
        return Some((compressed(lo).unwrap_or(lo as u32), 2));
    }
    let hi = half(pc.wrapping_add(2))?;
    Some(((hi as u32) << 16 | lo as u32, 4))
}

/// Whether an access of this kind at this address is misaligned: a
/// half wants an even address and a word a multiple of four, and a
/// byte is never misaligned. The core raises an exception rather than
/// supporting such an access, which the specification allows and issue
/// 138 asked for, so the model has to agree or the lockstep test will
/// say the core is wrong.
pub fn misaligned(kind: Kind, addr: u32) -> bool {
    use Kind::*;
    match kind {
        Lh | Lhu | Sh => addr & 1 != 0,
        Lw | Sw | LrW | ScW | AmoswapW | AmoaddW | AmoxorW | AmoandW
        | AmoorW | AmominW | AmomaxW | AmominuW | AmomaxuW => addr & 3 != 0,
        _ => false,
    }
}

/// The first address above the data memory: everything there and
/// beyond is a device's, and what a load there answers is what the bus
/// gave the core, which the caller hands over.
const DEVICES: u32 = DATA_BASE + DATA_BYTES;

/// Whether the core's data cache would hold the word at physical
/// `pa`: the DDR3's quarter of the map (issue 1554).
fn cached(pa: u32) -> bool {
    pa >> 30 == 1
}

/// Whether a data access at addr is the data RAM's (issue 1275).
fn in_dram(addr: u32) -> bool {
    addr.wrapping_sub(DRAM_BASE) < DRAM_BYTES
}

impl Model {
    /// The instruction at `pc`, from the boot memory below
    /// `IMEM_BYTES` or from the data memory above it, which is what
    /// the core does once it fetches from the bus (issue 134).
    fn fetch_at(&self, imem: &[u32], pc: u32) -> Option<(u32, u32)> {
        if let Some(bus) = &self.bus {
            let half = |at: u32| -> Option<u16> {
                Some((bus.load(at & !3)? >> (8 * (at & 2))) as u16)
            };
            let lo = half(pc)?;
            if crate::isa::is_compressed(lo) {
                let w = crate::isa::compressed(lo).unwrap_or(lo as u32);
                return Some((w, 2));
            }
            let hi = half(pc.wrapping_add(2))?;
            return Some(((hi as u32) << 16 | lo as u32, 4));
        }
        if pc < IMEM_BYTES {
            return fetch(imem, pc);
        }
        let half = |at: u32| -> Option<u16> {
            let off = at.wrapping_sub(DATA_BASE);
            (off < DATA_BYTES).then(|| {
                (self.mem[(off / 4) as usize] >> (8 * (at & 2))) as u16
            })
        };
        let lo = half(pc)?;
        if crate::isa::is_compressed(lo) {
            return Some((crate::isa::compressed(lo).unwrap_or(lo as u32), 2));
        }
        let hi = half(pc.wrapping_add(2))?;
        Some(((hi as u32) << 16 | lo as u32, 4))
    }

    /// A word by byte address: of the boot memory below `IMEM_BYTES`,
    /// which is on the bus read-only and holds a program's constants
    /// (issue 268); of the data memory; or, above it, the word the bus
    /// answered the core with, which the caller handed over. `None`
    /// between the boot memory and the data memory.
    fn word(&self, imem: &[u32], addr: u32) -> Option<u32> {
        // The step's data access, for the timing mode; a store marks
        // its own before it reads the word it merges into.
        if !matches!(self.access.get(), Some((a, true)) if a == addr) {
            self.access.set(Some((addr, false)));
        }
        if in_dram(addr) {
            return Some(self.dram[((addr - DRAM_BASE) / 4) as usize]);
        }
        if let Some(i) = self.ddr_index(addr) {
            return self.ddr.as_ref().map(|(_, m)| m[i]);
        }
        if let Some(bus) = &self.bus {
            return bus.load(addr & !3);
        }
        if addr < IMEM_BYTES {
            return Some(*imem.get((addr / 4) as usize).unwrap_or(&0));
        }
        if addr >= DEVICES {
            return Some(self.dev_word);
        }
        let off = addr.wrapping_sub(DATA_BASE);
        (off < DATA_BYTES).then(|| self.mem[(off / 4) as usize])
    }

    /// What a translation reads (issue 1014): `satp`, the privilege,
    /// and `SUM` and `MXR`.
    fn vm_mode(&self) -> Mode {
        Mode {
            satp: self.csr.satp,
            prv: self.prv as u8,
            sum: self.csr.mstatus & SUM != 0,
            mxr: self.csr.mstatus & MXR != 0,
        }
    }

    /// A page table entry as the walker reads it: from the boot memory
    /// or the data memory. One among the devices is beyond the model,
    /// as a fetch from one is, so it is taken as nothing.
    fn pte_word(&self, imem: &[u32], pa: u32) -> Option<u32> {
        // In a machine it is whatever the bus answers there.
        if let Some(bus) = &self.bus {
            return bus.load(pa & !3);
        }
        if pa >= DEVICES {
            return None;
        }
        self.word(imem, pa)
    }

    /// The halfword at physical address `at`, of the boot memory or the
    /// data memory, as the core fetches it from the bus.
    fn fetch_half(&self, imem: &[u32], at: u32) -> Option<u16> {
        if let Some(bus) = &self.bus {
            return Some((bus.load(at & !3)? >> (8 * (at & 2))) as u16);
        }
        if at < IMEM_BYTES {
            let w = imem.get((at / 4) as usize).copied().unwrap_or(0);
            return Some((w >> (8 * (at & 2))) as u16);
        }
        let off = at.wrapping_sub(DATA_BASE);
        (off < DATA_BYTES)
            .then(|| (self.mem[(off / 4) as usize] >> (8 * (at & 2))) as u16)
    }

    /// The instruction at `pc` and its length, or the trap its fetch
    /// raises, as a cause and a value (issue 1014). Without
    /// translation it is what `fetch_at` says. With it, the first
    /// half's page is translated, and the second half's when a
    /// thirty-two bit instruction's second half begins the next page,
    /// which is where the core's second word is. A page fault's value
    /// is the address of the half that faulted, and an access fault's
    /// the instruction's, as the core's fetch from the bus gives.
    /// `translate`, through the TLB when the model is in a machine.
    fn walk(
        &self,
        imem: &[u32],
        m: Mode,
        va: u32,
        access: Access,
    ) -> Result<u32, Fault> {
        let read = |pa| self.pte_word(imem, pa);
        if self.bus.is_none() {
            return translate(read, m, va, access);
        }
        // What the hart's walker reads when its buffers miss (issue
        // 1561), as `Mmu8`'s walk goes: the first level's entry, and the
        // second's unless the first is a leaf or a fault; and the leaf it
        // keeps, which it keeps whether or not the access is then
        // allowed, since the buffers check that on every hit.
        // Nothing is walked where translation is off: `satp` bare, or the
        // access's mode machine. `m.prv` is that mode, which for a load
        // or a store in machine mode with `MPRV` set is `MPP`'s, as
        // `data_pa` gives it, so such an access is walked as `MPP`'s.
        let on = m.satp >> 31 != 0 && m.prv != 3;
        // The core waits for the unit's answer on every translated load
        // and store, and on a translated fetch from a page other than the
        // one it last had (issue 1561).
        if on && self.timing.is_some() {
            let waits = match access {
                Access::Fetch => self.hw_tlb.borrow_mut().fetch_moves(va),
                _ => true,
            };
            if waits && access == Access::Store {
                self.xlate_stores.set(self.xlate_stores.get() + 1);
            } else if waits {
                self.xlates.set(self.xlates.get() + 1);
            }
        }
        let miss = self.timing.is_some()
            && on
            && !self.hw_tlb.borrow().holds(va, access);
        if miss {
            let leaf = |p: u32| p & 0xa != 0;
            // Not valid, writable but not readable, a pointer at the
            // second level, a megapage not aligned, or past 32 bits.
            let bad = |p: u32, top: bool| {
                p & 1 == 0
                    || (p & 2 == 0 && p & 4 != 0)
                    || (!leaf(p) && !top)
                    || (leaf(p) && top && (p >> 10) & 0x3ff != 0)
                    || p >> 30 != 0
            };
            let root = (m.satp & 0x3f_ffff) << 12;
            let first = read(root | (va >> 22) << 2).unwrap_or(0);
            let (reads, keep) = if bad(first, true) {
                (1, None)
            } else if leaf(first) {
                (1, Some(true))
            } else {
                let at = (first >> 10) << 12 | ((va >> 12) & 0x3ff) << 2;
                let second = read(at).unwrap_or(0);
                (2, (!bad(second, false)).then_some(false))
            };
            self.walks.set(self.walks.get() + reads);
            if let Some(mega) = keep {
                self.hw_tlb.borrow_mut().fill(va, mega, access);
            }
        }
        let vpn = va >> 12;
        if let Some(ppn) = self.tlb.borrow().get(vpn, &m, access) {
            return Ok(ppn << 12 | (va & 0xfff));
        }
        let pa = translate(read, m, va, access)?;
        self.tlb.borrow_mut().put(vpn, &m, access, pa >> 12);
        Ok(pa)
    }

    fn fetch_vm(
        &self,
        imem: &[u32],
        pc: u32,
    ) -> Result<(u32, u32), (u32, u32)> {
        let m = self.vm_mode();
        if m.satp >> 31 == 0 || m.prv == 3 {
            return self.fetch_at(imem, pc).ok_or((CAUSE_FETCH_ACCESS, pc));
        }
        let fault = |f: Fault, at: u32| match f {
            Fault::Page => (CAUSE_FETCH_PAGE, at),
            Fault::Access => (CAUSE_FETCH_ACCESS, pc),
        };
        let pa0 = self
            .walk(imem, m, pc, Access::Fetch)
            .map_err(|f| fault(f, pc))?;
        let lo = self.fetch_half(imem, pa0).ok_or((CAUSE_FETCH_ACCESS, pc))?;
        if is_compressed(lo) {
            return Ok((compressed(lo).unwrap_or(lo as u32), 2));
        }
        let pc2 = pc.wrapping_add(2);
        let pa1 = if pc2 & 0xfff == 0 {
            self.walk(imem, m, pc2, Access::Fetch)
                .map_err(|f| fault(f, pc2))?
        } else {
            pa0.wrapping_add(2)
        };
        let hi = self.fetch_half(imem, pa1).ok_or((CAUSE_FETCH_ACCESS, pc))?;
        Ok(((hi as u32) << 16 | lo as u32, 4))
    }

    /// The physical address of a data access at `va`, or the trap it
    /// raises with `va` as its value: a page fault, or an access fault
    /// where the walk read nothing (issue 1014). Without translation,
    /// `va` itself.
    fn data_pa(
        &self,
        imem: &[u32],
        va: u32,
        store: bool,
    ) -> Result<u32, (u32, u32)> {
        let access = if store { Access::Store } else { Access::Load };
        // In machine mode with `MPRV` set, as the mode `MPP` names
        // (issue 1105).
        let mut m = self.vm_mode();
        if self.prv == 3 && self.csr.mstatus & MPRV != 0 {
            m.prv = (self.csr.mstatus >> 11 & 3) as u8;
        }
        self.walk(imem, m, va, access).map_err(|f| {
            let cause = match (f, store) {
                (Fault::Page, false) => CAUSE_LOAD_PAGE,
                (Fault::Page, true) => CAUSE_STORE_PAGE,
                (Fault::Access, false) => CAUSE_LOAD_ACCESS,
                (Fault::Access, true) => CAUSE_STORE_ACCESS,
            };
            (cause, va)
        })
    }

    /// A store's word. Above the data memory the store is a device's
    /// business; the model keeps what it can check at the end, the
    /// timer's compare and the bytes given to the serial port. A store
    /// into the boot memory is refused by the memory and ignored by the
    /// core, which does not look at a write's answer, so it changes
    /// nothing here either.
    /// The word of the model's own DDR3 window at `addr`, if it has one
    /// and `addr` is in it (issue 1443).
    fn ddr_index(&self, addr: u32) -> Option<usize> {
        let (base, m) = self.ddr.as_ref()?;
        let i = (addr.wrapping_sub(*base) / 4) as usize;
        (addr >= *base && i < m.len()).then_some(i)
    }

    fn set_word(&mut self, addr: u32, v: u32) -> bool {
        if in_dram(addr) {
            self.dram[((addr - DRAM_BASE) / 4) as usize] = v;
            return true;
        }
        if let Some(i) = self.ddr_index(addr) {
            if let Some((_, m)) = self.ddr.as_mut() {
                m[i] = v;
            }
            return true;
        }
        if let Some(bus) = &self.bus {
            return bus.store(addr & !3, v, u32::MAX);
        }
        if addr < IMEM_BYTES {
            return true;
        }
        if addr & CLINT_MASK == CLINT_BASE {
            let off = addr & !CLINT_MASK;
            if off == MTIMECMP_OFF || off == MTIMECMP_OFF + 4 {
                let shift = 8 * (off & 4);
                self.mtimecmp = (self.mtimecmp & !(0xffff_ffff << shift))
                    | (v as u64) << shift;
            }
            if off == 0 {
                self.msip = v & 1 == 1;
            }
            return true;
        }
        if addr == UART_BASE {
            self.uart.push(v as u8);
            return true;
        }
        if addr >= DEVICES {
            return true;
        }
        let off = addr.wrapping_sub(DATA_BASE);
        if off < DATA_BYTES {
            self.mem[(off / 4) as usize] = v;
        }
        off < DATA_BYTES
    }

    /// The timer's pending bit: its line, as the core saw it.
    fn mtip(&self) -> u32 {
        if self.tirq {
            MTIMER
        } else {
            0
        }
    }

    /// The software interrupt's pending bit: the controller's `msip`,
    /// as the core sees it on its line.
    fn msip(&self) -> u32 {
        if self.msip {
            MSOFT
        } else {
            0
        }
    }

    /// `mip` whole: the external line, the timer's and the software
    /// interrupt's, and the supervisor's bits software set.
    fn mip_all(&self) -> u32 {
        self.csr.mip | self.mtip() | self.msip() | self.csr.mip_sw
    }

    /// Whether the hart, at its privilege, may reach a CSR at all: the
    /// address's bits 9 and 8 name the least privilege that may, and a
    /// counter below machine mode wants its bit in `mcounteren`, and in
    /// user mode in `scounteren` as well (issue 1012).
    /// Whether supervisor mode is running with one of `mstatus`'s
    /// `TVM`, `TW` and `TSR` set, which makes what it names illegal
    /// there (issue 1347).
    fn s_trapped(&self, bit: u32) -> bool {
        self.prv == 1 && self.csr.mstatus & bit != 0
    }

    fn csr_allowed(&self, addr: u32) -> bool {
        if addr == CSR_SATP && self.s_trapped(TVM) {
            return false;
        }
        if self.prv < (addr >> 8 & 3) {
            return false;
        }
        if matches!(addr, 0xc00..=0xc02 | 0xc80..=0xc82) {
            let bit = 1 << (addr & 3);
            if self.prv < 3 && self.csr.mcounteren & bit == 0 {
                return false;
            }
            if self.prv == 0 && self.csr.scounteren & bit == 0 {
                return false;
            }
        }
        // `hpmcounter3` to `hpmcounter31` and their upper halves: their
        // bits in `mcounteren` are zero for good, so below machine mode
        // they are illegal (issue 1460).
        if matches!(addr, 0xc03..=0xc1f | 0xc83..=0xc9f) && self.prv < 3 {
            return false;
        }
        true
    }

    /// Whether a CSR is one of the counters that count nothing here,
    /// which the privileged specification has a machine-mode hart hold
    /// all the same: `mcountinhibit`, `mhpmevent3` to `mhpmevent31`,
    /// `mhpmcounter3` to `mhpmcounter31` with their upper halves, and
    /// the user's copies. Each reads as zero and a write changes
    /// nothing (issue 1460). In the core, `mhpmcounter3` to `7` count
    /// the instruction cache's events (#1752); this model has no cache,
    /// so for it they stay here.
    fn hpm_zero(addr: u32) -> bool {
        matches!(
            addr,
            0x320
                | 0x323..=0x33f
                | 0xb03..=0xb1f
                | 0xb83..=0xb9f
                | 0xc03..=0xc1f
                | 0xc83..=0xc9f
        )
    }

    fn csr_read(&self, addr: u32) -> Option<u32> {
        Some(match addr {
            CSR_MSTATUS => self.csr.mstatus,
            CSR_MTVEC => self.csr.mtvec,
            CSR_MSCRATCH => self.csr.mscratch,
            CSR_MEPC => self.csr.mepc,
            CSR_MCAUSE => self.csr.mcause,
            CSR_MIE => self.csr.mie,
            CSR_MIP => self.mip_all(),
            CSR_MTVAL => self.csr.mtval,
            // The halt holds nothing: it reads as zero, and a write of
            // an odd value to it stops the machine.
            CSR_MHALT => 0,
            CSR_MBUSQUIET => self.csr.busquiet as u32,
            CSR_DCSR => self.dcsr,
            CSR_DPC => self.dpc,
            // What the machine is. `misa` says RV32IMAC with
            // supervisor and user modes; the four
            // machine information registers say that the vendor, the
            // architecture and the implementation are unassigned and
            // that `mhartid` is the hart's number (issue 1408).
            CSR_MISA => MISA,
            CSR_MVENDORID | CSR_MARCHID | CSR_MIMPID => 0,
            CSR_MHARTID => self.hartid,
            // The counters are legal here, so that a program reading
            // one traps in neither the core nor the model. What they
            // read is another matter: this model steps on a
            // retirement and has no idea how many cycles the pipeline
            // spent, so it counts what it can, which is retirements,
            // and answers zero for the cycles. A lockstep program must
            // therefore not read `mcycle` into a register, and the
            // counters are checked by a directed run instead, without
            // a model beside it. The same holds for the instruction
            // cache's counters, `mhpmcounter3` to `7` (#1752): this
            // model has no cache, and they read zero here as the ones
            // that count nothing do.
            CSR_MINSTRET => self.minstret as u32,
            CSR_MINSTRETH => (self.minstret >> 32) as u32,
            // In the timing mode, the cycles it has charged (issue
            // 1392).
            CSR_MCYCLE if self.timing.is_some() => self.cycles as u32,
            CSR_MCYCLEH if self.timing.is_some() => (self.cycles >> 32) as u32,
            CSR_MCYCLE | CSR_MCYCLEH => 0,
            CSR_MEDELEG => self.csr.medeleg,
            // Its upper half holds nothing here (issue 1076).
            CSR_MSTATUSH => 0,
            CSR_MENVCFG => self.csr.menvcfg,
            CSR_MENVCFGH => 0,
            CSR_SENVCFG => self.csr.senvcfg,
            CSR_MIDELEG => self.csr.mideleg,
            CSR_MCOUNTEREN => self.csr.mcounteren,
            CSR_SCOUNTEREN => self.csr.scounteren,
            CSR_SSTATUS => self.csr.mstatus & SSTATUS_W,
            CSR_SIE => self.csr.mie & self.csr.mideleg,
            CSR_SIP => self.mip_all() & self.csr.mideleg,
            CSR_STVEC => self.csr.stvec,
            CSR_SSCRATCH => self.csr.sscratch,
            CSR_SEPC => self.csr.sepc,
            CSR_SCAUSE => self.csr.scause,
            CSR_STVAL => self.csr.stval,
            CSR_SATP => self.csr.satp,
            // Zicntr: the cycles as `mcycle`, which the model cannot
            // know; the count the timer gave; the retirements.
            CSR_CYCLE if self.timing.is_some() => self.cycles as u32,
            CSR_CYCLEH if self.timing.is_some() => (self.cycles >> 32) as u32,
            CSR_CYCLE | CSR_CYCLEH => 0,
            CSR_TIME => self.time as u32,
            CSR_TIMEH => (self.time >> 32) as u32,
            CSR_INSTRET => self.minstret as u32,
            CSR_INSTRETH => (self.minstret >> 32) as u32,
            a if Self::hpm_zero(a) => 0,
            _ => return None,
        })
    }

    /// The external interrupt line at an edge: the pending bit is the
    /// line, set and cleared by the controller and not by software, as
    /// the privileged specification has MEIP (#788).
    pub fn line(&mut self, high: bool) {
        self.csr.mip = (self.csr.mip & !MEXT) | if high { MEXT } else { 0 };
    }

    /// The supervisor's external line, from the controller's second
    /// target: SEIP, which a read of `mip` shows ORed with the bit
    /// software sets, and which no write changes (issue 1094).
    pub fn sline(&mut self, high: bool) {
        self.csr.mip = (self.csr.mip & !SEXT) | if high { SEXT } else { 0 };
    }

    fn csr_write(&mut self, addr: u32, v: u32) {
        match addr {
            CSR_MSTATUS => self.csr.mstatus = mstatus_w(v),
            CSR_MTVEC => self.csr.mtvec = v & !3,
            CSR_MSCRATCH => self.csr.mscratch = v,
            CSR_MEPC => self.csr.mepc = v & !1,
            CSR_MCAUSE => self.csr.mcause = v,
            CSR_MIE => {
                self.csr.mie =
                    v & (MEXT | MSOFT | MTIMER | SEXT | SSOFT | STIMER)
            }
            // MEIP is read only, and the timer's and the software
            // interrupt's bits are their lines: a write changes none of
            // those. The supervisor's three are software's (issue 1012).
            CSR_MIP => self.csr.mip_sw = v & (SEXT | SSOFT | STIMER),
            CSR_MEDELEG => self.csr.medeleg = v & MEDELEG_W,
            CSR_MIDELEG => self.csr.mideleg = v & (SEXT | SSOFT | STIMER),
            CSR_MCOUNTEREN => self.csr.mcounteren = v & 7,
            CSR_SCOUNTEREN => self.csr.scounteren = v & 7,
            CSR_SSTATUS => {
                self.csr.mstatus =
                    (self.csr.mstatus & !SSTATUS_W) | (v & SSTATUS_W)
            }
            CSR_SIE => {
                let d = self.csr.mideleg;
                self.csr.mie = (self.csr.mie & !d) | (v & d)
            }
            // Of `sip`, supervisor software may set and clear its own
            // software interrupt, when it is delegated.
            CSR_SIP => {
                let d = self.csr.mideleg & SSOFT;
                self.csr.mip_sw = (self.csr.mip_sw & !d) | (v & d)
            }
            CSR_STVEC => self.csr.stvec = v & !3,
            CSR_SSCRATCH => self.csr.sscratch = v,
            CSR_MENVCFG => self.csr.menvcfg = v & 1,
            CSR_SENVCFG => self.csr.senvcfg = v & 1,
            CSR_SEPC => self.csr.sepc = v & !1,
            CSR_SCAUSE => self.csr.scause = v,
            CSR_STVAL => self.csr.stval = v,
            CSR_SATP => {
                self.csr.satp = v;
                self.tlb.borrow_mut().flush();
                self.hw_tlb.borrow_mut().flush();
            }
            CSR_MTVAL => self.csr.mtval = v,
            CSR_MBUSQUIET => self.csr.busquiet = v & 1 != 0,
            // `ebreakm` and `step` are the debugger's; the rest is the
            // core's to say.
            CSR_DCSR => {
                self.dcsr = 0x4000_0000 | (v & 0x8004) | (self.dcsr & 0x1c3)
            }
            CSR_DPC => self.dpc = v & !1,
            CSR_MINSTRET => {
                self.minstret = (self.minstret & !0xffff_ffff) | u64::from(v)
            }
            CSR_MINSTRETH => {
                self.minstret =
                    (self.minstret & 0xffff_ffff) | (u64::from(v) << 32)
            }
            _ => {}
        }
    }

    /// A trap: the cause, the instruction's address and the trap value
    /// are saved, the interrupt enable is saved and cleared, and the
    /// handler is next. The trap value is the word for an illegal
    /// instruction and zero otherwise.
    ///
    /// A trap below machine mode whose cause machine mode delegated, in
    /// `medeleg` for an exception and `mideleg` for an interrupt, goes to
    /// the supervisor instead, through its own registers (issue 1012).
    fn trap(&mut self, cause: u32, tval: u32) {
        self.hw_tlb.borrow_mut().forget_fetch();
        if let Some(n) = self.causes.get_mut(cause as usize) {
            *n += 1;
        }
        let deleg = if cause >> 31 == 1 {
            self.csr.mideleg
        } else {
            self.csr.medeleg
        };
        let s = &mut self.csr.mstatus;
        if self.prv < 3 && deleg >> (cause & 31) & 1 == 1 {
            self.csr.sepc = self.pc;
            self.csr.scause = cause;
            self.csr.stval = tval;
            let spie = if *s & SIE != 0 { SPIE } else { 0 };
            let spp = if self.prv == 1 { SPP } else { 0 };
            *s = (*s & !(SIE | SPIE | SPP)) | spie | spp;
            self.prv = 1;
            self.pc = self.csr.stvec;
        } else {
            self.csr.mepc = self.pc;
            self.csr.mcause = cause;
            self.csr.mtval = tval;
            let mpie = if *s & MIE != 0 { MPIE } else { 0 };
            *s = (*s & !(MIE | MPIE | MPP)) | mpie | (self.prv << 11);
            self.prv = 3;
            self.pc = self.csr.mtvec;
        }
    }

    /// The interrupt that would be taken before the next instruction,
    /// if any: the external one first, then the software one, then the
    /// timer's, each pending and enabled in `mie`, with interrupts
    /// enabled in `mstatus`.
    ///
    /// One machine mode keeps is taken below machine mode always and in
    /// it with `MIE`; one it delegated is taken below supervisor mode
    /// always, in it with `SIE`, and never in machine mode; and the
    /// machine's come first (issue 1012).
    pub fn interrupt(&self) -> Option<u32> {
        let pending = self.mip_all() & self.csr.mie;
        let s = self.csr.mstatus;
        let m_on = self.prv < 3 || s & MIE != 0;
        let s_on = self.prv == 0 || (self.prv == 1 && s & SIE != 0);
        let m = if m_on { pending & !self.csr.mideleg } else { 0 };
        let sv = if s_on { pending & self.csr.mideleg } else { 0 };
        let set = if m != 0 { m } else { sv };
        // The order the specification gives: the external interrupt
        // first, then the software one, then the timer's, the
        // machine's before the supervisor's.
        [
            (MEXT, CAUSE_MEXT),
            (MSOFT, CAUSE_MSOFT),
            (MTIMER, CAUSE_MTIMER),
            (SEXT, CAUSE_SEXT),
            (SSOFT, CAUSE_SSOFT),
            (STIMER, CAUSE_STIMER),
        ]
        .into_iter()
        .find(|&(bit, _)| set & bit != 0)
        .map(|(_, cause)| cause)
    }

    /// One instruction, or the interrupt taken instead of it when
    /// `interrupt` names one: the caller decides, since the core decides
    /// on the pending bits and the count as they stood a cycle earlier.
    /// Debug mode entered, before the instruction at `pc`, which
    /// becomes `dpc`: the cause is a step when one has just run, an
    /// `ebreak` when that is the instruction and `dcsr.ebreakm` is set,
    /// and a halt request otherwise.
    pub fn enter_debug(&mut self, imem: &[u32]) {
        let at_ebreak = self
            .fetch_at(imem, self.pc)
            .map(|(w, _)| matches!(decode(w).kind, Kind::Ebreak))
            .unwrap_or(false);
        let cause = if self.stepped {
            4
        } else if at_ebreak && self.dcsr & 0x8000 != 0 {
            1
        } else {
            3
        };
        self.dcsr =
            0x4000_0000 | (self.dcsr & 0x8004) | (cause << 6) | self.prv;
        self.dpc = self.pc;
        // A machine that had stopped itself is halted no longer: the
        // debugger may resume it from `dpc` (issue 1047).
        self.halted = None;
        self.debug = true;
        self.stepped = false;
    }

    /// The reset line: the program counter to zero, every CSR and both
    /// counters as configuration left them, the timer's compare to all
    /// ones, debug mode left and the halt ended. The register file and
    /// the memory keep what they hold, as the hardware's do; so do the
    /// bytes the serial port was given, which are the run's record
    /// rather than the machine's state (issue 419).
    pub fn reset(&mut self) {
        self.pc = 0;
        self.tlb.borrow_mut().flush();
        self.hw_tlb.borrow_mut().flush();
        self.csr = Csr::default();
        self.minstret = 0;
        self.mtimecmp = u64::MAX;
        self.halted = None;
        self.debug = false;
        self.dpc = 0;
        self.dcsr = 0x4000_0003;
        self.step_armed = false;
        self.stepped = false;
        self.rsv = None;
        self.prv = 3;
    }

    /// A write the debug module makes while the core is in debug mode:
    /// `dcsr`'s `ebreakm` and `step`, or `dpc` (issue 972).
    pub fn debug_write(&mut self, csr: u32, v: u32) {
        if matches!(csr, CSR_DCSR | CSR_DPC) {
            self.csr_write(csr, v);
        }
    }

    /// Debug mode left, to `dpc`, arming a single step when `dcsr.step`
    /// asks for one.
    pub fn resume(&mut self) {
        self.hw_tlb.borrow_mut().forget_fetch();
        self.debug = false;
        self.pc = self.dpc;
        // The mode the hart was in, as `dcsr.prv` says (issue 1012).
        self.prv = match self.dcsr & 3 {
            2 => 0,
            p => p,
        };
        self.step_armed = self.dcsr & 4 != 0;
    }

    /// Does nothing once halted.
    pub fn step(&mut self, imem: &[u32], interrupt: Option<u32>) {
        if self.halted.is_some() || self.debug {
            return;
        }
        self.wrote = None;
        if self.timing.is_none() {
            return self.execute(imem, interrupt);
        }
        // In the timing mode the step is charged what the core would
        // spend on it (issue 1392).
        self.access.set(None);
        self.excl.set(None);
        self.wfi_now.set(false);
        self.mkind.set(0);
        self.ilen.set(4);
        self.back.set(false);
        self.reads.set((0, 0));
        self.loaded.set(0);
        self.walks.set(0);
        self.xlates.set(0);
        self.xlate_stores.set(0);
        self.ifence.set(false);
        self.fenced.set(false);
        let pc = self.pc;
        self.execute(imem, interrupt);
        let t = self.timing.as_ref().expect("timing");
        let mut c = t.base;
        let taken =
            self.pc != pc.wrapping_add(4) && self.pc != pc.wrapping_add(2);
        if taken {
            c += t.taken;
        } else if self.back.get() {
            c += t.guessed;
        }
        // The instruction cache, by the instruction's line (issue 1561).
        if let Some(Icache {
            jumped,
            ran_on,
            step,
            after,
            shared,
        }) = t.icache
        {
            let line = pc >> 4;
            if self.ic.len() != 1024 {
                self.ic = vec![0; 1024];
            }
            if line != self.last_line {
                let slot = (line & 1023) as usize;
                if self.ic[slot] != line + 1 {
                    c += if self.fell { ran_on } else { jumped };
                    if self.contended {
                        c += shared;
                    }
                    self.ic[slot] = line + 1;
                    self.ic_fills += 1;
                } else if self.fell {
                    c += step;
                }
                self.last_line = line;
            }
            // The word after a jump, fetched as it executes (#1679): its
            // line is looked up when it is not the jump's, and filled, and
            // kept, when it is a line not held.
            let next = pc.wrapping_add(self.ilen.get()) >> 4;
            let slot = (next & 1023) as usize;
            if taken && next != line {
                if self.ic[slot] == next + 1 {
                    c += after.0;
                } else {
                    c += after.1;
                    if self.contended {
                        c += shared;
                    }
                    self.ic[slot] = next + 1;
                    self.ic_fills += 1;
                }
            }
        }
        self.fell = !taken;
        c += match self.mkind.get() {
            1 => t.mul,
            2 => t.div,
            _ => 0,
        };
        let (r1, r2) = self.reads.get();
        if self.prev_loaded != 0
            && (r1 == self.prev_loaded || r2 == self.prev_loaded)
        {
            c += t.load_use;
        }
        self.prev_loaded = self.loaded.get();
        c += self.walks.get() as u64 * t.walk / 2;
        c += self.xlates.get() as u64 * t.xlate;
        // A store and `sc.w` wait less than a load, and an AMO not at all.
        let per_store = match self.excl.get() {
            Some((3, _)) => t.xlate_amo,
            _ => t.xlate_store,
        };
        c += self.xlate_stores.get() as u64 * per_store;
        if self.ifence.get() {
            self.ic.clear();
            self.last_line = u32::MAX;
        }
        // A fence waits for a store into the DDR3 still on its way.
        if self.fenced.get() {
            c += t.fence_alone
                + t.fence.saturating_sub(self.cycles - self.stored_at);
        }
        if let Some((addr, true)) = self.access.get() {
            if cached(addr) {
                self.stored_at = self.cycles + c;
            }
        }
        let busy = self.contended as u64;
        let (hit, excl, contention) = (t.hit, t.excl, t.contention);
        if let Some((kind, pa)) = self.excl.get().filter(|(_, pa)| cached(*pa))
        {
            // An exclusive pair on the bus, which leaves its line out of
            // the cache.
            c += match kind {
                1 => excl.0,
                2 => excl.1 + busy * contention.1,
                _ => excl.2 + busy * contention.2,
            };
            if hit.is_some() {
                self.dc_forget(pa);
            }
        } else if let Some((addr, store)) = self.access.get() {
            let line = (addr >> 4) + 1;
            let slot = ((addr >> 4) & 255) as usize;
            let held = cached(addr) && self.dc.get(slot) == Some(&line);
            c += match hit {
                Some(h) if !store && held => h,
                _ => t.access(addr, store),
            };
            if store && cached(addr) {
                c += busy * contention.0;
            }
            if hit.is_some() && !store && !held && cached(addr) && self.scan_on
            {
                c += t.scan_fill;
            }
            if hit.is_some() && !store && cached(addr) {
                if self.dc.len() != 256 {
                    self.dc = vec![0; 256];
                }
                self.dc[slot] = line;
            }
        }
        // A hart but the first forgets its data cache as it wakes from
        // `wfi`, whatever woke it (issue 1408).
        if self.wfi_now.get() {
            self.last_wfi = self.cycles;
            if self.hartid != 0 {
                self.dc.clear();
            }
        }
        self.cycles += c;
    }

    /// The data cache in the timing mode no longer holds the line of
    /// `pa`: another hart wrote it, or an exclusive went round it.
    pub fn dc_forget(&mut self, pa: u32) {
        let slot = ((pa >> 4) & 255) as usize;
        if self.dc.get(slot) == Some(&((pa >> 4) + 1)) {
            self.dc[slot] = 0;
        }
    }

    /// One instruction, or the trap taken instead of it.
    fn execute(&mut self, imem: &[u32], interrupt: Option<u32>) {
        // One more retired, counted before the instruction runs so
        // that a read of `minstret` by this instruction does not count
        // itself, which is what the specification asks for.
        self.minstret = self.minstret.wrapping_add(1);
        // A fetch from nowhere: the bus refuses it and the core raises
        // the instruction access fault (issue 423). The model has no
        // words for a device either, so a program run from one is
        // beyond it.
        //
        // Under translation the fetch may fault on its page instead
        // (issue 1014). An interrupt is taken ahead of either, as the
        // core takes one instead of the instruction in execute,
        // whatever its fetch said.
        let fetched = self.fetch_vm(imem, self.pc);
        // Its length, for the timing mode's word after it (#1679), also
        // when an interrupt is taken in its place.
        if let Ok((_, len)) = fetched {
            self.ilen.set(len);
        }
        if let Some(cause) = interrupt {
            self.trap(cause, 0);
            return;
        }
        let (w, len) = match fetched {
            Ok(f) => f,
            Err((cause, tval)) => {
                self.trap(cause, tval);
                return;
            }
        };
        // The instruction about to run is the stepped one, if a step
        // was armed: the core asks to enter again before the next.
        if self.step_armed {
            self.step_armed = false;
            self.stepped = true;
        }
        let d = decode(w);
        if self.timing.is_some() {
            self.reads.set((d.rs1, d.rs2));
            self.back.set(
                matches!(
                    d.kind,
                    Kind::Beq
                        | Kind::Bne
                        | Kind::Blt
                        | Kind::Bge
                        | Kind::Bltu
                        | Kind::Bgeu
                ) && d.imm < 0,
            );
            self.mkind.set(match d.kind {
                Kind::Mul | Kind::Mulh | Kind::Mulhsu | Kind::Mulhu => 1,
                Kind::Div | Kind::Divu | Kind::Rem | Kind::Remu => 2,
                _ => 0,
            });
            if matches!(
                d.kind,
                Kind::Lb | Kind::Lh | Kind::Lw | Kind::Lbu | Kind::Lhu
            ) {
                self.loaded.set(d.rd);
            }
        }
        let a = self.x[d.rs1 as usize];
        let b = self.x[d.rs2 as usize];
        let imm = d.imm as u32;
        let sh = (b & 31) as u32;
        let shi = (imm & 31) as u32;
        let mut next = self.pc.wrapping_add(len);
        let mut rd: Option<u32> = None;
        // Whether this instruction is the one that stops the
        // machine: a write of an odd value to `mhalt`.
        let mut halting = false;
        use Kind::*;
        match d.kind {
            Lui => rd = Some(imm),
            Auipc => rd = Some(self.pc.wrapping_add(imm)),
            Jal => {
                rd = Some(next);
                next = self.pc.wrapping_add(imm);
            }
            Jalr => {
                rd = Some(next);
                next = a.wrapping_add(imm) & !1;
            }
            Beq | Bne | Blt | Bge | Bltu | Bgeu => {
                let taken = match d.kind {
                    Beq => a == b,
                    Bne => a != b,
                    Blt => (a as i32) < (b as i32),
                    Bge => (a as i32) >= (b as i32),
                    Bltu => a < b,
                    _ => a >= b,
                };
                if taken {
                    next = self.pc.wrapping_add(imm);
                }
            }
            Lb | Lh | Lw | Lbu | Lhu => {
                let va = a.wrapping_add(imm);
                // A half wants an even address and a word a multiple of
                // four. The core raises the exception rather than
                // supporting the access, which the specification allows
                // and issue 138 asked for, so the model does too.
                if misaligned(d.kind, va) {
                    self.trap(CAUSE_LOAD_MISALIGNED, va);
                    return;
                }
                // Then the translation, which may fault (issue 1014).
                let addr = match self.data_pa(imem, va, false) {
                    Ok(pa) => pa,
                    Err((cause, tval)) => {
                        self.trap(cause, tval);
                        return;
                    }
                };
                // The bus refused: the caller says so with the answer,
                // and the load is an access fault at its address.
                if addr >= DEVICES && self.dev_err && !self.csr.busquiet {
                    self.trap(CAUSE_LOAD_ACCESS, va);
                    return;
                }
                // Between the boot memory and the data memory nothing
                // answers, and the router says so: an access fault too.
                let Some(word) = self.word(imem, addr) else {
                    self.trap(CAUSE_LOAD_ACCESS, va);
                    return;
                };
                let byte = (word >> (8 * (addr & 3))) & 0xff;
                let half = (word >> (16 * (addr >> 1 & 1))) & 0xffff;
                rd = Some(match d.kind {
                    Lb => byte as u8 as i8 as i32 as u32,
                    Lh => half as u16 as i16 as i32 as u32,
                    Lw => word,
                    Lbu => byte,
                    _ => half,
                });
            }
            Sb | Sh | Sw => {
                let va = a.wrapping_add(imm);
                if misaligned(d.kind, va) {
                    self.trap(CAUSE_STORE_MISALIGNED, va);
                    return;
                }
                let addr = match self.data_pa(imem, va, true) {
                    Ok(pa) => pa,
                    Err((cause, tval)) => {
                        self.trap(cause, tval);
                        return;
                    }
                };
                self.access.set(Some((addr, true)));
                self.wrote = Some(addr);
                // In a machine the store goes out with its lanes' mask,
                // and nothing is read first (issue 1016); one into the
                // data RAM stays in the core (issue 1275).
                if let (Some(bus), false) = (&self.bus, in_dram(addr)) {
                    let (lanes, shift) = match d.kind {
                        Sb => (0xff, 8 * (addr & 3)),
                        Sh => (0xffff, 16 * (addr >> 1 & 1)),
                        _ => (u32::MAX, 0),
                    };
                    let v = (b & lanes) << shift;
                    if !bus.store(addr & !3, v, lanes << shift) {
                        self.halted = Some(Halt::Fault(addr));
                        return;
                    }
                } else {
                    let Some(word) = self.word(imem, addr) else {
                        self.halted = Some(Halt::Fault(addr));
                        return;
                    };
                    let v = match d.kind {
                        Sb => {
                            let lane = 8 * (addr & 3);
                            (word & !(0xff << lane)) | ((b & 0xff) << lane)
                        }
                        Sh => {
                            let lane = 16 * (addr >> 1 & 1);
                            (word & !(0xffff << lane)) | ((b & 0xffff) << lane)
                        }
                        _ => b,
                    };
                    self.set_word(addr, v);
                }
            }
            // The A extension (issue 1010). One hart, so atomic within
            // it: `lr.w` loads and reserves the word, `sc.w` stores only
            // to the reserved word and uses the reservation up either
            // way, and an AMO reads the word, writes the operation's
            // result and returns the old word. A misaligned one traps,
            // `lr.w` as a load and the rest as a store; a word the bus
            // refuses is a load's access fault for `lr.w` and a store's
            // for an AMO, which writes nothing.
            LrW => {
                if misaligned(d.kind, a) {
                    self.trap(CAUSE_LOAD_MISALIGNED, a);
                    return;
                }
                self.rsv = Some(a);
                let pa = match self.data_pa(imem, a, false) {
                    Ok(pa) => pa,
                    Err((cause, tval)) => {
                        self.trap(cause, tval);
                        return;
                    }
                };
                self.rsv_pa = Some(pa);
                self.excl.set(Some((1, pa)));
                if pa >= DEVICES && self.dev_err && !self.csr.busquiet {
                    self.trap(CAUSE_LOAD_ACCESS, a);
                    return;
                }
                let Some(word) = self.word(imem, pa) else {
                    self.trap(CAUSE_LOAD_ACCESS, a);
                    return;
                };
                rd = Some(word);
            }
            ScW => {
                let held = self.rsv == Some(a);
                self.rsv = None;
                if misaligned(d.kind, a) {
                    self.trap(CAUSE_STORE_MISALIGNED, a);
                    return;
                }
                // The core translates whether or not the reservation
                // holds, so a page fault traps either way.
                let pa = match self.data_pa(imem, a, true) {
                    Ok(pa) => pa,
                    Err((cause, tval)) => {
                        self.trap(cause, tval);
                        return;
                    }
                };
                self.excl.set(Some((2, pa)));
                if held {
                    if self.word(imem, pa).is_none() {
                        self.halted = Some(Halt::Fault(a));
                        return;
                    }
                    self.set_word(pa, b);
                    self.wrote = Some(pa);
                }
                rd = Some(!held as u32);
            }
            AmoswapW | AmoaddW | AmoxorW | AmoandW | AmoorW | AmominW
            | AmomaxW | AmominuW | AmomaxuW => {
                if misaligned(d.kind, a) {
                    self.trap(CAUSE_STORE_MISALIGNED, a);
                    return;
                }
                // An AMO writes, so it is translated as a store.
                let pa = match self.data_pa(imem, a, true) {
                    Ok(pa) => pa,
                    Err((cause, tval)) => {
                        self.trap(cause, tval);
                        return;
                    }
                };
                let refused = pa >= DEVICES && self.dev_err;
                if refused && !self.csr.busquiet {
                    self.trap(CAUSE_STORE_ACCESS, a);
                    return;
                }
                self.excl.set(Some((3, pa)));
                let Some(old) = self.word(imem, pa) else {
                    self.trap(CAUSE_STORE_ACCESS, a);
                    return;
                };
                let new = match d.kind {
                    AmoswapW => b,
                    AmoaddW => old.wrapping_add(b),
                    AmoxorW => old ^ b,
                    AmoandW => old & b,
                    AmoorW => old | b,
                    AmominW => (old as i32).min(b as i32) as u32,
                    AmomaxW => (old as i32).max(b as i32) as u32,
                    AmominuW => old.min(b),
                    _ => old.max(b),
                };
                // A refusal the program asked to be quiet about reads
                // the bus's word and writes nothing, as a load does.
                if !refused {
                    self.set_word(pa, new);
                    self.wrote = Some(pa);
                }
                rd = Some(old);
            }
            Addi => rd = Some(a.wrapping_add(imm)),
            Slti => rd = Some(((a as i32) < (imm as i32)) as u32),
            Sltiu => rd = Some((a < imm) as u32),
            Xori => rd = Some(a ^ imm),
            Ori => rd = Some(a | imm),
            Andi => rd = Some(a & imm),
            Slli => rd = Some(a << shi),
            Srli => rd = Some(a >> shi),
            Srai => rd = Some(((a as i32) >> shi) as u32),
            Add => rd = Some(a.wrapping_add(b)),
            Sub => rd = Some(a.wrapping_sub(b)),
            Sll => rd = Some(a << sh),
            Slt => rd = Some(((a as i32) < (b as i32)) as u32),
            Sltu => rd = Some((a < b) as u32),
            Xor => rd = Some(a ^ b),
            Srl => rd = Some(a >> sh),
            Sra => rd = Some(((a as i32) >> sh) as u32),
            Or => rd = Some(a | b),
            And => rd = Some(a & b),
            // The M extension. A product's low word is the same for
            // every signedness; the high word depends on it. Division
            // by zero and the one overflow are what the manual says,
            // which is what Rust's wrapping division says too, except
            // for the zero, which Rust would refuse.
            Mul => rd = Some(a.wrapping_mul(b)),
            Mulh => {
                rd = Some(((a as i32 as i64 * b as i32 as i64) >> 32) as u32)
            }
            Mulhsu => rd = Some(((a as i32 as i64 * b as i64) >> 32) as u32),
            Mulhu => rd = Some(((a as u64 * b as u64) >> 32) as u32),
            Div => {
                rd = Some(if b == 0 {
                    u32::MAX
                } else {
                    (a as i32).wrapping_div(b as i32) as u32
                })
            }
            Divu => rd = Some(if b == 0 { u32::MAX } else { a / b }),
            Rem => {
                rd = Some(if b == 0 {
                    a
                } else {
                    (a as i32).wrapping_rem(b as i32) as u32
                })
            }
            Remu => rd = Some(if b == 0 { a } else { a % b }),
            // `fence.i` empties the instruction cache in the timing mode.
            Fence => {
                self.ifence.set(w >> 12 & 7 == 1);
                if w >> 12 & 7 == 1 {
                    self.hw_tlb.borrow_mut().forget_fetch();
                }
                self.fenced.set(w >> 12 & 7 == 0);
            }
            Ebreak => {
                // A breakpoint, not a halt: a monitor catches it,
                // prints, steps, continues. A program that means to
                // stop writes `mhalt` instead, which is issue 139.
                self.trap(CAUSE_BREAKPOINT, self.pc);
                return;
            }
            // The cause says which mode called: 8 user, 9 supervisor,
            // 11 machine (issue 1012).
            Ecall => {
                self.trap(8 + self.prv, 0);
                return;
            }
            Illegal => {
                self.trap(CAUSE_ILLEGAL, w);
                return;
            }
            // The core waits for an interrupt rather than spinning,
            // but nothing retires while it waits, and the lockstep
            // steps this model on a retirement. So the model has no
            // waiting state: it retires `wfi` and moves on, and the
            // core's wait shows up as cycles in which it retires
            // nothing, which is what the test measures.
            // Below supervisor mode it is an illegal instruction.
            // Not in user mode; there is nothing for the model to drop,
            // since it holds no translations (issue 1014).
            SfenceVma => {
                if self.prv == 0 || self.s_trapped(TVM) {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                self.tlb.borrow_mut().flush();
                self.hw_tlb.borrow_mut().flush();
            }
            Wfi => {
                if self.prv == 0 || self.s_trapped(TW) {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                self.wfi_now.set(true);
            }
            // `mret` only in machine mode and `sret` not in user mode;
            // each returns to the mode it saved and leaves user mode
            // behind in its place (issue 1012).
            Mret => {
                if self.prv < 3 {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                let s = self.csr.mstatus;
                let mie = if s & MPIE != 0 { MIE } else { 0 };
                // A return below machine mode clears `MPRV` (issue 1105).
                let mprv = if s >> 11 & 3 == 3 { s & MPRV } else { 0 };
                self.csr.mstatus =
                    (s & !(MIE | MPP | MPRV)) | MPIE | mie | mprv;
                self.prv = s >> 11 & 3;
                next = self.csr.mepc;
                self.hw_tlb.borrow_mut().forget_fetch();
            }
            Sret => {
                if self.prv == 0 || self.s_trapped(TSR) {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                let s = self.csr.mstatus;
                let sie = if s & SPIE != 0 { SIE } else { 0 };
                self.csr.mstatus = (s & !(SIE | SPP | MPRV)) | SPIE | sie;
                self.prv = s >> 8 & 1;
                next = self.csr.sepc;
                self.hw_tlb.borrow_mut().forget_fetch();
            }
            Csrrw | Csrrs | Csrrc | Csrrwi | Csrrsi | Csrrci => {
                // `dcsr` and `dpc` only in debug mode, which a program
                // never runs in: the debugger writes them through
                // `debug_write` (issue 972).
                if (matches!(imm, CSR_DCSR | CSR_DPC) && !self.debug)
                    || !self.csr_allowed(imm)
                {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                let Some(old) = self.csr_read(imm) else {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                };
                // A write to a register whose address begins with two
                // set bits is an illegal instruction. `csrrw` and
                // `csrrwi` always write; a set or a clear writes only
                // when its source field is not zero, which is what the
                // specification says and is not the same as the value
                // in the register being zero.
                let writes = matches!(d.kind, Csrrw | Csrrwi) || d.rs1 != 0;
                // That is the information registers and, since issue
                // 1012, the unprivileged counters.
                if writes && imm >> 10 == 3 {
                    self.trap(CAUSE_ILLEGAL, w);
                    return;
                }
                let src = match d.kind {
                    Csrrw | Csrrs | Csrrc => a,
                    _ => d.rs1,
                };
                // A set or a clear of `mip` modifies the software's SEIP,
                // not the controller's line a read shows with it (issue
                // 1094).
                let base = if imm == CSR_MIP {
                    (old & !SEXT) | (self.csr.mip_sw & SEXT)
                } else {
                    old
                };
                let v = match d.kind {
                    Csrrw | Csrrwi => src,
                    Csrrs | Csrrsi => base | src,
                    _ => base & !src,
                };
                // A read writes nothing, as the core does not (#807).
                if writes {
                    self.csr_write(imm, v);
                }
                rd = Some(old);
                // A write of an odd value to `mhalt` is how a program
                // says it is finished. The register holds nothing and
                // reads as zero, and the instruction retires as any
                // other does before the machine stops, which is what
                // the core does too.
                halting = imm == CSR_MHALT && v & 1 != 0;
            }
        }
        if let Some(v) = rd {
            if d.rd != 0 {
                self.x[d.rd as usize] = v;
            }
        }
        self.pc = next;
        if halting {
            self.halted = Some(Halt::Break);
        }
    }
}

#[cfg(test)]
mod tests {

    /// A memory of words for the TLB's test, as a machine's bus.
    #[derive(Debug, Default)]
    struct Words(std::cell::RefCell<std::collections::HashMap<u32, u32>>);

    impl Bus for Words {
        fn load(&self, addr: u32) -> Option<u32> {
            Some(*self.0.borrow().get(&addr).unwrap_or(&0))
        }
        fn store(&self, addr: u32, v: u32, mask: u32) -> bool {
            let mut m = self.0.borrow_mut();
            let w = m.entry(addr).or_insert(0);
            *w = (*w & !mask) | (v & mask);
            true
        }
    }

    /// The machine's TLB answers as the walk does (issue 1132): a miss
    /// walks and keeps the answer, a hit gives the same address, a
    /// changed entry is not seen until `sfence.vma` or a write of
    /// `satp`, as on a hart, and a fault is never kept.
    #[test]
    fn the_machines_tlb_answers_as_the_walk_does() {
        use txhdl_parts::mmu::pte::{to, A, D, R, V, W};
        let root = 0x8000_0000u32;
        let l0 = 0x8000_1000u32;
        let words = std::rc::Rc::new(Words::default());
        let put = |a: u32, v: u32| assert!(words.store(a, v, u32::MAX));
        let va = 0x0040_1234u32;
        let leaf = l0 + ((va >> 12) & 0x3ff) * 4;
        put(root + (va >> 22) * 4, to(l0, V));
        put(leaf, to(0x5000_0000, V | R | W | A | D));
        let mut m = Model {
            bus: Some(words.clone() as std::rc::Rc<dyn Bus>),
            ..Model::default()
        };
        m.prv = 1;
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        let walk = |m: &Model, store: bool| {
            let access = if store { Access::Store } else { Access::Load };
            let read = |pa| m.pte_word(&[], pa);
            translate(read, m.vm_mode(), va, access)
        };
        assert_eq!(m.data_pa(&[], va, false), Ok(0x5000_0234));
        assert_eq!(Ok(0x5000_0234), walk(&m, false), "the walk agrees");
        assert_eq!(m.data_pa(&[], va, false), Ok(0x5000_0234), "a hit");
        // The page moves. Until a flush the TLB keeps the old one, as a
        // hart's may; after it, the new one.
        put(leaf, to(0x6000_0000, V | R | W | A | D));
        assert_eq!(m.data_pa(&[], va, false), Ok(0x5000_0234), "kept");
        m.tlb.borrow_mut().flush();
        assert_eq!(m.data_pa(&[], va, false), Ok(0x6000_0234), "flushed");
        assert_eq!(Ok(0x6000_0234), walk(&m, false));
        // A store to a page whose dirty bit is clear faults every time.
        put(leaf, to(0x6000_0000, V | R | W | A));
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        assert_eq!(m.data_pa(&[], va, true), Err((CAUSE_STORE_PAGE, va)));
        assert_eq!(m.data_pa(&[], va, true), Err((CAUSE_STORE_PAGE, va)));
        assert_eq!(m.data_pa(&[], va, false), Ok(0x6000_0234));
    }

    /// The waits for a translation the timing mode counts (issue 1561):
    /// one for every translated load or store, one for a fetch from a
    /// page other than the last one fetched from, none for a fetch from
    /// the same page, none in machine mode, and one for a load there that
    /// `MPRV` translates; a write of `satp` and a trap forget the fetch's
    /// page. A store's is counted apart, for `xlate_store`.
    #[test]
    fn the_timing_mode_counts_the_waits_for_a_translation() {
        use txhdl_parts::mmu::pte::{to, A, D, R, V, W, X};
        let root = 0x8000_0000u32;
        let words = std::rc::Rc::new(Words::default());
        let put = |a: u32, v: u32| assert!(words.store(a, v, u32::MAX));
        // A megapage at 0x0080_0000 that may be read, written and run.
        put(root + 8, to(0x4000_0000, V | R | W | X | A | D));
        let mut m = Model {
            bus: Some(words.clone() as std::rc::Rc<dyn Bus>),
            timing: Some(Timing::board_cached()),
            ..Model::default()
        };
        m.prv = 1;
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        let waits = |m: &Model, f: &dyn Fn(&Model)| {
            m.xlates.set(0);
            m.xlate_stores.set(0);
            f(m);
            m.xlates.get() + m.xlate_stores.get()
        };
        let load = |va: u32| {
            move |m: &Model| {
                let _ = m.data_pa(&[], va, false);
            }
        };
        let store = |va: u32| {
            move |m: &Model| {
                let _ = m.data_pa(&[], va, true);
            }
        };
        let fetch = |va: u32| {
            move |m: &Model| {
                let _ = m.fetch_vm(&[], va);
            }
        };
        assert_eq!(waits(&m, &load(0x0080_0010)), 1, "a load");
        assert_eq!(waits(&m, &store(0x0080_0010)), 1, "a store");
        assert_eq!(m.xlate_stores.get(), 1, "counted as a store's");
        assert_eq!(waits(&m, &load(0x0080_0010)), 1, "a load");
        assert_eq!(m.xlate_stores.get(), 0, "not as a store's");
        assert_eq!(waits(&m, &load(0x0080_0014)), 1, "every one, held or not");
        assert_eq!(waits(&m, &fetch(0x0080_1000)), 1, "a fetch's first page");
        assert_eq!(waits(&m, &fetch(0x0080_1ff0)), 0, "the same page");
        assert_eq!(waits(&m, &fetch(0x0080_2000)), 1, "the next page");
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        assert_eq!(waits(&m, &fetch(0x0080_2004)), 1, "satp forgets it");
        m.prv = 3;
        assert_eq!(waits(&m, &load(0x4000_0010)), 0, "machine mode");
        m.csr.mstatus |= MPRV | 1 << 11;
        assert_eq!(waits(&m, &load(0x0080_0010)), 1, "MPRV, MPP supervisor");
        m.csr.mstatus &= !(MPRV | MPP);
        m.prv = 1;
        assert_eq!(waits(&m, &fetch(0x0080_2008)), 0, "the page held");
        m.trap(CAUSE_BREAKPOINT, 0);
        m.prv = 1;
        assert_eq!(waits(&m, &fetch(0x0080_2008)), 1, "a trap forgets it");
        let t = m.timing.as_ref().unwrap();
        assert_eq!(t.xlate, 4, "the RTL's wait, and the board's");
        assert_eq!(t.xlate_store, 2, "the board's for a store");
    }

    /// The timing mode charges the walks the hart's own buffers make, as
    /// `Mmu8` has them (issue 1561), not the fast TLB's: eight data
    /// entries filled in turn, a load and a store sharing one, a
    /// megapage one entry for its 4 MiB and one table read, a page two,
    /// a first-level entry not valid one, fetches apart, and a write of
    /// `satp` emptying them.
    #[test]
    fn the_timing_mode_charges_the_walks_the_harts_buffers_make() {
        use txhdl_parts::mmu::pte::{to, A, D, R, V, W, X};
        let root = 0x8000_0000u32;
        let l0 = 0x8000_1000u32;
        let words = std::rc::Rc::new(Words::default());
        let put = |a: u32, v: u32| assert!(words.store(a, v, u32::MAX));
        let page = |k: u32| 0x0040_0000 + k * 0x1000;
        put(root + 4, to(l0, V));
        for k in 0..16 {
            put(l0 + k * 4, to(0x5000_0000 + k * 0x1000, V | R | W | A | D));
        }
        let mega = 0x0080_0000u32;
        put(root + 8, to(0x4000_0000, V | R | W | X | A | D));
        let mut m = Model {
            bus: Some(words.clone() as std::rc::Rc<dyn Bus>),
            timing: Some(Timing::board_cached()),
            ..Model::default()
        };
        m.prv = 1;
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        // The table entries read for one access.
        let reads = |m: &Model, va: u32, store: bool| {
            m.walks.set(0);
            let _ = m.data_pa(&[], va, store);
            m.walks.get()
        };
        assert_eq!(reads(&m, page(0), false), 2, "a page: two levels");
        assert_eq!(reads(&m, page(0) + 8, false), 0, "held");
        assert_eq!(reads(&m, page(0), true), 0, "a store shares the load's");
        assert_eq!(reads(&m, mega + 0x12_3450, false), 1, "a megapage: one");
        assert_eq!(reads(&m, mega + 0x3f_fffc, true), 0, "all 4 MiB of it");
        // Seven more pages fill the eight; the next takes the oldest
        // entry, page 0's, and leaves the rest.
        for k in 1..8 {
            assert_eq!(reads(&m, page(k), false), 2, "page {k}");
        }
        assert_eq!(reads(&m, page(0), false), 2, "page 0 was replaced");
        assert_eq!(reads(&m, page(3), false), 0, "page 3 was not");
        // Fetches have buffers of their own.
        assert!(!m.hw_tlb.borrow().holds(page(3), Access::Fetch));
        // Nothing mapped at the first level: one read, the fault, and
        // nothing kept, so the next try reads again.
        assert_eq!(reads(&m, 0x0140_0000, false), 1, "one read, no leaf");
        assert!(m.data_pa(&[], 0x0140_0000, false).is_err());
        assert_eq!(reads(&m, 0x0140_0000, false), 1, "not kept");
        // Writable but not readable at the first level: a fault there.
        put(root + 24, W | V);
        assert_eq!(reads(&m, 0x0180_0000, false), 1, "reserved: one read");
        // Nothing at the second level: two reads, and nothing kept.
        assert_eq!(reads(&m, page(20), false), 2, "no leaf at the second");
        assert_eq!(reads(&m, page(20), false), 2, "still not kept");
        // A leaf the access may not use is kept all the same, as the
        // RTL keeps it and checks on each hit: a store to a page that is
        // not writable faults, and a load after it does not walk.
        put(l0 + 4 * 16, to(0x5001_0000, V | R | A));
        assert_eq!(reads(&m, page(16), true), 2, "the store walks");
        assert!(m.data_pa(&[], page(16), true).is_err(), "and faults");
        assert_eq!(reads(&m, page(16), false), 0, "the leaf was kept");
        // A write of `satp` empties them.
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        assert_eq!(reads(&m, page(3), false), 2, "flushed");
        // Fetches walk into their own buffers.
        m.walks.set(0);
        let _ = m.fetch_vm(&[], mega);
        assert_eq!(m.walks.get(), 1, "a fetch from the megapage");
        assert_eq!(reads(&m, mega, false), 1, "the data port's own walk");
        m.walks.set(0);
        let _ = m.fetch_vm(&[], mega + 4);
        assert_eq!(m.walks.get(), 0, "the fetch's entry held");
        // Nor in machine mode, where nothing is translated, whatever
        // the page or how many there are.
        m.prv = 3;
        for k in 0..12 {
            assert_eq!(reads(&m, 0x4000_0000 + k * 0x1000, false), 0, "M");
        }
        // Unless `MPRV` sends a load through the supervisor's tables.
        m.csr.mstatus |= MPRV | 1 << 11;
        assert_eq!(reads(&m, page(9), false), 2, "MPRV, MPP supervisor");
        m.csr.mstatus &= !(MPRV | MPP);
        m.prv = 1;
        // Nor with `satp` bare.
        m.csr_write(CSR_SATP, 0);
        assert_eq!(reads(&m, page(10), false), 0, "satp bare");
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        // The other faults the walker stops at: a pointer at the second
        // level, a megapage not aligned, and an entry past 32 bits, each
        // read and never kept.
        put(l0 + 4 * 21, to(l0, V));
        assert_eq!(reads(&m, page(21), false), 2, "a pointer at level 0");
        assert_eq!(reads(&m, page(21), false), 2, "not kept");
        put(root + 28, to(0x4000_1000, V | R | W | A | D));
        assert_eq!(reads(&m, 0x01c0_0000, false), 1, "megapage not aligned");
        assert_eq!(reads(&m, 0x01c0_0000, false), 1, "not kept");
        put(root + 32, 1 << 30 | V | R | W | A | D);
        assert_eq!(reads(&m, 0x0200_0000, false), 1, "past 32 bits");
        assert_eq!(reads(&m, 0x0200_0000, false), 1, "not kept");
        // Outside the timing mode nothing is counted.
        m.timing = None;
        m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
        assert_eq!(reads(&m, page(5), false), 0, "not timed");
    }

    use super::*;
    use crate::isa::{addi, c_addi, c_jal, c_jr, c_nop, halt, lui};

    /// A program of both widths: a compressed instruction, a thirty-two
    /// bit one that starts in the upper half of the first word and ends
    /// in the second, a compressed call whose link is two bytes on, and
    /// a compressed return to that link.
    #[test]
    fn half_words() {
        let lui5 = lui(5, 0x12345);
        let imem = [
            (lui5 & 0xffff) << 16 | c_addi(1, 7) as u32, // 0, 2
            (c_jal(10) as u32) << 16 | lui5 >> 16,       // 6: to 16
            addi(6, 0, 1),                               // 8
            halt(),                                      // 12
            (c_nop() as u32) << 16 | c_jr(1) as u32,     // 16: to 8
        ];
        assert_eq!(fetch(&imem, 0), Some((addi(1, 1, 7), 2)));
        assert_eq!(fetch(&imem, 2), Some((lui5, 4)), "across two words");
        assert_eq!(fetch(&imem, 20), None, "past the end");
        let mut m = Model::default();
        let mut pcs = vec![];
        while m.halted.is_none() {
            pcs.push(m.pc);
            m.step(&imem, None);
        }
        assert_eq!(pcs, [0, 2, 6, 16, 8, 12]);
        assert_eq!(m.halted, Some(Halt::Break));
        assert_eq!(m.x[1], 8, "the link is the address after c.jal");
        assert_eq!(m.x[5], 0x1234_5000);
        assert_eq!(m.x[6], 1);
    }

    /// An unaligned load and an unaligned store each trap, with the
    /// cause the specification names and the address as the trap
    /// value. A byte never traps, whatever the address, and the
    /// aligned forms still work. This is issue 138: each of these used
    /// to use the aligned word and say nothing.
    #[test]
    fn an_unaligned_access_traps() {
        use crate::isa::{csrrw, lb, lh, lw, sh, sw};
        let handler = 0x40;
        // The trap handler is a word past everything else, and every
        // program below sets it, does one access, and ends.
        let run = |access: u32| {
            let mut imem = vec![
                lui(1, 0x1),            // 0: x1 = 0x1000, the data
                addi(2, 0, 1),          // 4: x2 = 1, a byte to store
                addi(3, 0, handler),    // 8: x3 = the handler
                csrrw(0, CSR_MTVEC, 3), // 12: mtvec = x3
                access,                 // 16
                halt(),                 // 20
            ];
            imem.resize(32, halt());
            let mut m = Model::default();
            for _ in 0..8 {
                if m.halted.is_some() {
                    break;
                }
                m.step(&imem, None);
            }
            m
        };
        // The aligned forms go through and leave no cause behind.
        let m = run(lw(4, 1, 0));
        assert_eq!(m.csr.mcause, 0, "an aligned word is no trap");
        let m = run(lb(4, 1, 3));
        assert_eq!(m.csr.mcause, 0, "a byte at any address is no trap");
        // The unaligned ones trap, with the address they asked for.
        let m = run(lw(4, 1, 2));
        assert_eq!(m.csr.mcause, CAUSE_LOAD_MISALIGNED, "lw at 0x1002");
        assert_eq!(m.csr.mtval, 0x1002, "the address is the trap value");
        assert_eq!(m.csr.mepc, 16, "the access that trapped");
        // The handler's first word is one of the halts the fill above
        // put there, and a halt retires before the machine stops, so
        // the program counter stands one word past the entry.
        assert_eq!(m.pc, handler as u32 + 4, "and the handler ran");
        let m = run(lh(4, 1, 1));
        assert_eq!(m.csr.mcause, CAUSE_LOAD_MISALIGNED, "lh at 0x1001");
        let m = run(sw(2, 1, 1));
        assert_eq!(m.csr.mcause, CAUSE_STORE_MISALIGNED, "sw at 0x1001");
        assert_eq!(m.csr.mtval, 0x1001);
        let m = run(sh(2, 1, 3));
        assert_eq!(m.csr.mcause, CAUSE_STORE_MISALIGNED, "sh at 0x1003");
        // And the store that trapped wrote nothing.
        assert_eq!(m.mem[0], 0, "a trapping store leaves the word alone");
    }

    /// A line of the instruction cache costs by how it was reached (#1679),
    /// against no cache: cold, the first line and a line a `jal` went to
    /// each cost a jumped-to miss, the line after the `jal`'s its fill,
    /// and the line the second ran on into a ran-on miss; warm, the line
    /// after the `jal`'s its lookup, and the line run on into its running
    /// on.
    #[test]
    fn a_line_costs_by_how_it_was_reached() {
        use crate::isa::jal;
        let mut imem = vec![addi(1, 1, 1), addi(1, 1, 1), addi(1, 1, 1)];
        imem.push(jal(0, 52)); // 12: to 64, line 4
        imem.resize(16, halt());
        imem.extend([addi(1, 1, 1); 5]); // 64 to 80: lines 4 and 5
        imem.push(halt());
        let run = |icache: Option<Icache>| {
            let timing = Timing {
                icache,
                ..Timing::board()
            };
            let mut m = Model {
                timing: Some(timing),
                ..Model::default()
            };
            let mut cycles = vec![];
            for _ in 0..2 {
                // Back to the start, as a jump goes there.
                (m.pc, m.halted, m.fell) = (0, None, false);
                let before = m.cycles;
                while m.halted.is_none() {
                    m.step(&imem, None);
                }
                cycles.push(m.cycles - before);
                cycles.push(m.ic_fills);
            }
            cycles
        };
        let icache = Icache {
            jumped: 84,
            ran_on: 45,
            step: 3,
            after: (2, 40),
            shared: 0,
        };
        let (with, without) = (run(Some(icache)), run(None));
        assert_eq!(with[0] - without[0], 84 + 40 + 84 + 45, "cold");
        // Four fills cold, the same four lines' costs, none warm, and
        // none without the cache (#1760).
        assert_eq!([with[1], with[3]], [4, 4], "lines filled");
        assert_eq!([without[1], without[3]], [0, 0], "no cache, no fill");
        assert_eq!(with[2] - without[2], 2 + 3, "warm");
    }

    /// The line the word after a jump fills is the next one's to find
    /// (#1679): a jump into it finds it held.
    #[test]
    fn the_word_after_a_jump_fills_its_line() {
        use crate::isa::jal;
        let cold = |target: u32| {
            let mut imem = vec![addi(1, 1, 1), addi(1, 1, 1), addi(1, 1, 1)];
            imem.push(jal(0, target as i32 - 12)); // 12, line 0's last word
            imem.resize(target as usize / 4 + 1, halt());
            let run = |icache: Option<Icache>| {
                let timing = Timing {
                    icache,
                    ..Timing::board()
                };
                let mut m = Model {
                    timing: Some(timing),
                    ..Model::default()
                };
                while m.halted.is_none() {
                    m.step(&imem, None);
                }
                m.cycles
            };
            let icache = Icache {
                jumped: 84,
                ran_on: 45,
                step: 3,
                after: (2, 40),
                shared: 0,
            };
            run(Some(icache)) - run(None)
        };
        // To line 1's second word: line 1 is filled after the jump.
        assert_eq!(cold(20), 84 + 40, "into the line after");
    }
    /// A backward branch that falls through costs its wrong guess (#1752):
    /// a round of a loop with one costs `guessed` more than a round with
    /// a no-op in its place.
    #[test]
    fn a_backward_branch_not_taken_costs_its_guess() {
        use crate::isa::bne;
        let run = |w: u32| {
            let imem = vec![
                addi(2, 0, 10),
                addi(1, 1, 1),
                w,
                addi(2, 2, -1),
                bne(2, 0, -12),
                halt(),
            ];
            let mut m = Model {
                timing: Some(Timing::board()),
                ..Model::default()
            };
            while m.halted.is_none() {
                m.step(&imem, None);
            }
            m.cycles
        };
        let (with, without) = (run(bne(0, 0, -4)), run(addi(0, 0, 0)));
        assert_eq!(with - without, 10 * Timing::board().guessed);
    }
    /// A fill while the other hart runs costs `shared` more (#1752), every
    /// fill and nothing else: a cold run whose `jal` fills its own line
    /// and the line after it costs two `shared` more contended than not.
    /// And the board's timing charges a wrong guess the board's 5, where
    /// the core's own figure stays 2.
    #[test]
    fn a_fill_while_the_other_hart_runs_costs_shared() {
        use crate::isa::jal;
        let mut imem = vec![addi(1, 1, 1), addi(1, 1, 1), addi(1, 1, 1)];
        imem.push(jal(0, 20 - 12)); // 12, line 0's last word, to 20
        imem.resize(6, halt());
        let run = |contended: bool| {
            let icache = Icache {
                jumped: 84,
                ran_on: 45,
                step: 3,
                after: (2, 40),
                shared: 7,
            };
            let timing = Timing {
                icache: Some(icache),
                ..Timing::board()
            };
            let mut m = Model {
                timing: Some(timing),
                contended,
                ..Model::default()
            };
            while m.halted.is_none() {
                m.step(&imem, None);
            }
            m.cycles
        };
        assert_eq!(run(true) - run(false), 2 * 7, "two fills, each shared");
        assert_eq!(Timing::board_cached().guessed, 5);
        assert_eq!(Timing::board().guessed, 2);
    }

    /// A load that fills a line of the data cache costs `scan_fill` more
    /// while the scanout shows a frame (issue 1561), and nothing else
    /// does: of two loads from two lines of the DDR3, one again from the
    /// first, which the cache holds, and a store, only the two fills pay.
    #[test]
    fn a_fill_while_the_scanout_shows_costs_scan_fill() {
        use crate::isa::{lw, sw};
        let imem = vec![
            lui(2, 0x4010_0000 >> 12),
            lw(3, 2, 0),
            lw(3, 2, 16),
            lw(3, 2, 4),
            sw(3, 2, 32),
            halt(),
        ];
        let run = |scan_on: bool| {
            let mut m = Model {
                timing: Some(Timing::board_cached()),
                scan_on,
                ..Model::default()
            };
            while m.halted.is_none() {
                m.step(&imem, None);
            }
            m.cycles
        };
        let fill = Timing::board_cached().scan_fill;
        assert_eq!(fill, 7, "the board's, about 1.74 a word");
        assert_eq!(run(true) - run(false), 2 * fill, "the two fills");
        assert_eq!(Timing::board().scan_fill, 0, "without the cache");
    }

    /// What a translated store, an AMO, a load and an `lr.w` and `sc.w`
    /// pair cost the step (issue 1561), each beyond the same
    /// untranslated: a store `xlate_store`, an AMO `xlate_amo`, a load
    /// and `lr.w` `xlate`, and `sc.w` `xlate_store`, once the page's entry
    /// is held. The data
    /// is translated through `MPRV`, as the supervisor's, while the
    /// fetches stay machine mode's and untranslated.
    #[test]
    fn a_translated_store_waits_less_than_a_load_or_an_amo() {
        use crate::isa::{amoadd_w, lr_w, lw, sc_w, sw};
        use txhdl_parts::mmu::pte::{to, A, D, R, V, W, X};
        let root = 0x8000_0000u32;
        let imem = vec![
            lw(3, 2, 0), // the page's walk, then held
            sw(3, 2, 4),
            amoadd_w(0, 2, 3),
            lw(3, 2, 8),
            lr_w(4, 2),
            sc_w(5, 2, 3),
            halt(),
        ];
        // The cycles of each step, with the data at `base`, translated or
        // not.
        let run = |translated: bool| {
            let words = std::rc::Rc::new(Words::default());
            let put = |a: u32, v: u32| assert!(words.store(a, v, u32::MAX));
            put(root + 8, to(0x4000_0000, V | R | W | X | A | D));
            // With a bus the program is fetched from it.
            for (i, w) in imem.iter().enumerate() {
                put(4 * i as u32, *w);
            }
            let mut m = Model {
                bus: Some(words.clone() as std::rc::Rc<dyn Bus>),
                timing: Some(Timing::board_cached()),
                ..Model::default()
            };
            if translated {
                m.csr_write(CSR_SATP, 1 << 31 | root >> 12);
                m.csr.mstatus |= MPRV | 1 << 11;
            }
            m.x[2] = if translated { 0x0080_0010 } else { 0x4000_0010 };
            let mut each = vec![];
            while m.halted.is_none() && each.len() < imem.len() {
                let was = m.cycles;
                m.step(&imem, None);
                each.push(m.cycles - was);
            }
            each
        };
        let (on, off) = (run(true), run(false));
        let t = Timing::board_cached();
        assert_eq!(on[1] - off[1], t.xlate_store, "a store");
        assert_eq!(on[2] - off[2], t.xlate_amo, "an AMO");
        assert_eq!(on[3] - off[3], t.xlate, "a load");
        assert_eq!(on[4] - off[4], t.xlate, "lr.w, a load");
        assert_eq!(on[5] - off[5], t.xlate_store, "sc.w, a store");
        assert_eq!((t.xlate_store, t.xlate_amo), (2, 0));
    }
}
