// SPDX-License-Identifier: Apache-2.0
//! The Vreteno board as a fast machine: the reference model of the core,
//! one `step` an instruction, inside the memory and devices a kernel
//! needs, where Linux boots in simulation (issue 1016, item M9 of #279).
//! The cycle-level runs are far too slow for a kernel; this is the
//! model the lockstep test already holds the core to, given a bus.
//!
//! What it holds, each at the address the board's maps give it:
//!
//! * the DDR3, a gigabyte at `0x4000_0000`, as plain memory;
//! * the CLINT, with the count advancing one an instruction, and hart
//!   1's words and the mailbox;
//! * the PLIC with its four targets, machine and supervisor for each of
//!   two harts;
//! * the serial port as SiFive's `sifive,uart0`;
//! * the Ethernet port's slots, with a peer on the cable that answers
//!   ARP and ping (issue 1203).
//!
//! The stack window at `0x1_0000` (issue 1278) is the model's own, as it
//! is the core's (issue 1275), and not on this bus.
//!
//! A program and a device tree blob are loaded into the DDR3, and the
//! hart starts as a bootloader would leave it: `a0` the hart's number,
//! `a1` the blob's address. Everything else on the board answers
//! nothing, which the model takes as an access fault, so a program that
//! strays is told rather than quietly read zeros.
//!
//! A second hart (issue 1408) waits as the board's does until the first
//! starts it through the mailbox: when its `msip` is set it clears it and
//! starts at the mailbox's entry, with `a0` one and `a1` the argument,
//! and `mie.MSIE` on, which is what the board's `park` does; a jump to
//! zero, where `park` is on the board, sends it back to wait. The
//! board's `park` is not on this bus, so its effect is the machine's
//! own. While it runs it steps until it has caught up with the first
//! hart, in cycles in the timing mode and in steps otherwise, so the two
//! run side by side; the count moves with the first. Each step is a
//! whole instruction, so an AMO is atomic, and a store by either hart
//! takes the other's reservation of that word.
//!
//! The devices are in files of their own, and the model learns of them
//! only through its `Bus`, so the ISA's own growth in `model.rs` (A,
//! then the supervisor and user modes) and this machine's do not touch.
pub mod clint;
pub mod eth;
pub mod fbpeer;
pub mod memory;
pub mod plic;
pub mod trng;
pub mod uart;

use crate::board::{BoardMap, SlotMap};
use crate::model::{Bus, Model};
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::map::AddrMap;

/// The PLIC source the serial port asks on: the first of the three the
/// board wires (`board.rs`, and `PLIC_SOURCES` once issue 1015 lands).
pub const SERIAL_SOURCE: usize = 1;
/// The PLIC's sources on the board.
pub const PLIC_SOURCES: usize = 3;
/// The PLIC source the Ethernet port asks on: the board's third.
pub const ETH_SOURCE: usize = 3;

/// A router range: its base, and its length from the mask.
fn range<const N: usize, M: AddrMap<N>>(what: &str) -> (u32, u32) {
    let i = M::NAMES
        .iter()
        .position(|n| n.contains(what))
        .unwrap_or_else(|| panic!("no range in the map is {what}"));
    let (base, mask) = M::RANGES[i];
    (base as u32, ((!mask & 0xffff_ffff) + 1) as u32)
}

/// Where each device is.
#[derive(Clone, Copy, Debug)]
pub struct Map {
    pub ddr: (u32, u32),
    /// The data memory on the bus, 4 KiB at `0x1000` (issue 1392).
    pub dmem: (u32, u32),
    pub clint: (u32, u32),
    pub plic: (u32, u32),
    pub uart: (u32, u32),
    pub eth: (u32, u32),
    pub trng: (u32, u32),
    /// The third slot, where the video peripheral and the scanout are.
    pub video: (u32, u32),
    /// Where the Ethernet port's slots are in the DDR3.
    pub eth_bufs: u32,
}

impl Map {
    /// The board's, from its maps.
    pub fn board() -> Self {
        Map {
            ddr: range::<8, BoardMap>("DDR3"),
            dmem: range::<8, BoardMap>("the data memory"),
            clint: range::<8, BoardMap>("timer"),
            plic: range::<8, BoardMap>("interrupt controller"),
            uart: range::<10, SlotMap>("serial"),
            eth: range::<10, SlotMap>("Ethernet port's registers"),
            trng: range::<10, SlotMap>("entropy source"),
            video: range::<10, SlotMap>("third slot"),
            eth_bufs: crate::isa::ETH_BUF_BASE,
        }
    }
}

fn inside((base, len): (u32, u32), addr: u32) -> Option<u32> {
    let off = addr.wrapping_sub(base);
    (off < len).then_some(off)
}

/// The machine's devices and memory, as one bus.
#[derive(Debug)]
pub struct Devices {
    pub map: Map,
    pub ddr: memory::Memory,
    /// The data memory, which a program may run code from, as the
    /// board's `cpi.rs` does (issue 1392).
    pub dmem: memory::Memory,
    pub clint: clint::Clint,
    pub plic: plic::Plic,
    pub uart: uart::Uart,
    pub eth: eth::Eth,
    pub trng: trng::Trng,
    /// The video peripheral's and the scanout's registers, as words
    /// that keep what is written (issue 1177): a program that points
    /// the scanout at a frame and shows it runs here, and nothing is
    /// shown.
    pub video: [u32; 64],
    /// The lines the PLIC drives, as last worked out, and whether they
    /// have to be worked out again: they change only when a program
    /// reaches the PLIC or the serial port, or when a byte arrives, so
    /// a step need not ask them otherwise (issue 1132).
    pub meip: bool,
    pub seip: bool,
    /// The second hart's two, from the third and fourth targets (issue
    /// 1408).
    pub meip1: bool,
    pub seip1: bool,
    pub stale: bool,
    pub rx_seen: usize,
    /// Steps taken, or in the timing mode cycles: the clock of what is
    /// on the far end of the cable.
    pub steps: u64,
    /// What the last step took: one, or in the timing mode the cycles it
    /// was charged (issue 1392).
    pub elapsed: u64,
}

impl Devices {
    /// Where the scanout shows its frame from, once its control bit
    /// shows it: the third slot's upper half, from `1 << SCAN_BIT`, the
    /// base at its first word and the control at its second (issue
    /// 1440).
    pub fn scanout(&self) -> Option<u32> {
        use txhdl_parts::scanout::{scan, SCAN_BIT};
        let word =
            |off: u32| self.video[(((1 << SCAN_BIT) + off) / 4) as usize];
        (word(scan::ctrl) & 1 != 0).then(|| word(scan::base))
    }
}

/// The bus the model reaches the devices through.
#[derive(Debug)]
pub struct Board(pub RefCell<Devices>);

impl Bus for Board {
    fn load(&self, addr: u32) -> Option<u32> {
        let mut d = self.0.borrow_mut();
        let map = d.map;
        if d.ddr.holds(addr) {
            return Some(d.ddr.load(addr));
        }
        if d.dmem.holds(addr) {
            return Some(d.dmem.load(addr));
        }
        if let Some(off) = inside(map.clint, addr) {
            return Some(d.clint.load(off));
        }
        if let Some(off) = inside(map.plic, addr) {
            d.stale = true;
            return Some(d.plic.load(off));
        }
        if let Some(off) = inside(map.uart, addr) {
            d.stale = true;
            return Some(d.uart.load(off));
        }
        if let Some(off) = inside(map.eth, addr) {
            return Some(d.eth.load(off));
        }
        if let Some(off) = inside(map.trng, addr) {
            return Some(d.trng.load(off));
        }
        if let Some(off) = inside(map.video, addr) {
            return Some(d.video[(off / 4) as usize % 64]);
        }
        None
    }

    fn store(&self, addr: u32, v: u32, mask: u32) -> bool {
        let mut d = self.0.borrow_mut();
        let map = d.map;
        if d.ddr.holds(addr) {
            let was = d.ddr.load(addr);
            d.ddr.store(addr, (was & !mask) | (v & mask));
            return true;
        }
        if d.dmem.holds(addr) {
            let was = d.dmem.load(addr);
            d.dmem.store(addr, (was & !mask) | (v & mask));
            return true;
        }
        // The devices take whole words: a narrower store writes its lanes
        // over zeros, which is what their registers see from the bus.
        if let Some(off) = inside(map.clint, addr) {
            d.clint.store(off, v);
            return true;
        }
        if let Some(off) = inside(map.plic, addr) {
            d.stale = true;
            d.plic.store(off, v);
            return true;
        }
        if let Some(off) = inside(map.uart, addr) {
            d.stale = true;
            d.uart.store(off, v);
            return true;
        }
        if let Some(off) = inside(map.eth, addr) {
            d.stale = true;
            // A transmit is the fetch engine reading the slot, done at
            // once.
            if let eth::Effect::Send { slot, len } = d.eth.store(off, v) {
                let at = map.eth_bufs + eth::TX_REGION + slot * eth::SLOT;
                let frame = d.ddr.get(at, len);
                d.eth.sent(frame);
            }
            return true;
        }
        if let Some(off) = inside(map.trng, addr) {
            d.trng.store(off, v);
            return true;
        }
        if let Some(off) = inside(map.video, addr) {
            d.video[(off / 4) as usize % 64] = v;
            return true;
        }
        false
    }
}

/// The model in its machine, and the second hart's (issue 1408).
pub struct Machine {
    pub model: Model,
    pub board: Rc<Board>,
    /// When set, what each address the hart ran at cost: the cycles, or
    /// the steps outside the timing mode, and how many times it ran
    /// (issue 1434). A function's first address counts its calls.
    pub profile: Option<std::collections::HashMap<u32, (u64, u64)>>,
    /// The same for the second hart (issue 1408).
    pub profile1: Option<std::collections::HashMap<u32, (u64, u64)>>,
    /// An address to watch, and for each caller that reached it, by the
    /// return address in `ra`, how many times and the sum of `a2`, a
    /// copy's length (issue 1434).
    pub watch: Option<(u32, std::collections::HashMap<u32, (u64, u64)>)>,
    /// The second hart, whether it waits to be started, and the two
    /// harts' clocks: cycles in the timing mode, steps otherwise.
    pub model1: Model,
    pub parked1: bool,
    pub clock: (u64, u64),
}

impl Machine {
    /// The board with its memory empty and its devices at reset.
    pub fn new() -> Self {
        let map = Map::board();
        let board = Rc::new(Board(RefCell::new(Devices {
            map,
            ddr: memory::Memory::new(map.ddr.0, map.ddr.1),
            dmem: memory::Memory::new(map.dmem.0, map.dmem.1),
            clint: clint::Clint::default(),
            plic: plic::Plic::new(PLIC_SOURCES),
            uart: uart::Uart::default(),
            eth: eth::Eth::default(),
            trng: trng::Trng::default(),
            video: [0; 64],
            meip: false,
            seip: false,
            meip1: false,
            seip1: false,
            stale: true,
            rx_seen: 0,
            steps: 0,
            elapsed: 1,
        })));
        let model = Model {
            bus: Some(board.clone() as Rc<dyn Bus>),
            ..Model::default()
        };
        let model1 = Model {
            bus: Some(board.clone() as Rc<dyn Bus>),
            hartid: 1,
            ..Model::default()
        };
        Machine {
            model,
            board,
            profile: None,
            profile1: None,
            watch: None,
            model1,
            parked1: true,
            clock: (0, 0),
        }
    }

    /// Bytes laid down in the DDR3 at `addr`, as a loader does.
    pub fn load(&mut self, addr: u32, bytes: &[u8]) {
        let mut d = self.board.0.borrow_mut();
        assert!(
            d.ddr.holds(addr) && d.ddr.holds(addr + bytes.len() as u32 - 1),
            "{} bytes at {addr:#x} are not all in the DDR3",
            bytes.len()
        );
        d.ddr.put(addr, bytes);
    }

    /// The hart as a bootloader leaves it: at `entry`, with `a0` the
    /// hart's number and `a1` the device tree blob's address.
    pub fn boot(&mut self, entry: u32, dtb: u32) {
        self.model.pc = entry;
        self.model.x[10] = 0;
        self.model.x[11] = dtb;
    }

    /// One instruction, or the interrupt taken instead of it, after the
    /// devices' lines are given to the hart; then the count moves on.
    pub fn step(&mut self) {
        {
            let mut d = self.board.0.borrow_mut();
            let before = d.steps;
            d.steps += d.elapsed;
            // The fastboot client on the cable, every microsecond of the
            // core's: its stack moves on and what it sent goes onto the
            // wire towards the port (issue 1390). In the timing mode a
            // microsecond is a hundred cycles, not steps (issue 1392).
            if d.steps / 100 != before / 100 {
                let now = d.steps;
                if let Some(mut c) = d.eth.client.take() {
                    c.poll(now);
                    let out = c.take_sent();
                    d.eth.client = Some(c);
                    d.eth.inbox.extend(out);
                }
            }
            // A frame on the wire goes into a slot when the receive
            // side has room, as the store engine writes it.
            if !d.eth.inbox.is_empty() {
                let elapsed = d.elapsed as u32;
                if let Some((slot, f)) = d.eth.arrival_after(elapsed) {
                    let at = d.map.eth_bufs + slot * eth::SLOT;
                    d.ddr.put(at, &f);
                    d.stale = true;
                }
            }
            if d.stale || d.uart.rx.len() != d.rx_seen {
                let rx = d.uart.irq();
                d.plic.line(SERIAL_SOURCE, rx);
                let e = d.eth.irq();
                d.plic.line(ETH_SOURCE, e);
                // The second target, the supervisor's, is `mip.SEIP`'s
                // line (issue 1094).
                (d.meip, d.seip) = (d.plic.irq(0), d.plic.irq(1));
                (d.meip1, d.seip1) = (d.plic.irq(2), d.plic.irq(3));
                d.rx_seen = d.uart.rx.len();
                d.stale = false;
            }
            // `time` is the CLINT's count, as the board gives the core
            // the timer's (issue 1111).
            self.model.time = d.clint.mtime;
            self.model.tirq = d.clint.mtip();
            self.model.msip = d.clint.msip;
            let (meip, seip) = (d.meip, d.seip);
            // The second hart: started when its `msip` is set, as `park`
            // does, then given its lines as the first is.
            if self.parked1 && d.clint.msip1 {
                d.clint.msip1 = false;
                self.parked1 = false;
                self.model1.pc = d.clint.mbox_entry;
                self.model1.x[10] = 1;
                self.model1.x[11] = d.clint.mbox_arg;
                self.model1.csr.mie |= 1 << 3;
                self.model1.timing = self.model.timing.clone();
                self.clock.1 = self.clock.0;
            }
            self.model1.time = d.clint.mtime;
            self.model1.tirq = d.clint.mtip1();
            self.model1.msip = d.clint.msip1;
            let (meip1, seip1) = (d.meip1, d.seip1);
            drop(d);
            self.model.line(meip);
            self.model.sline(seip);
            self.model1.line(meip1);
            self.model1.sline(seip1);
        }
        let interrupt = self.model.interrupt();
        let was = self.model.cycles;
        let pc = self.model.pc;
        if let Some((at, callers)) = &mut self.watch {
            if pc == *at && interrupt.is_none() {
                let e = callers.entry(self.model.x[1]).or_insert((0, 0));
                e.0 += 1;
                e.1 += self.model.x[12] as u64;
            }
        }
        self.model.step(&[], interrupt);
        Self::takes(&self.model, &mut self.model1);
        // The timer counts the core's cycles: one a step, or in the
        // timing mode what the step was charged (issue 1392), so that a
        // timeout and the timer's ticks are in the same time as `mcycle`.
        let n = if self.model.timing.is_some() {
            self.model.cycles - was
        } else {
            1
        };
        if let Some(p) = &mut self.profile {
            let e = p.entry(pc).or_insert((0, 0));
            e.0 += n;
            e.1 += 1;
        }
        self.clock.0 += n;
        // The second hart, until it has caught up.
        while !self.parked1
            && self.clock.1 < self.clock.0
            && self.model1.halted.is_none()
        {
            let interrupt = self.model1.interrupt();
            let was = self.model1.cycles;
            let pc1 = self.model1.pc;
            self.model1.step(&[], interrupt);
            Self::takes(&self.model1, &mut self.model);
            let n1 = if self.model1.timing.is_some() {
                (self.model1.cycles - was).max(1)
            } else {
                1
            };
            if let Some(p) = &mut self.profile1 {
                let e = p.entry(pc1).or_insert((0, 0));
                e.0 += n1;
                e.1 += 1;
            }
            self.clock.1 += n1;
            if self.model1.pc == 0 {
                self.parked1 = true;
            }
        }
        let mut d = self.board.0.borrow_mut();
        d.clint.tick(n);
        d.elapsed = n;
    }

    /// A store by one hart takes the other's reservation of the same
    /// word (issue 1408).
    fn takes(by: &Model, other: &mut Model) {
        if let Some(addr) = by.wrote {
            if other.rsv_pa.is_some_and(|r| r & !3 == addr & !3) {
                other.rsv = None;
                other.rsv_pa = None;
            }
        }
    }

    /// Steps until the hart halts or `limit` instructions have run, and
    /// says how many ran.
    pub fn run(&mut self, limit: u64) -> u64 {
        let mut n = 0;
        while n < limit && self.model.halted.is_none() {
            self.step();
            n += 1;
        }
        n
    }

    /// Bytes arriving on the serial port's line, in order, as a terminal
    /// typed them; the port's receive queue takes them and its
    /// interrupt follows (issue 1127).
    pub fn type_bytes(&mut self, bytes: &[u8]) {
        self.board.0.borrow_mut().uart.rx.extend(bytes);
    }

    /// What the serial port has sent.
    pub fn sent(&self) -> Vec<u8> {
        self.board.0.borrow().uart.sent.clone()
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    /// The base and the bit that shows it, written as the boot shim
    /// writes them, say where the screen is (issue 1440).
    #[test]
    fn the_scanout_is_where_its_base_says_once_shown() {
        let mut m = Machine::new();
        let video = m.board.0.borrow().map.video.0;
        use crate::model::Bus;
        assert!(m.board.store(video + 0x80, 0x4200_0000, !0));
        assert_eq!(m.board.0.borrow().scanout(), None, "not shown yet");
        assert!(m.board.store(video + 0x84, 1, !0));
        assert_eq!(m.board.0.borrow().scanout(), Some(0x4200_0000));
        let _ = &mut m;
    }

    use super::*;

    /// The sources the model drives are the board's: the serial port's
    /// and the Ethernet port's, each where `board::PLIC_SOURCES` puts
    /// it, counting from one.
    #[test]
    fn the_models_plic_sources_are_the_boards() {
        let at = |what: &str| {
            1 + crate::board::PLIC_SOURCES
                .iter()
                .position(|s| *s == what)
                .unwrap()
        };
        assert_eq!(SERIAL_SOURCE, at("serial"));
        assert_eq!(ETH_SOURCE, at("ethernet"));
        assert_eq!(PLIC_SOURCES, crate::board::PLIC_SOURCES.len());
    }

    /// A small RV32I program in the DDR3: `lui t0, 0x3` puts the serial
    /// port's base in a register, three `sw`s write "Hi\n" to `txdata`,
    /// and writing `mhalt` halts, as the board's programs stop.
    #[test]
    fn a_program_in_the_ddr3_writes_to_the_serial_port() {
        fn i(w: u32) -> [u8; 4] {
            w.to_le_bytes()
        }
        let li = |rd: u32, imm: u32| (imm << 20) | (rd << 7) | 0x13;
        let sw =
            |rs2: u32, rs1: u32| (rs2 << 20) | (rs1 << 15) | (2 << 12) | 0x23;
        let mut prog = Vec::new();
        prog.extend(i((0x3 << 12) | (5 << 7) | 0x37)); // lui t0, 0x3
        for c in b"Hi\n" {
            prog.extend(i(li(6, *c as u32))); // addi t1, x0, c
            prog.extend(i(sw(6, 5))); // sw t1, 0(t0)
        }
        prog.extend(i((0x7c0 << 20) | (1 << 15) | (5 << 12) | 0x73)); // csrwi mhalt, 1
        let mut m = Machine::new();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0x4100_8000);
        let ran = m.run(100);
        assert!(m.model.halted.is_some(), "it halted, after {ran}");
        assert_eq!(m.sent(), b"Hi\n");
        assert_eq!(m.model.x[11], 0x4100_8000, "a1 is the blob's address");
    }

    /// The timer interrupts through the CLINT: a compare set, the count
    /// reaching it, and the model seeing its line.
    #[test]
    fn the_clint_counts_one_an_instruction() {
        let mut m = Machine::new();
        let nop = 0x0000_0013u32.to_le_bytes();
        let prog: Vec<u8> = (0..16).flat_map(|_| nop).collect();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0);
        m.board.0.borrow_mut().clint.mtimecmp = 5;
        m.run(4);
        assert!(!m.model.tirq);
        m.run(2);
        assert!(m.model.tirq, "the count passed the compare");
    }

    /// A byte arriving on the serial port, its source enabled for the
    /// controller's supervisor target only, raises `mip.SEIP` and not
    /// MEIP (issue 1094).
    #[test]
    fn the_supervisor_target_raises_seip() {
        use crate::isa::{MEXT, SEXT};
        let mut m = Machine::new();
        let nop = 0x0000_0013u32.to_le_bytes();
        let prog: Vec<u8> = (0..16).flat_map(|_| nop).collect();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0);
        {
            let mut d = m.board.0.borrow_mut();
            d.plic.store(4 * SERIAL_SOURCE as u32, 1);
            d.plic.store(plic::ENABLE[1], 1 << SERIAL_SOURCE);
            // The receive watermark's enable, which the program would
            // set, as both the model and the hardware reset it clear
            // (issues 1097 and 1137).
            d.uart.ie = 2;
            d.uart.rx.push_back(b'x');
        }
        m.run(2);
        assert_eq!(m.model.csr.mip & SEXT, SEXT, "the supervisor's line");
        assert_eq!(m.model.csr.mip & MEXT, 0, "and not the machine's");
    }

    /// `rdtime` reads the CLINT's count, one an instruction: three
    /// instructions before it, it reads three (issue 1111).
    #[test]
    fn time_is_the_clints_count() {
        use crate::isa::{csrrs, CSR_TIME};
        let mut m = Machine::new();
        let nop = 0x0000_0013u32;
        let halt = (0x7c0u32 << 20) | (1 << 15) | (5 << 12) | 0x73;
        let prog: Vec<u8> = [nop, nop, nop, csrrs(5, CSR_TIME, 0), halt]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0);
        m.run(10);
        assert!(m.model.halted.is_some());
        assert_eq!(m.model.x[5], 3, "the count after three instructions");
    }

    /// What is typed reaches a program that reads the port: an echo
    /// loop sends back each byte it receives and stops at a newline
    /// (issue 1127).
    #[test]
    fn typed_bytes_reach_a_program_that_reads_the_port() {
        use crate::isa::{addi, blt, bne, halt, lui, lw, sw};
        let prog: Vec<u8> = [
            lui(5, 3),      // t0 = the serial port, 0x3000
            lw(6, 5, 4),    // t1 = rxdata
            blt(6, 0, -4),  // nothing yet: bit 31 is set
            sw(6, 5, 0),    // send it back
            addi(7, 0, 10), // t2 = '\n'
            bne(6, 7, -16), // until a newline
            halt(),
        ]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
        let mut m = Machine::new();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0);
        m.run(50);
        assert!(m.sent().is_empty(), "nothing typed, nothing sent");
        m.type_bytes(b"hi\n");
        m.run(200);
        assert!(m.model.halted.is_some(), "the newline stopped it");
        assert_eq!(m.sent(), b"hi\n");
    }

    /// A load from where nothing is, between the devices, faults.
    #[test]
    fn nothing_answers_outside_the_map() {
        let b = Machine::new().board;
        assert_eq!(b.load(0x9000_0000), None);
        assert!(!b.store(0x9000_0000, 1, u32::MAX));
        assert_eq!(b.load(0x4000_0000), Some(0));
    }

    /// `rd` gets `v`, in two words.
    fn li(rd: u32, v: u32) -> [u32; 2] {
        use crate::isa::{addi, lui};
        let hi = v.wrapping_add(0x800) >> 12;
        [lui(rd, hi), addi(rd, rd, v.wrapping_sub(hi << 12) as i32)]
    }

    /// A thousand adds with `amoadd.w` to one counter and a
    /// thousand with `lr.w` and `sc.w` to another, in x5 and x16.
    fn adds(p: &mut Vec<u32>) {
        use crate::isa::{addi, amoadd_w, bne, lr_w, sc_w};
        p.extend(li(5, 0x4000_0100));
        p.extend(li(6, 1));
        p.extend(li(16, 0x4000_0200));
        p.extend(li(10, 1000));
        p.extend([
            amoadd_w(0, 5, 6),
            lr_w(20, 16),
            addi(20, 20, 1),
            sc_w(21, 16, 20),
            bne(21, 0, -12),
            addi(10, 10, -1),
            bne(10, 0, -24),
        ]);
    }

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// The second hart (issue 1408): the first starts it through the
    /// mailbox; each adds one to a counter a thousand times with
    /// `amoadd.w` and to another a thousand times with `lr.w` and
    /// `sc.w`, side by side, and neither count loses one; then the second
    /// says it is done and jumps back to wait. In steps and in cycles.
    #[test]
    fn two_harts_lose_no_count() {
        use crate::isa::{beq, halt, jalr, lw, sw, CLINT_BASE};
        use crate::model::Bus;
        const JOB: u32 = 0x4001_0000;
        const FLAG: u32 = 0x4000_0300;
        for timing in [false, true] {
            let mut m = Machine::new();
            if timing {
                m.model.timing = Some(crate::model::Timing::board());
            }
            let mut job = vec![];
            adds(&mut job);
            job.extend(li(22, 1));
            job.extend([sw(22, 11, 0), jalr(0, 0, 0)]);
            m.load(JOB, &bytes(&job));
            let mut p = vec![];
            p.extend(li(9, CLINT_BASE + 0xc000));
            p.extend(li(8, JOB));
            p.push(sw(8, 9, 0));
            p.extend(li(12, FLAG));
            p.push(sw(12, 9, 4));
            p.extend(li(13, CLINT_BASE));
            p.extend(li(14, 1));
            p.push(sw(14, 13, 4));
            adds(&mut p);
            p.extend([lw(15, 12, 0), beq(15, 0, -4), halt()]);
            m.load(0x4000_0000, &bytes(&p));
            m.boot(0x4000_0000, 0);
            m.run(1_000_000);
            assert!(
                m.model.halted.is_some(),
                "hart 0 finished, timing {timing}"
            );
            assert!(m.parked1, "hart 1 went back to wait");
            assert_eq!(m.board.load(0x4000_0100), Some(2000), "the AMOs");
            assert_eq!(m.board.load(0x4000_0200), Some(2000), "the pairs");
        }
    }

    /// The second hart's lines (issue 1408): a byte on the serial port,
    /// its source enabled for the controller's third target alone, raises
    /// hart 1's `mip.MEIP` and not hart 0's; hart 1's `msip` and compare
    /// raise its software and timer bits.
    #[test]
    fn the_second_harts_lines_are_its_own() {
        use crate::isa::MEXT;
        let mut m = Machine::new();
        let nop = 0x0000_0013u32.to_le_bytes();
        let prog: Vec<u8> = (0..16).flat_map(|_| nop).collect();
        m.load(0x4000_0000, &prog);
        m.boot(0x4000_0000, 0);
        {
            let mut d = m.board.0.borrow_mut();
            d.plic.store(4 * SERIAL_SOURCE as u32, 1);
            d.plic.store(plic::ENABLE[2], 1 << SERIAL_SOURCE);
            d.uart.ie = 2;
            d.uart.rx.push_back(b'x');
            d.clint.store(clint::MTIMECMP1 + 4, 0);
            d.clint.store(clint::MTIMECMP1, 0);
        }
        m.run(2);
        assert_eq!(m.model1.csr.mip & MEXT, MEXT, "hart 1's external line");
        assert_eq!(m.model.csr.mip & MEXT, 0, "and not hart 0's");
        assert!(m.model1.tirq, "hart 1's timer");
        assert!(!m.model.tirq, "and not hart 0's timer");
        // Hart 1's software interrupt, while it waits, starts it rather
        // than interrupting it, so it is seen once it has started.
        assert!(m.parked1, "hart 1 waits");
    }
}
