// SPDX-License-Identifier: Apache-2.0
//! An MDIO master on AXI-Lite, the management interface of an Ethernet
//! PHY, and a model of a PHY to check it against (issue 864).
//!
//! MDIO is IEEE 802.3 clause 22: a clock, MDC, which the master always
//! drives, and one data line, MDIO, which the master drives except
//! while the PHY answers a read. A frame is 32 ones of preamble and
//! then 32 bits, high bit first: the start `01`, the operation (`10` a
//! read, `01` a write), the PHY's address in five bits, the register's
//! in five, two bits of turnaround, and the sixteen bits of data. On a
//! read the master lets go of the line for the turnaround, the PHY
//! drives a zero in its second bit, and then the data.
//!
//! The four registers and their fields are declared once with
//! `regmap!` below: `ctrl`, `cmd`, `data` and `state`.
//!
//! Half of an MDC cycle is `div + 1` system cycles, so MDC is the
//! system clock over `2 * (div + 1)`: 49 is 1 MHz from 100 MHz. 802.3
//! allows MDC up to 2.5 MHz. `div` resets to zero, which is far too
//! fast for any PHY, so a driver writes `ctrl` before its first frame.
//!
//! Writing `cmd` starts a frame: bits 0 to 4 the register, 5 to 9 the
//! PHY's address, bit 10 set for a write, and bits 16 to 31 the word
//! to write. `state` bit 0 is busy; bit 1 is set when a frame ends and
//! is written with one to clear; a command clears it itself. `data` is
//! the word the last read took, once busy has fallen.
//!
//! A bit is set on the line while MDC is low and taken by the PHY as
//! MDC rises. The master takes a bit the PHY drives at the end of the
//! low half, just before MDC rises, which is a whole MDC cycle after
//! the PHY changed it: 802.3 gives the PHY up to 300 ns for that, and
//! at 1 MHz there are 1000.
//!
//! The line is three-state. `mdio_out` is the level and `mdio_oe` high
//! drives it, so a board wrapper drives the pad from the two and reads
//! the pad back into `mdio_in`. The board has a pull-up on MDIO, as
//! 802.3 asks, so a line nobody drives reads one.
use txhdl::comp::{join2, mux, until, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{map}
regmap! { regs (regs_read, regs_we), 2: [
    (0, ctrl, rw, "the divider", [
        (div, 0, 8, rw, 0, "half of an MDC cycle is this many cycles, less one"),
    ]),
    (1, cmd, wo, "one frame; written, it starts", [
        (regad, 0, 5, wo, 0, "the register"),
        (phyad, 5, 5, wo, 0, "the PHY's address"),
        (write, 10, 1, wo, 0, "write the word rather than read"),
        (word, 16, 16, wo, 0, "the word to write"),
    ]),
    (2, data, ro, "the word the last read took"),
    (3, state, w1c, "busy and done", [
        (busy, 0, 1, ro, 0, "a frame is running"),
        (fired, 1, 1, w1c, 0, "a frame has finished"),
    ]),
] }
// end{map}

/// The master's lines, named as the netlist names them.
pub struct MdioLines {
    /// The data line, as read at the pad.
    pub mdio_in: In<Bit>,
    /// The management clock.
    pub mdc: Out<Bit>,
    /// The level the master drives on the data line.
    pub mdio_out: Out<Bit>,
    /// High while the master drives the data line.
    pub mdio_oe: Out<Bit>,
}

// begin{state}
/// An MDIO master: one clause 22 frame per command.
#[derive(Trace, Default)]
pub struct Mdio {
    /// Half of an MDC cycle is this many cycles, less one.
    pub div: Reg<U<8>>,
    /// A frame is running.
    pub busy: Reg<Bit>,
    /// A frame has finished and nothing has cleared it.
    pub fired: Reg<Bit>,
    /// Up for the one cycle after a frame ends.
    pub done: Reg<Bit>,
    /// Cycles into the half of an MDC cycle.
    pub tick: Reg<U<8>>,
    /// The 32 bits after the preamble, the next one on top.
    pub frame: Reg<U<32>>,
    /// Which of those bits the master drives, the next one on top.
    pub drive: Reg<U<32>>,
    /// The bits the line carried, the last one at the bottom; after a
    /// read, the data.
    pub rx: Reg<U<16>>,
    /// The clock, as driven.
    pub mdc_q: Reg<Bit>,
    /// The data line's level, as driven.
    pub out: Reg<Bit>,
    /// Whether the master drives the data line.
    pub oe: Reg<Bit>,
}
// end{state}

// begin{run}
/// Whether a write to `cmd` starts a frame this cycle: the bus has a
/// write, it is to `cmd`, and no frame is running.
#[lower]
fn cmd_go(wgo: Bit, wsel: U<2>, busy: Bit) -> Bit {
    // `cmd`'s write enable, bit 1 of the map's.
    regs_we(wgo, wsel).bit(1) & !busy
}

/// Whether the half of an MDC cycle ends this cycle.
#[lower]
fn h_go(busy: Bit, tick: U<8>, div: U<8>) -> Bit {
    busy & Bit::from(tick == div)
}

/// The 32 bits after the preamble: the start, the operation, the two
/// addresses, the turnaround and the word. A read's turnaround and
/// data are ones, which the master does not drive.
#[lower]
fn frame_of(written: U<32>) -> U<32> {
    let wr = regs_cmd_write(written);
    let op = mux(wr, U::<32>::from(0x5u8), U::<32>::from(0x6u8));
    let tail = mux(
        wr,
        U::<32>::from(0x2_0000u32) | regs_cmd_word(written).zext::<32>(),
        U::<32>::from(0x3_ffffu32),
    );
    (op << 28)
        | (regs_cmd_phyad(written).zext::<32>() << 23)
        | (regs_cmd_regad(written).zext::<32>() << 18)
        | tail
}

/// Which of the 32 bits the master drives: all of a write, and the
/// first fourteen of a read, up to the turnaround.
#[lower]
fn drive_of(written: U<32>) -> U<32> {
    mux(
        regs_cmd_write(written),
        U::<32>::from(0xffff_ffffu32),
        U::<32>::from(0xfffc_0000u32),
    )
}

#[lower]
impl Unit for Mdio {
    /// Two processes, as the I2C master has. The bus process answers
    /// the registers every cycle, counts the cycles of a half, and
    /// marks a frame done. The engine is the frame written as the
    /// sequence it is: a wait for a command, the preamble, then the 32
    /// bits, each a low half and a high half.
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        MdioLines {
            mdio_in,
            mdc,
            mdio_out,
            mdio_oe,
        }: MdioLines,
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let div = self.div.get();
                    let busy = self.busy.get();
                    let fired = self.fired.get();
                    let tick = self.tick.get();
                    // The bus.
                    let arh = bus.ar.head();
                    let awh = bus.aw.head();
                    let wh = bus.w.head();
                    let rsel = arh.addr.slice::<2, 2>();
                    let wsel = awh.addr.slice::<2, 2>();
                    let rgo = bus.r.ready() & bus.ar.peek().is_some();
                    let _ = bus.ar.recv_if(bus.r.ready());
                    let wgo = bus.b.ready()
                        & bus.aw.peek().is_some()
                        & bus.w.peek().is_some();
                    let _ = bus.aw.recv_if(wgo);
                    let _ = bus.w.recv_if(wgo);
                    let written = wh.data;
                    let starting = cmd_go(wgo, wsel, busy);
                    let h_last = tick == div;
                    // The word a read answers; `cmd` is written only,
                    // and reads zero.
                    let word = regs_read(
                        rsel,
                        regs_ctrl_pack(div),
                        U::<32>::from(0u8),
                        self.rx.get().zext::<32>(),
                        regs_state_pack(busy, fired),
                    );
                    let we = regs_we(wgo, wsel);
                    let clearing = we.bit(3);
                    with!(self <= {
                        we.bit(0) ? div: regs_ctrl_div(written),
                        starting ? tick: U::<8>::from(0u8),
                        !starting & busy ? tick: mux(
                            h_last,
                            U::<8>::from(0u8),
                            tick + 1
                        ),
                        starting ? fired: Bit::Zero,
                        self.done.get() ? fired: Bit::One,
                        clearing & regs_state_fired(written) ? fired: Bit::Zero,
                    });
                    if rgo.to_bool() {
                        bus.r.send(LiteR {
                            data: word,
                            resp: Resp::Okay,
                        });
                    }
                    if wgo.to_bool() {
                        bus.b.send(LiteB { resp: Resp::Okay });
                    }
                    mdc.set(self.mdc_q.get());
                    mdio_out.set(self.out.get());
                    mdio_oe.set(self.oe.get());
                }
            },
            async {
                loop {
                    // Idle until a write to `cmd`, which is taken as it
                    // is written.
                    until(DefaultClock::rising, || {
                        cmd_go(
                            bus.b.ready()
                                & bus.aw.peek().is_some()
                                & bus.w.peek().is_some(),
                            bus.aw.head().addr.slice::<2, 2>(),
                            self.busy.get(),
                        )
                        .to_bool()
                    })
                    .await;
                    let written = bus.w.head().data;
                    with!(self <= {
                        busy: Bit::One,
                        frame: frame_of(written),
                        drive: drive_of(written),
                    });
                    // The preamble: 32 ones, driven.
                    for _ in 0..32 {
                        with!(self <= {
                            mdc_q: Bit::Zero,
                            out: Bit::One,
                            oe: Bit::One,
                        });
                        until(DefaultClock::rising, || {
                            h_go(
                                self.busy.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { mdc_q: Bit::One });
                        until(DefaultClock::rising, || {
                            h_go(
                                self.busy.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                    }
                    // The frame: each bit put on the line while MDC is
                    // low, or the line let go, and the line taken at
                    // the end of the low half.
                    for _ in 0..32 {
                        with!(self <= {
                            mdc_q: Bit::Zero,
                            out: self.frame.get().bit(31),
                            oe: self.drive.get().bit(31),
                        });
                        until(DefaultClock::rising, || {
                            h_go(
                                self.busy.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        let taken = mdio_in.get();
                        with!(self <= {
                            mdc_q: Bit::One,
                            rx: (self.rx.get() << 1) | taken.zext::<16>(),
                            frame: self.frame.get() << 1,
                            drive: self.drive.get() << 1,
                        });
                        until(DefaultClock::rising, || {
                            h_go(
                                self.busy.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                    }
                    // The end: the clock parked low and the line let
                    // go; `done` is up for one cycle, and `busy` falls
                    // a cycle after it.
                    with!(self <= {
                        mdc_q: Bit::Zero,
                        oe: Bit::Zero,
                        done: Bit::One,
                    });
                    DefaultClock::rising().await;
                    with!(self <= { done: Bit::Zero, busy: Bit::Zero });
                }
            },
        )
        .await;
    }
}
// end{run}

/// The offsets of the master's four words, from the map.
pub mod reg {
    use super::regs;
    /// The divider.
    pub const CTRL: u32 = regs::ctrl;
    /// One frame; writing it starts the frame.
    pub const CMD: u32 = regs::cmd;
    /// The word the last read took.
    pub const DATA: u32 = regs::data;
    /// Busy and done.
    pub const STATE: u32 = regs::state;
}

/// Command words, from the map.
pub mod cmd {
    use super::regs;
    /// Read register `regad` of the PHY at `phyad`.
    pub fn read(phyad: u8, regad: u8) -> u32 {
        regs::cmd_phyad.with(phyad as u32) | regs::cmd_regad.with(regad as u32)
    }
    /// Write `word` to register `regad` of the PHY at `phyad`.
    pub fn write(phyad: u8, regad: u8, word: u16) -> u32 {
        read(phyad, regad)
            | regs::cmd_write.mask()
            | regs::cmd_word.with(word as u32)
    }
}

/// A model of a PHY's management interface, to check the master
/// against.
pub mod sim {
    /// A clause 22 PHY at one address with 32 registers. It takes a bit
    /// as MDC rises, and on a read to its address drives the
    /// turnaround's zero and the sixteen bits, each changed just after
    /// MDC rises.
    pub struct MdioPhy {
        /// The PHY's address.
        pub addr: u8,
        /// Its registers.
        pub regs: [u16; 32],
        /// Every frame it took: the PHY's address, the register's,
        /// and the word written, or `None` for a read.
        pub frames: Vec<(u8, u8, Option<u16>)>,
        mdc: bool,
        ones: u32,
        pos: Option<u32>,
        bits: u32,
        answer: Option<u16>,
        drive: Option<bool>,
    }

    impl MdioPhy {
        /// A PHY at `addr` holding `regs`.
        pub fn new(addr: u8, regs: [u16; 32]) -> Self {
            MdioPhy {
                addr,
                regs,
                frames: Vec::new(),
                mdc: false,
                ones: 0,
                pos: None,
                bits: 0,
                answer: None,
                drive: None,
            }
        }

        /// The level the PHY drives on the data line, if it drives it.
        pub fn drives(&self) -> Option<bool> {
            self.drive
        }

        /// One system cycle: `mdc` and the data line as it reads.
        pub fn step(&mut self, mdc: bool, mdio: bool) {
            let rose = mdc && !self.mdc;
            self.mdc = mdc;
            if !rose {
                return;
            }
            let Some(p) = self.pos else {
                // Waiting for a start after at least 32 ones.
                if mdio {
                    self.ones += 1;
                } else {
                    if self.ones >= 32 {
                        self.pos = Some(1);
                        self.bits = 0;
                    }
                    self.ones = 0;
                }
                return;
            };
            self.bits = (self.bits << 1) | mdio as u32;
            let p = p + 1;
            self.pos = Some(p);
            if p == 2 && self.bits & 3 != 1 {
                // Not a clause 22 start.
                self.pos = None;
                return;
            }
            if p == 14 {
                let op = (self.bits >> 10) & 3;
                let phy = ((self.bits >> 5) & 31) as u8;
                let reg = (self.bits & 31) as u8;
                if op == 2 {
                    self.frames.push((phy, reg, None));
                    if phy == self.addr {
                        self.answer = Some(self.regs[reg as usize]);
                    }
                }
            }
            if let Some(word) = self.answer {
                // The turnaround's zero, then the word, high bit first.
                self.drive = match p {
                    15 => Some(false),
                    16..=31 => Some((word >> (31 - p)) & 1 == 1),
                    _ => None,
                };
            }
            if p == 32 {
                let op = (self.bits >> 28) & 3;
                if op == 1 {
                    let phy = ((self.bits >> 23) & 31) as u8;
                    let reg = ((self.bits >> 18) & 31) as u8;
                    let word = self.bits as u16;
                    self.frames.push((phy, reg, Some(word)));
                    if phy == self.addr {
                        self.regs[reg as usize] = word;
                    }
                }
                self.pos = None;
                self.answer = None;
                self.drive = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sim::MdioPhy;
    use super::MdioLines;
    use super::{cmd, reg, Mdio};
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LitePort, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Clock, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};

    type Host = LiteHost<32, 32, 4>;

    /// Half of an MDC cycle in these runs: three cycles, so a frame is
    /// 64 bits of six cycles.
    const DIV: u32 = 2;

    async fn poke(h: &Host, off: u32, word: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(off),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(word),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if b.recv().is_some() {
                return;
            }
        }
    }

    async fn peek(h: &Host, off: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAw {
            addr: U::from(off),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(got) = r.recv() {
                return got.data.raw() as u32;
            }
        }
    }

    /// One frame, and the wait for it: the state word once it is no
    /// longer busy.
    async fn frame(h: &Host, word: u32) -> u32 {
        poke(h, reg::CMD, word).await;
        loop {
            let state = peek(h, reg::STATE).await;
            if state & 1 == 0 {
                return state;
            }
        }
    }

    /// Run `client` against the master with `phy` on the lines. The
    /// line reads what the master drives, else what the PHY drives,
    /// else one through the pull-up. Returns the PHY, whether the
    /// client finished, and every cycle's (mdc, master drives, PHY
    /// drives).
    fn on_the_bus<F>(
        phy: MdioPhy,
        client: impl FnOnce(Host) -> F,
    ) -> (MdioPhy, bool, Vec<(bool, bool, bool)>)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let (in_o, mdio_in) = signal::<Bit, DefaultClock>();
        let (mdc_o, mdc) = signal::<Bit, DefaultClock>();
        let (out_o, out) = signal::<Bit, DefaultClock>();
        let (oe_o, oe) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let fin = done.clone();
        let body = client(link.host);
        let mut master = Mdio::default();
        let mut sim = Running::new(join2(
            async move {
                body.await;
                *fin.borrow_mut() = true;
            },
            master.run(
                bus,
                MdioLines {
                    mdio_in,
                    mdc: mdc_o,
                    mdio_out: out_o,
                    mdio_oe: oe_o,
                },
            ),
        ));
        let mut phy = phy;
        let mut lines = Vec::new();
        in_o.set(Bit::One);
        for _ in 0..20000u64 {
            sim.cycle();
            let (c, m, e) =
                (mdc.get().to_bool(), out.get().to_bool(), oe.get().to_bool());
            let line = if e { m } else { phy.drives().unwrap_or(true) };
            phy.step(c, line);
            let line = if e { m } else { phy.drives().unwrap_or(true) };
            in_o.set(Bit::from_bool(line));
            lines.push((c, e, phy.drives().is_some()));
            if *done.borrow() {
                return (phy, true, lines);
            }
        }
        (phy, false, lines)
    }

    fn regs() -> [u16; 32] {
        let mut r = [0u16; 32];
        for (i, w) in r.iter_mut().enumerate() {
            *w = (0x1000 * (i as u16 % 16)).wrapping_add(0x0101 * i as u16);
        }
        r[2] = 0x937c;
        r[3] = 0x4023;
        r
    }

    /// A read of the PHY's two identifier registers: each word comes
    /// back in `data`, and the PHY saw two read frames.
    #[test]
    fn a_read_returns_the_register() {
        let phy = MdioPhy::new(1, regs());
        let got = Rc::new(RefCell::new(Vec::new()));
        let put = got.clone();
        let (phy, ended, _) = on_the_bus(phy, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            for r in [2u8, 3] {
                let state = frame(&h, cmd::read(1, r)).await;
                assert_eq!(state & 2, 2, "the frame is marked done");
                let w = peek(&h, reg::DATA).await;
                put.borrow_mut().push(w);
            }
        });
        assert!(ended, "the client finished");
        assert_eq!(*got.borrow(), vec![0x937c, 0x4023], "the identifier");
        assert_eq!(
            phy.frames,
            vec![(1, 2, None), (1, 3, None)],
            "the frames the PHY took"
        );
    }

    /// Every one of the 32 registers reads back as the PHY holds it.
    #[test]
    fn every_register_reads_back() {
        let want = regs();
        let phy = MdioPhy::new(7, want);
        let got = Rc::new(RefCell::new(Vec::new()));
        let put = got.clone();
        let (_, ended, _) = on_the_bus(phy, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            for r in 0..32u8 {
                frame(&h, cmd::read(7, r)).await;
                let w = peek(&h, reg::DATA).await as u16;
                put.borrow_mut().push(w);
            }
        });
        assert!(ended, "the client finished");
        assert_eq!(*got.borrow(), want.to_vec(), "all 32 registers");
    }

    /// A read to an address nobody answers: the line stays high, so
    /// the word is all ones, which is how a driver finds the PHY.
    #[test]
    fn an_absent_phy_reads_all_ones() {
        let phy = MdioPhy::new(1, regs());
        let got = Rc::new(RefCell::new(0));
        let put = got.clone();
        let (phy, ended, _) = on_the_bus(phy, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            frame(&h, cmd::read(4, 2)).await;
            *put.borrow_mut() = peek(&h, reg::DATA).await;
        });
        assert!(ended, "the client finished");
        assert_eq!(*got.borrow(), 0xffff, "nobody drove the line");
        assert_eq!(phy.frames, vec![(4, 2, None)], "the PHY saw the frame");
    }

    /// A write lands in the register it names, and a read after it
    /// takes it back.
    #[test]
    fn a_write_lands() {
        let phy = MdioPhy::new(3, regs());
        let got = Rc::new(RefCell::new(0));
        let put = got.clone();
        let (phy, ended, _) = on_the_bus(phy, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            frame(&h, cmd::write(3, 31, 0xa5c3)).await;
            frame(&h, cmd::read(3, 31)).await;
            *put.borrow_mut() = peek(&h, reg::DATA).await;
        });
        assert!(ended, "the client finished");
        assert_eq!(phy.regs[31], 0xa5c3, "the register written");
        assert_eq!(*got.borrow(), 0xa5c3, "and read back");
    }

    /// The master and the PHY never drive the line in the same cycle,
    /// and MDC holds each level for a whole half: 3 cycles at
    /// `DIV` = 2.
    #[test]
    fn the_line_is_never_driven_twice() {
        let phy = MdioPhy::new(1, regs());
        let (_, ended, lines) = on_the_bus(phy, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            frame(&h, cmd::read(1, 2)).await;
            frame(&h, cmd::write(1, 0, 0x1140)).await;
        });
        assert!(ended, "the client finished");
        assert!(
            lines.iter().all(|&(_, m, p)| !(m && p)),
            "one driver at a time"
        );
        let mut run = 0;
        let mut last = lines[0].0;
        let mut runs = Vec::new();
        for &(c, _, _) in &lines[1..] {
            run += 1;
            if c != last {
                runs.push(run);
                run = 0;
                last = c;
            }
        }
        assert!(runs.len() > 200, "the clock ran: {} edges", runs.len());
        assert!(
            runs[1..].iter().all(|&r| r > DIV as usize),
            "every half lasts at least div + 1 cycles: {runs:?}"
        );
    }

    /// The master lowers: one module, with the clock, the line's three
    /// signals and the bus as ports.
    #[test]
    fn the_master_lowers() {
        let v = Mdio::verilog("mdio");
        assert!(v.contains("module mdio("), "the module");
        for port in ["mdc", "mdio_out", "mdio_oe", "mdio_in"] {
            assert!(v.contains(port), "{port}");
        }
    }
}
