// SPDX-License-Identifier: Apache-2.0
//! A general I2C master on AXI-Lite, and a model of a device to check
//! it against.
//!
//! `I2cInit` of [`hdmi`](crate::hdmi) replays a fixed list of writes
//! and cannot read. This master is the other kind: a program says what
//! one piece of a transaction is and the master does it, so a driver
//! can address a device, write to it, turn the bus around and read
//! from it.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | the divider in 0 to 15, the interrupt enable |
//! | `0x04` | `cmd` | one piece of a transaction; writing it starts it |
//! | `0x08` | `data` | read: the byte that came in |
//! | `0x0c` | `state` | busy, done, not acknowledged, arbitration lost |
//!
//! A quarter of a bit takes `div + 1` cycles, so the bus runs at the
//! system clock over `4 * (div + 1)`: 249 is 100 kbit/s from 100 MHz,
//! and 62 is 400.
//!
//! `cmd` is bit 0 a start, bit 1 a stop, bit 2 a write, bit 3 a read,
//! bit 4 the answer to give after a read (set for a NACK, which is
//! what a master sends after the last byte it wants), and bits 8 to 15
//! the byte to write. One command is at most a start, one byte either
//! way, and a stop, which is how a driver writes an address and then
//! reads: `start|write` with the address, then `start|read|nack` with
//! the repeated start.
//!
//! `state` bit 1 is written with one to clear, as is bit 2 and bit 3;
//! a command clears all three itself. The interrupt is bit 1 while the
//! enable is set.
//!
//! The two lines are open drain. `scl_low` and `sda_low` high pull a
//! line low, and a line nobody pulls is high through the board's
//! pull-up, so a board wrapper drives a pad from each and reads the
//! pad back into `scl_in` and `sda_in`. The master watches both: a
//! device that holds the clock low stretches it, and the master waits
//! rather than counting; a data line that reads low while the master
//! released it is another master winning the bus, which sets
//! arbitration lost and ends the command.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};

// begin{state}
/// An I2C master: one piece of a transaction per command, on two open
/// drain lines.
#[derive(Trace, Default)]
pub struct I2c {
    /// A quarter of a bit is this many cycles, less one.
    pub div: Reg<U<16>>,
    /// Whether a finished command raises the interrupt.
    pub ie: Reg<Bit>,
    /// The command asked for a start, or a repeated start.
    pub req_start: Reg<Bit>,
    /// The command asked for a stop once the byte is done.
    pub req_stop: Reg<Bit>,
    /// The command asked to write the byte in `shift`.
    pub req_write: Reg<Bit>,
    /// The command asked to read a byte.
    pub req_read: Reg<Bit>,
    /// The answer to give after a read: set for a NACK.
    pub req_nack: Reg<Bit>,
    /// A command is running.
    pub busy: Reg<Bit>,
    /// A command has finished and nothing has cleared it.
    pub fired: Reg<Bit>,
    /// The device did not acknowledge the byte written.
    pub nack: Reg<Bit>,
    /// Another master held the data line low while this one released
    /// it.
    pub lost: Reg<Bit>,
    /// Where in the transaction the run is: idle 0, the start 1, the
    /// eight bits 2 to 9, the acknowledge 10, the stop 11, the end 12.
    pub step: Reg<U<4>>,
    /// Which quarter of the bit the lines are in.
    pub quarter: Reg<U<2>>,
    /// Cycles into the quarter.
    pub tick: Reg<U<16>>,
    /// The byte going out, its next bit on top, or the byte coming in,
    /// its first bit at the bottom by the end.
    pub shift: Reg<U<8>>,
    /// The byte a read took.
    pub data: Reg<U<8>>,
    /// A transaction is open: a start has been sent and no stop has.
    /// The lines are parked with the clock low between the commands of
    /// one transaction, since a data line that moves while the clock
    /// is high is a start or a stop to every device on the bus.
    pub held: Reg<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl Unit for I2c {
    async fn run(
        &mut self,
        (aw, ar, w, scl_in, sda_in): (
            Rx<LiteAw<32>>,
            Rx<LiteAr<32>>,
            Rx<LiteW<32, 4>>,
            In<Bit>,
            In<Bit>,
        ),
        (b, r, scl_low, sda_low, irq): (
            Tx<LiteB>,
            Tx<LiteR<32>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let div = self.div.get();
            let ie = self.ie.get();
            let busy = self.busy.get();
            let fired = self.fired.get();
            let nack = self.nack.get();
            let lost = self.lost.get();
            let step = self.step.get();
            let quarter = self.quarter.get();
            let tick = self.tick.get();
            let shift = self.shift.get();
            let writing = self.req_write.get();
            let reading = self.req_read.get();
            // Where in the transaction the step is.
            let at_start = step == 1;
            let at_bits = (step >= 2) & (step <= 9);
            let at_ack = step == 10;
            let at_stop = step == 11;
            let at_end = step == 12;
            // The lines as the step and the quarter want them, high
            // meaning released. A start pulls the data line low while
            // the clock is high, and a stop releases it while the
            // clock is high; a bit is carried through the two middle
            // quarters, where the clock is high.
            let clock_in_bit = (quarter == 1) | (quarter == 2);
            let parked = !self.held.get();
            let scl_want = mux(
                at_start,
                Bit::from(quarter < 2),
                mux(
                    at_bits | at_ack,
                    Bit::from(clock_in_bit),
                    mux(at_stop, Bit::from(quarter != 0), parked),
                ),
            );
            let bit_out = mux(writing, shift.bit(7), Bit::One);
            let ack_out = mux(writing, Bit::One, self.req_nack.get());
            let sda_want = mux(
                at_start,
                Bit::from(quarter == 0),
                mux(
                    at_bits,
                    bit_out,
                    mux(
                        at_ack,
                        ack_out,
                        mux(at_stop, Bit::from(quarter >= 2), Bit::One),
                    ),
                ),
            );
            // A device that holds the clock low while the master
            // released it is stretching, and the run waits for it.
            let stretch = scl_want & !scl_in.get();
            let running = busy & !stretch;
            let q_last = tick == div;
            let q_go = running & Bit::from(q_last);
            let step_go = q_go & (quarter == 3);
            // The lines are read in the middle of the clock's high
            // half, which is the end of the second quarter.
            let sample = q_go & (quarter == 2);
            let taken = sda_in.get();
            let took_bit = sample & at_bits & reading;
            let took_ack = sample & at_bits & writing;
            let heard = sample & at_ack & writing;
            // Another master won the bus: the data line is low where
            // this one released it.
            let stolen = took_ack & shift.bit(7) & !taken;
            // The bus.
            let arh = ar.head();
            let awh = aw.head();
            let wh = w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            let rgo = r.ready() & ar.peek().is_some();
            let _ = ar.recv_if(r.ready());
            let wgo = b.ready() & aw.peek().is_some() & w.peek().is_some();
            let _ = aw.recv_if(wgo);
            let _ = w.recv_if(wgo);
            let written = wh.data;
            // A write to `cmd` starts a command, and is ignored while
            // one is running: a program reads `state` first.
            let cmd_go = wgo & (wsel == 1) & !busy;
            let wants_byte = written.bit(2) | written.bit(3);
            // Where a command with no start begins: at the byte, or
            // at the stop when it asked for neither.
            let first_step =
                mux(wants_byte, U::<4>::from(2u8), U::<4>::from(11u8));
            // Where the step goes next: past the bits when the command
            // wants none, past the stop when it asked for none.
            let after_start =
                mux(writing | reading, U::<4>::from(2u8), U::<4>::from(11u8));
            let after_ack = mux(
                self.req_stop.get(),
                U::<4>::from(11u8),
                U::<4>::from(12u8),
            );
            let next_step = select!(step.raw() => {
                1 => after_start,
                9 => U::<4>::from(10u8),
                10 => after_ack,
                11 => U::<4>::from(12u8),
                _ => step + 1,
            });
            let ctrl = ie.zext::<1>().concat::<16, 17>(div).zext::<32>();
            let state = lost
                .zext::<1>()
                .concat::<1, 2>(nack.zext::<1>())
                .concat::<1, 3>(fired.zext::<1>())
                .concat::<1, 4>(busy.zext::<1>())
                .zext::<32>();
            let word = select!(rsel.raw() => {
                0 => ctrl,
                2 => self.data.get().zext::<32>(),
                3 => state,
                _ => U::<32>::from(0u8),
            });
            let clearing = wgo & (wsel == 3);
            with!(self <= {
                wgo & (wsel == 0) ? {
                    div: written.slice::<0, 16>(),
                    ie: written.bit(16),
                },
                cmd_go & written.bit(0) ? held: Bit::One,
                cmd_go ? {
                    busy: Bit::One,
                    fired: Bit::Zero,
                    nack: Bit::Zero,
                    lost: Bit::Zero,
                    req_start: written.bit(0),
                    req_stop: written.bit(1),
                    req_write: written.bit(2),
                    req_read: written.bit(3),
                    req_nack: written.bit(4),
                    shift: written.slice::<8, 8>(),
                    step: mux(written.bit(0), U::<4>::from(1u8), first_step),
                    quarter: U::<2>::from(0u8),
                    tick: U::<16>::from(0u8),
                },
                running ? tick: mux(q_last, U::<16>::from(0u8), tick + 1),
                q_go ? quarter: quarter + 1,
                took_bit ? shift: (shift << 1) | taken.zext::<8>(),
                heard ? nack: taken,
                stolen ? lost: Bit::One,
                step_go ? step: next_step,
                step_go & at_bits ? shift: mux(reading, shift, shift << 1),
                step_go & (step == 9) & reading ? data: shift,
                step_go & at_stop ? held: Bit::Zero,
                at_end ? {
                    busy: Bit::Zero,
                    fired: Bit::One,
                    step: U::<4>::from(0u8),
                },
                stolen ? {
                    busy: Bit::Zero,
                    fired: Bit::One,
                    step: U::<4>::from(0u8),
                },
                clearing & written.bit(1) ? fired: Bit::Zero,
                clearing & written.bit(2) ? nack: Bit::Zero,
                clearing & written.bit(3) ? lost: Bit::Zero,
            });
            if rgo.to_bool() {
                r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                b.send(LiteB { resp: Resp::Okay });
            }
            scl_low.set(!scl_want);
            sda_low.set(!sda_want);
            irq.set(fired & ie);
        }
    }
}
// end{run}

/// The offsets of the master's four words.
pub mod reg {
    /// The divider and the interrupt enable.
    pub const CTRL: u32 = 0x0;
    /// One piece of a transaction; writing it starts the command.
    pub const CMD: u32 = 0x4;
    /// The byte a read took.
    pub const DATA: u32 = 0x8;
    /// Busy, done, not acknowledged, arbitration lost.
    pub const STATE: u32 = 0xc;
}

/// The bits of a command word.
pub mod cmd {
    /// Send a start, or a repeated start.
    pub const START: u32 = 1 << 0;
    /// Send a stop when the byte is done.
    pub const STOP: u32 = 1 << 1;
    /// Write the byte in bits 8 to 15.
    pub const WRITE: u32 = 1 << 2;
    /// Read a byte.
    pub const READ: u32 = 1 << 3;
    /// Answer a read with a NACK rather than an ACK.
    pub const NACK: u32 = 1 << 4;
    /// The byte to write, shifted into place.
    pub fn byte(v: u8) -> u32 {
        (v as u32) << 8
    }
}

/// A device on the bus, written as a simulation: a model to check the
/// master against, which the master's example and tests step between
/// cycles as the HDMI document's I2C device is stepped.
pub mod sim {
    /// A device with a byte of memory per register, addressed as most
    /// small I2C parts are: a write of one byte sets the register
    /// pointer, a write of another stores there, and a read gives the
    /// byte the pointer names and moves it on.
    ///
    /// It acknowledges its own address and nothing else, and it can be
    /// told to hold the clock low for a few cycles after each byte,
    /// which is what clock stretching is.
    pub struct I2cDev {
        /// The device's seven-bit address.
        pub addr: u8,
        /// The registers, one byte each.
        pub regs: Vec<u8>,
        /// Cycles to hold the clock low after a byte.
        pub stretch: u32,
        scl: bool,
        sda: bool,
        /// Held: the transaction is open; addressed: the address byte
        /// matched; reading: the master asked for bytes.
        held: bool,
        addressed: bool,
        reading: bool,
        first: bool,
        bits: u32,
        byte: u8,
        pointer: usize,
        /// The bit the device presents while the master reads, and
        /// whether it is pulling the line for an acknowledge.
        driving: bool,
        acking: bool,
        out: u8,
        stretching: u32,
        /// Every byte the master wrote, in order.
        pub written: Vec<u8>,
    }

    impl I2cDev {
        /// A device at `addr` whose registers hold `regs`.
        pub fn new(addr: u8, regs: &[u8]) -> Self {
            I2cDev {
                addr,
                regs: regs.to_vec(),
                stretch: 0,
                scl: true,
                sda: true,
                held: false,
                addressed: false,
                reading: false,
                first: false,
                bits: 0,
                byte: 0,
                pointer: 0,
                driving: false,
                acking: false,
                out: 0,
                stretching: 0,
                written: Vec::new(),
            }
        }

        /// What the device is doing, for a test that has to see why a
        /// byte went astray: the bits counted, the byte so far,
        /// whether a transaction is open, whether the address matched,
        /// whether the master asked to read, and whether the device is
        /// acknowledging.
        pub fn probe(&self) -> (u32, u8, bool, bool, bool, bool) {
            (
                self.bits,
                self.byte,
                self.held,
                self.addressed,
                self.reading,
                self.acking,
            )
        }

        /// Whether the device pulls the data line low now.
        pub fn pulls_sda(&self) -> bool {
            self.acking || (self.driving && self.out & 0x80 == 0)
        }

        /// Whether the device holds the clock low now.
        pub fn pulls_scl(&self) -> bool {
            self.stretching > 0
        }

        /// One cycle of the lines. `scl_low` and `sda_low` are the
        /// master's pulls; a line is high when nobody pulls it.
        pub fn step(&mut self, scl_low: bool, sda_low: bool) {
            let scl = !scl_low && !self.pulls_scl();
            let sda = !sda_low && !self.pulls_sda();
            if self.stretching > 0 {
                self.stretching -= 1;
            }
            // A start or a stop, which are the two moves of the data
            // line while the clock is high.
            if self.scl && scl {
                if self.sda && !sda {
                    self.held = true;
                    self.addressed = false;
                    self.reading = false;
                    self.bits = 0;
                    self.byte = 0;
                    self.driving = false;
                } else if !self.sda && sda && self.held {
                    self.held = false;
                    self.acking = false;
                    self.driving = false;
                }
            }
            // A bit is taken on the clock's rising edge, and the
            // device's own bit moves on the falling one.
            if !self.scl && scl && self.held {
                if self.acking {
                    // The acknowledge the device is giving.
                } else if self.reading {
                    self.bits += 1;
                    if self.bits == 8 {
                        // The master's answer comes on the ninth bit.
                    }
                } else {
                    self.byte = (self.byte << 1) | u8::from(sda);
                    self.bits += 1;
                }
            }
            if self.scl && !scl && self.held {
                if self.acking {
                    self.acking = false;
                    self.bits = 0;
                    self.stretching = self.stretch;
                    if self.reading && self.first {
                        self.first = false;
                        self.load();
                    }
                } else if self.reading {
                    if self.bits == 8 {
                        self.bits = 0;
                        self.stretching = self.stretch;
                        self.load();
                    } else {
                        self.out <<= 1;
                    }
                } else if self.bits == 8 {
                    self.take();
                }
            }
            self.scl = scl;
            self.sda = sda;
        }

        /// The byte the pointer names goes out, and the pointer moves.
        fn load(&mut self) {
            self.out = *self.regs.get(self.pointer).unwrap_or(&0xff);
            self.pointer = (self.pointer + 1) % self.regs.len().max(1);
            self.driving = true;
        }

        /// A whole byte arrived: the address, or a byte written.
        fn take(&mut self) {
            let byte = self.byte;
            self.bits = 0;
            self.byte = 0;
            if !self.addressed {
                if byte >> 1 == self.addr {
                    self.addressed = true;
                    self.acking = true;
                    self.reading = byte & 1 == 1;
                    self.driving = false;
                    if self.reading {
                        self.first = true;
                    }
                }
            } else {
                self.written.push(byte);
                if self.written.len() == 1 {
                    self.pointer = byte as usize % self.regs.len().max(1);
                } else {
                    let at = self.pointer;
                    if at < self.regs.len() {
                        self.regs[at] = byte;
                    }
                    self.pointer = (at + 1) % self.regs.len().max(1);
                }
                self.acking = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sim::I2cDev;
    use super::{cmd, reg, I2c};
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Clock, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};

    type Host = LiteHost<32, 32, 4>;

    /// A quarter of a bit in these runs: four cycles, so a bit is
    /// sixteen and a byte with its acknowledge is a hundred and forty
    /// four.
    const DIV: u32 = 3;

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

    /// One command, and the wait for it: the state word once it is no
    /// longer busy.
    async fn command(h: &Host, word: u32) -> u32 {
        poke(h, reg::CMD, word).await;
        loop {
            let state = peek(h, reg::STATE).await;
            if state & 1 == 0 {
                return state;
            }
        }
    }

    /// Run `client` against the master, with `dev` on the two lines.
    /// The device is stepped between cycles, as the HDMI document's is,
    /// and the lines are open drain: high unless somebody pulls.
    fn on_the_bus<F>(
        dev: I2cDev,
        client: impl FnOnce(Host) -> F,
    ) -> (I2cDev, bool)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let (aw, ar, w, b, r) = link.per;
        let (scl_in_o, scl_in) = signal::<Bit, DefaultClock>();
        let (sda_in_o, sda_in) = signal::<Bit, DefaultClock>();
        let (scl_low_o, scl_low) = signal::<Bit, DefaultClock>();
        let (sda_low_o, sda_low) = signal::<Bit, DefaultClock>();
        let (irq_o, _irq) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let fin = done.clone();
        let body = client(link.host);
        let mut master = I2c::default();
        let mut sim = Running::new(join2(
            async move {
                body.await;
                *fin.borrow_mut() = true;
            },
            master.run(
                (aw, ar, w, scl_in, sda_in),
                (b, r, scl_low_o, sda_low_o, irq_o),
            ),
        ));
        let dev = RefCell::new(dev);
        scl_in_o.set(Bit::One);
        sda_in_o.set(Bit::One);
        for _ in 0..40000 {
            sim.cycle();
            let (sc, sd) = (scl_low.get().to_bool(), sda_low.get().to_bool());
            let mut d = dev.borrow_mut();
            d.step(sc, sd);
            scl_in_o.set(Bit::from_bool(!sc && !d.pulls_scl()));
            sda_in_o.set(Bit::from_bool(!sd && !d.pulls_sda()));
            drop(d);
            if *done.borrow() {
                return (dev.into_inner(), true);
            }
        }
        (dev.into_inner(), false)
    }

    /// The address and two bytes, as a driver writes a register of a
    /// device: the device acknowledges each, and the second byte lands
    /// where the first pointed.
    #[test]
    fn a_write_reaches_the_device() {
        let dev = I2cDev::new(0x50, &[0; 8]);
        let (dev, ended) = on_the_bus(dev, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            let state =
                command(&h, cmd::START | cmd::WRITE | cmd::byte(0x50 << 1))
                    .await;
            assert_eq!(state & 4, 0, "the address was acknowledged");
            command(&h, cmd::WRITE | cmd::byte(5)).await;
            let state =
                command(&h, cmd::WRITE | cmd::STOP | cmd::byte(0x42)).await;
            assert_eq!(state & 4, 0, "every byte was acknowledged");
        });
        assert!(ended, "the client finished");
        assert_eq!(dev.written, vec![5, 0x42], "the bytes the device took");
        assert_eq!(dev.regs[5], 0x42, "the register the pointer named");
    }

    /// A read, after a repeated start: the byte the pointer names comes
    /// back in `data`.
    #[test]
    fn a_read_takes_a_byte_from_the_device() {
        let dev = I2cDev::new(0x50, &[0x11, 0x22, 0x33, 0x44]);
        let (_, ended) = on_the_bus(dev, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            command(&h, cmd::START | cmd::WRITE | cmd::byte(0x50 << 1)).await;
            command(&h, cmd::WRITE | cmd::byte(2)).await;
            // The repeated start turns the bus around, and the last
            // byte a master wants is answered with a NACK.
            command(&h, cmd::START | cmd::WRITE | cmd::byte((0x50 << 1) | 1))
                .await;
            let state = command(&h, cmd::READ | cmd::NACK | cmd::STOP).await;
            assert_eq!(state & 4, 0, "the address was acknowledged");
            assert_eq!(peek(&h, reg::DATA).await, 0x33, "the third register");
        });
        assert!(ended, "the client finished");
    }

    /// A device that is not there acknowledges nothing, and the master
    /// says so.
    #[test]
    fn an_address_nobody_answers_sets_nack() {
        let dev = I2cDev::new(0x50, &[0; 4]);
        let (dev, ended) = on_the_bus(dev, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            let state = command(
                &h,
                cmd::START | cmd::WRITE | cmd::STOP | cmd::byte(0x20 << 1),
            )
            .await;
            assert_eq!(state & 4, 4, "not acknowledged");
            // Writing the bit back clears it.
            poke(&h, reg::STATE, 4).await;
            assert_eq!(peek(&h, reg::STATE).await & 4, 0, "cleared");
        });
        assert!(ended, "the client finished");
        assert!(dev.written.is_empty(), "the device took nothing");
    }

    /// A device that holds the clock low after each byte is waited for,
    /// and the transaction still goes through.
    #[test]
    fn a_device_that_stretches_the_clock_is_waited_for() {
        let mut dev = I2cDev::new(0x50, &[0; 8]);
        dev.stretch = 9;
        let (dev, ended) = on_the_bus(dev, |h| async move {
            poke(&h, reg::CTRL, DIV).await;
            command(&h, cmd::START | cmd::WRITE | cmd::byte(0x50 << 1)).await;
            command(&h, cmd::WRITE | cmd::byte(1)).await;
            let state =
                command(&h, cmd::WRITE | cmd::STOP | cmd::byte(0x77)).await;
            assert_eq!(state & 4, 0, "every byte was acknowledged");
        });
        assert!(ended, "the client finished");
        assert_eq!(dev.regs[1], 0x77, "the byte landed through the stretch");
    }

    /// The master lowers: one module, with the two open drain pulls and
    /// the interrupt as ports.
    #[test]
    fn the_master_lowers() {
        let v = I2c::verilog("i2c");
        assert!(v.contains("module i2c("), "the module");
        for port in ["scl_low", "sda_low", "scl_in", "sda_in", "irq"] {
            assert!(v.contains(port), "{port}");
        }
    }
}
