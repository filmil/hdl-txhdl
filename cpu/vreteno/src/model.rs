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

/// The architectural state, and only that.
#[derive(Clone, Debug)]
pub struct Model {
    /// The machine the model runs in, when it is one: every fetch, load
    /// and store goes to it. `None` is the board as the lockstep test
    /// holds the core to it, its memories the model's own.
    pub bus: Option<std::rc::Rc<dyn Bus>>,
    /// The fast machine's translations; empty and unused without a bus.
    pub tlb: std::cell::RefCell<Tlb>,
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
    /// The instruction cache's lines in the timing mode, each the line's
    /// address plus one, the line the last instruction was in, and
    /// whether control fell through to this one (issue 1561).
    pub ic: Vec<u32>,
    pub last_line: u32,
    pub fell: bool,
    /// What the step was for the timing mode: a multiply 1 or a division
    /// 2; the registers it reads; the register a load of it wrote; the
    /// page walks it made; whether it was `fence.i` (issue 1561).
    pub mkind: std::cell::Cell<u8>,
    pub reads: std::cell::Cell<(u32, u32)>,
    pub loaded: std::cell::Cell<u32>,
    pub prev_loaded: u32,
    pub walks: std::cell::Cell<u32>,
    pub ifence: std::cell::Cell<bool>,
    /// Whether the step was a `fence`, and the cycle count at the last
    /// store into the DDR3 (issue 1561).
    pub fenced: std::cell::Cell<bool>,
    pub stored_at: u64,
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
    /// 1024 lines of 16 bytes; a line it does not hold costs the first,
    /// and running on into the next line it holds the second. None
    /// models no cache, as `taken` alone did.
    pub icache: Option<(u64, u64)>,
    /// A translation the TLB does not hold: the page walk's two reads of
    /// the DDR3 (issue 1561).
    pub walk: u64,
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
    /// load's word used at once 1 more, an instruction-cache line not
    /// held 54.5 more than one held and running on into the next line 3.
    /// A page walk is two loads that miss.
    pub fn board_cached() -> Self {
        let mut t = Timing {
            hit: Some(2),
            excl: (40, 15, 55),
            mul: 2,
            div: 34,
            load_use: 1,
            icache: Some((54, 0)),
            walk: 86,
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
            ic: Vec::new(),
            last_line: u32::MAX,
            fell: false,
            mkind: std::cell::Cell::new(0),
            reads: std::cell::Cell::new((0, 0)),
            loaded: std::cell::Cell::new(0),
            prev_loaded: 0,
            walks: std::cell::Cell::new(0),
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
        let vpn = va >> 12;
        if let Some(ppn) = self.tlb.borrow().get(vpn, &m, access) {
            return Ok(ppn << 12 | (va & 0xfff));
        }
        let pa = translate(read, m, va, access)?;
        self.walks.set(self.walks.get() + 1);
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
    /// nothing (issue 1460).
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
            // a model beside it.
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
        self.reads.set((0, 0));
        self.loaded.set(0);
        self.walks.set(0);
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
        }
        // The instruction cache, by the instruction's line (issue 1561).
        if let Some((miss, step)) = t.icache {
            let line = pc >> 4;
            if line != self.last_line {
                if self.ic.len() != 1024 {
                    self.ic = vec![0; 1024];
                }
                let slot = (line & 1023) as usize;
                if self.ic[slot] != line + 1 {
                    c += miss;
                    self.ic[slot] = line + 1;
                } else if self.fell {
                    c += step;
                }
                self.last_line = line;
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
        c += self.walks.get() as u64 * t.walk;
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
}
