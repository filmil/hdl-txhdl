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
//! The four registers and their fields are declared once with
//! `regmap!` below (issue 676): `ctrl`, `cmd`, `data` and `state`.
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
use txhdl::comp::{join2, mux, until, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, regmap, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{map}
regmap! { regs (regs_read, regs_we), 2: [
    (0, ctrl, rw, "the divider and the interrupt enable", [
        (div, 0, 16, rw, 0, "a quarter of a bit is this many cycles, less one"),
        (ie, 16, 1, rw, 0, "a finished command raises the interrupt"),
    ]),
    (1, cmd, wo, "one piece of a transaction; written, it starts", [
        (start, 0, 1, wo, 0, "send a start, or a repeated start"),
        (stop, 1, 1, wo, 0, "send a stop when the byte is done"),
        (write, 2, 1, wo, 0, "write the byte"),
        (read, 3, 1, wo, 0, "read a byte"),
        (nack, 4, 1, wo, 0, "answer a read with a NACK rather than an ACK"),
        (byte, 8, 8, wo, 0, "the byte to write"),
    ]),
    (2, data, ro, "the byte a read took"),
    (3, state, w1c, "busy, done, not acknowledged, arbitration lost", [
        (busy, 0, 1, ro, 0, "a command is running"),
        (fired, 1, 1, w1c, 0, "a command has finished"),
        (nack, 2, 1, w1c, 0, "the device did not acknowledge the byte"),
        (lost, 3, 1, w1c, 0, "another master won the bus"),
    ]),
] }
// end{map}

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
    /// The clock line pulled low by the engine; clear is released.
    pub scl_pull: Reg<Bit>,
    /// The data line pulled low by the engine; clear is released.
    pub sda_pull: Reg<Bit>,
    /// Up for the one cycle after a command ends, when what it found
    /// is latched.
    pub done: Reg<Bit>,
    /// The command's byte was not acknowledged, as the engine saw it.
    pub nack_hit: Reg<Bit>,
    /// The engine lost the bus to another master during the command.
    pub lost_hit: Reg<Bit>,
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
/// Whether a write to `cmd` starts a command this cycle: the bus has a
/// write, it is to `cmd`, and no command is running.
#[lower]
fn cmd_go(wgo: Bit, wsel: U<2>, busy: Bit) -> Bit {
    // `cmd`'s write enable, bit 1 of the map's.
    regs_we(wgo, wsel).bit(1) & !busy
}

/// Whether the quarter of a bit ends this cycle: a command is running,
/// the clock is not being stretched by a device holding it low where
/// the master released it, and the quarter's cycles are up.
#[lower]
fn q_go(busy: Bit, scl_pull: Bit, scl_in: Bit, tick: U<16>, div: U<16>) -> Bit {
    busy & (scl_pull | scl_in) & Bit::from(tick == div)
}

#[lower]
impl Unit for I2c {
    /// Two processes. The bus process answers the registers every
    /// cycle, counts the cycles of a quarter, and latches what a
    /// command found once it is done. The engine is the transaction
    /// written as the sequence it is: a wait for a command, then the
    /// start, the eight bits, the acknowledge and the stop, each a
    /// quarter of a bit at a time, with the lines set for each quarter
    /// before its wait. Every wait after the first is the end of a
    /// quarter, and the lowering numbers them into the state
    /// register the hand-written version kept as `step` and
    /// `quarter`.
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (scl_in, sda_in, scl_low, sda_low, irq): (
            In<Bit>,
            In<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    let div = self.div.get();
                    let ie = self.ie.get();
                    let busy = self.busy.get();
                    let fired = self.fired.get();
                    let nack = self.nack.get();
                    let lost = self.lost.get();
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
                    // A quarter of a bit is `div` cycles and one, and
                    // the count pauses while a device stretches the
                    // clock.
                    let stretch = !(self.scl_pull.get() | scl_in.get());
                    let running = busy & !stretch;
                    let q_last = tick == div;
                    // The word a read answers, the fields packed as the
                    // map places them; `cmd` is written only, and reads
                    // zero.
                    let word = regs_read(
                        rsel,
                        regs_ctrl_pack(div, ie),
                        U::<32>::from(0u8),
                        self.data.get().zext::<32>(),
                        regs_state_pack(busy, fired, nack, lost),
                    );
                    let we = regs_we(wgo, wsel);
                    let clearing = we.bit(3);
                    let done = self.done.get();
                    with!(self <= {
                        we.bit(0) ? {
                            div: regs_ctrl_div(written),
                            ie: regs_ctrl_ie(written),
                        },
                        starting ? tick: U::<16>::from(0u8),
                        !starting & running ? tick: mux(
                            q_last,
                            U::<16>::from(0u8),
                            tick + 1
                        ),
                        starting ? {
                            fired: Bit::Zero,
                            nack: Bit::Zero,
                            lost: Bit::Zero,
                        },
                        done ? {
                            fired: Bit::One,
                            nack: self.nack_hit.get(),
                            lost: self.lost_hit.get(),
                        },
                        clearing & regs_state_fired(written) ? fired: Bit::Zero,
                        clearing & regs_state_nack(written) ? nack: Bit::Zero,
                        clearing & regs_state_lost(written) ? lost: Bit::Zero,
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
                    // The lines as the engine pulls them; a line not
                    // pulled is released and reads high.
                    scl_low.set(self.scl_pull.get());
                    sda_low.set(self.sda_pull.get());
                    irq.set(fired & ie);
                }
            },
            async {
                loop {
                    // Idle until a write to `cmd`, which is taken as it
                    // is written: what the command asks for, and the
                    // byte to send.
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
                        req_start: regs_cmd_start(written),
                        req_stop: regs_cmd_stop(written),
                        req_write: regs_cmd_write(written),
                        req_read: regs_cmd_read(written),
                        req_nack: regs_cmd_nack(written),
                        shift: regs_cmd_byte(written),
                        nack_hit: Bit::Zero,
                        lost_hit: Bit::Zero,
                        regs_cmd_start(written) ? held: Bit::One,
                    });
                    // The start, or a repeated start: the data line
                    // falls while the clock is high, then the clock
                    // falls.
                    if written.bit(0).to_bool() {
                        with!(self <= {
                            scl_pull: Bit::Zero,
                            sda_pull: Bit::Zero,
                        });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { sda_pull: Bit::One });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { scl_pull: Bit::One });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                    }
                    // The byte, high bit first, and the acknowledge
                    // after it. A bit is set up while the clock is
                    // low, carried through the two quarters the clock
                    // is high, and read at the end of the second of
                    // them. A master writing a one that reads back a
                    // zero has lost the bus to another master: it
                    // releases both lines for the rest of the byte,
                    // which the counted loop still walks through, and
                    // skips the acknowledge and the stop.
                    if (self.req_write.get() | self.req_read.get()).to_bool() {
                        for _ in 0..8 {
                            let lost = self.lost_hit.get();
                            with!(self <= {
                                scl_pull: !lost,
                                sda_pull: mux(
                                    self.req_write.get() & !lost,
                                    !self.shift.get().bit(7),
                                    Bit::Zero
                                ),
                            });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            with!(self <= { scl_pull: Bit::Zero });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            let taken = sda_in.get();
                            let shift = self.shift.get();
                            let keeping = !self.lost_hit.get();
                            with!(self <= {
                                self.req_read.get() & keeping ? shift:
                                    (shift << 1) | taken.zext::<8>(),
                                self.req_write.get() & keeping ? shift:
                                    shift << 1,
                                self.req_write.get() & keeping
                                    & shift.bit(7) & !taken
                                    ? lost_hit: Bit::One,
                                scl_pull: keeping,
                            });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                        }
                        // The acknowledge: the master releases the
                        // line for a byte it wrote and reads the
                        // device's answer, or answers a byte it read.
                        if !self.lost_hit.get().to_bool() {
                            with!(self <= {
                                self.req_read.get() ? data: self.shift.get(),
                                scl_pull: Bit::One,
                                sda_pull: mux(
                                    self.req_write.get(),
                                    Bit::Zero,
                                    !self.req_nack.get()
                                ),
                            });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            with!(self <= { scl_pull: Bit::Zero });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                            with!(self <= {
                                self.req_write.get() ? nack_hit: sda_in.get(),
                                scl_pull: Bit::One,
                            });
                            until(DefaultClock::rising, || {
                                q_go(
                                    self.busy.get(),
                                    self.scl_pull.get(),
                                    scl_in.get(),
                                    self.tick.get(),
                                    self.div.get(),
                                )
                                .to_bool()
                            })
                            .await;
                        }
                    }
                    // The stop: the data line rises while the clock is
                    // high, and the transaction is over.
                    if (self.req_stop.get() & !self.lost_hit.get()).to_bool() {
                        with!(self <= {
                            scl_pull: Bit::One,
                            sda_pull: Bit::One,
                        });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { scl_pull: Bit::Zero });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { sda_pull: Bit::Zero });
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        until(DefaultClock::rising, || {
                            q_go(
                                self.busy.get(),
                                self.scl_pull.get(),
                                scl_in.get(),
                                self.tick.get(),
                                self.div.get(),
                            )
                            .to_bool()
                        })
                        .await;
                        with!(self <= { held: Bit::Zero });
                    }
                    // The end: the lines parked, with the clock low
                    // while the transaction stays open, and released
                    // when it is over or the bus was lost; `done` is
                    // up for one cycle, which is when the bus process
                    // latches what the command found, and `busy` falls
                    // a cycle after it.
                    let over = self.req_stop.get() | self.lost_hit.get();
                    with!(self <= {
                        self.lost_hit.get() ? held: Bit::Zero,
                        scl_pull: !over & self.held.get(),
                        sda_pull: Bit::Zero,
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
    /// The divider and the interrupt enable.
    pub const CTRL: u32 = regs::ctrl;
    /// One piece of a transaction; writing it starts the command.
    pub const CMD: u32 = regs::cmd;
    /// The byte a read took.
    pub const DATA: u32 = regs::data;
    /// Busy, done, not acknowledged, arbitration lost.
    pub const STATE: u32 = regs::state;
}

/// The bits of a command word, from the map.
pub mod cmd {
    use super::regs;
    /// Send a start, or a repeated start.
    pub const START: u32 = regs::cmd_start.mask();
    /// Send a stop when the byte is done.
    pub const STOP: u32 = regs::cmd_stop.mask();
    /// Write the byte in bits 8 to 15.
    pub const WRITE: u32 = regs::cmd_write.mask();
    /// Read a byte.
    pub const READ: u32 = regs::cmd_read.mask();
    /// Answer a read with a NACK rather than an ACK.
    pub const NACK: u32 = regs::cmd_nack.mask();
    /// The byte to write, shifted into place.
    pub fn byte(v: u8) -> u32 {
        regs::cmd_byte.with(v as u32)
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
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LitePort, LiteW};
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
        let (dev, ended, _) = on_the_bus_with(dev, |_| false, client);
        (dev, ended)
    }

    /// What the lines did in one cycle: whether this master pulled the
    /// clock and the data line, whether the rival pulled the data
    /// line, and how the two lines read.
    type Cycle = (bool, bool, bool, bool, bool);

    /// `on_the_bus` with another master on the data line: `rival`
    /// says, per cycle, whether it pulls the line low. Returns what
    /// the lines did, a record per cycle.
    fn on_the_bus_with<F>(
        dev: I2cDev,
        rival: impl Fn(u64) -> bool,
        client: impl FnOnce(Host) -> F,
    ) -> (I2cDev, bool, Vec<Cycle>)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
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
            master.run(bus, (scl_in, sda_in, scl_low_o, sda_low_o, irq_o)),
        ));
        let dev = RefCell::new(dev);
        let mut lines: Vec<Cycle> = Vec::new();
        scl_in_o.set(Bit::One);
        sda_in_o.set(Bit::One);
        for cycle in 0..40000u64 {
            sim.cycle();
            let (sc, sd) = (scl_low.get().to_bool(), sda_low.get().to_bool());
            let other = rival(cycle);
            let mut d = dev.borrow_mut();
            // The device sees the line as everybody pulls it.
            d.step(sc, sd || other);
            let scl = !sc && !d.pulls_scl();
            let sda = !sd && !d.pulls_sda() && !other;
            scl_in_o.set(Bit::from_bool(scl));
            sda_in_o.set(Bit::from_bool(sda));
            drop(d);
            lines.push((sc, sd, other, scl, sda));
            if *done.borrow() {
                return (dev.into_inner(), true, lines);
            }
        }
        (dev.into_inner(), false, lines)
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

    /// Another master holds the data line low through the address this
    /// one sends, and wins the third bit, where this one drove a one.
    /// The specification (NXP UM10204, arbitration) asks the loser to
    /// stop driving the data line at once, and lets it clock on to the
    /// end of the byte or not; so the master reports arbitration lost
    /// and never pulls the data line again in that command, and
    /// nothing is said about its clock.
    #[test]
    fn a_master_that_loses_arbitration_lets_go_of_the_data_line() {
        let dev = I2cDev::new(0x50, &[0; 4]);
        let (_, ended, lines) = on_the_bus_with(
            dev,
            |_| true,
            |h| async move {
                poke(&h, reg::CTRL, DIV).await;
                let state = command(
                    &h,
                    cmd::START | cmd::WRITE | cmd::STOP | cmd::byte(0x20),
                )
                .await;
                assert_eq!(state & 8, 8, "arbitration lost");
            },
        );
        assert!(ended, "the client finished");
        // Where it lost: past the bits it drove, the first cycle the
        // clock read high while the master had released the data line
        // and the rival held it.
        let drove = lines
            .iter()
            .position(|&(_, sda_pull, ..)| sda_pull)
            .expect("the master drove the zeros first");
        let lost = drove
            + lines[drove..]
                .iter()
                .position(|&(scl_pull, sda_pull, other, scl, _)| {
                    scl && !scl_pull && !sda_pull && other
                })
                .expect("the master drove a one against the rival");
        assert!(
            lines[..lost].iter().any(|&(_, sda_pull, ..)| sda_pull),
            "the master drove the zeros before it"
        );
        assert!(
            lines[lost..].iter().all(|&(_, sda_pull, ..)| !sda_pull),
            "the data line is never pulled again"
        );
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
