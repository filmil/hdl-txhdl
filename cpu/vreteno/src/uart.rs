// SPDX-License-Identifier: Apache-2.0
//! The serial port: an AXI-Lite peripheral with a line each way. It
//! sits behind a bridge from the core's AXI4 link, which hands it one
//! transaction at a time with no identifier and no burst, so a read is
//! an address in and a word out, and a write is an address and a word
//! in and a response out.
//!
//! A byte written to its first word goes out on the line as a start
//! bit, eight data bits least significant first and a stop bit, each
//! `DIV` cycles long; a frame coming in on the other line, sampled in
//! the middle of each bit, lands in a buffer of eight behind its third
//! word, where a load takes the oldest, and the port's interrupt line
//! is high while the buffer holds any. The second word is the status:
//! bit 0 busy while a byte is going out, bit 1 a byte received and
//! not yet read, bit 2 the buffer full. A byte written while busy is
//! dropped, and a byte received when the buffer is full is dropped
//! and counted, so a program polls the status and keeps up. The
//! lines rest high. `DIV` is 868 for 115200 baud at 100 MHz, and 4 in
//! the runs that are checked, so that a byte takes forty cycles rather
//! than nine thousand.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::regmap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};

// The pieces of the step, each a function of its own, inlined by the
// lowering where the step calls it.

/// The frame a byte goes out as, least significant bit first: a start
/// bit low, the byte, a stop bit high.
#[lower]
fn frame(octet: U<8>) -> U<10> {
    U::<1>::from(1u8)
        .concat::<_, 9>(octet)
        .concat::<_, 10>(U::<1>::from(0u8))
}

/// The frame after a bit has gone out: shifted down, the line's rest
/// state shifted in at the top.
#[lower]
fn shifted(shift: U<10>) -> U<10> {
    U::<1>::from(1u8).concat::<_, 10>(shift.slice::<1, 9>())
}

/// Whether the current bit's last cycle has come.
#[lower]
fn last_cycle<const DIV: u32>(tick: U<32>) -> bool {
    tick == DIV - 1
}

/// Whether the current bit's middle cycle has come, where the line is
/// sampled.
#[lower]
fn middle<const DIV: u32>(tick: U<32>) -> bool {
    tick == DIV / 2
}

/// A frame coming in, one more bit taken at the top: bit 0 of the
/// byte comes first and ends up lowest.
#[lower]
fn taken_in(shift: U<8>, line: Bit) -> U<8> {
    line.zext::<1>().concat::<_, 8>(shift.slice::<1, 7>())
}

// begin{map}
// The map: three words, two address bits above the byte bits selecting
// one, and the fourth reads zero (issues 499 and 669).
regmap! { serial (serial_read, serial_we, serial_re), 2: [
    (0, tx, rw, "written, a byte to send; read, the last byte sent", [
        (data, 0, 8, rw, 0, "the byte"),
    ]),
    (1, status, ro, "what the port is doing", [
        (busy, 0, 1, ro, 0, "a byte is going out"),
        (ready, 1, 1, ro, 0, "a byte received and not yet read"),
        (full, 2, 1, ro, 0, "the buffer of eight is full"),
    ]),
    (2, rx, rc, "the oldest byte received; the read takes it", [
        (data, 0, 8, rc, 0, "the byte"),
    ]),
] }
// end{map}

#[derive(Trace, Default)]
pub struct Uart<const DIV: u32> {
    /// The frame going out, least significant bit first.
    pub shift: Reg<U<10>>,
    /// Bits left to send; busy while not zero.
    pub bits: Reg<U<4>>,
    /// Cycles left in the current bit.
    pub tick: Reg<U<32>>,
    /// The last byte accepted, and how many were.
    pub last: Reg<U<8>>,
    pub sent: Reg<U<8>>,
    /// The line in, as the edge left it: one register between the
    /// pin and the logic.
    pub line: Reg<Bit>,
    /// The frame coming in, the bits left of it, start and stop
    /// included, and the cycles into the current bit.
    pub rx_shift: Reg<U<8>>,
    pub rx_bits: Reg<U<4>>,
    pub rx_tick: Reg<U<32>>,
    /// The bytes received and not yet read: a buffer of eight, the
    /// index of the oldest and how many are held; how many came in
    /// all, and how many found the buffer full and were dropped.
    pub fifo: Mem<U<8>, 8>,
    pub head: Reg<U<3>>,
    pub count: Reg<U<4>>,
    pub received: Reg<U<8>>,
    pub dropped: Reg<U<8>>,
}

#[lower]
impl<const DIV: u32> Unit for Uart<DIV> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (rst, rx, tx, irq): (In<Bit>, In<Bit>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get().to_bool();
            let rx_ready = self.count != 0;
            let full = self.count == 8;
            let rx_data = self.fifo.read(self.head.get());
            let busy = self.bits != 0;
            // The bridge sends this peripheral only the transactions in
            // its range, so it checks no address. A read is answered in
            // the cycle it is taken, and a write is taken when its
            // address and its word are both there, and answered at once.
            let arh = bus.ar.head();
            let take_read = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let sel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            // The map's enables: which word a write goes to, and whether
            // this read is the one that takes a received byte.
            let we = serial_we(wgo, wsel);
            let re = serial_re(take_read, sel);
            let read_rx = re.bit(2).to_bool();
            // A byte written while idle starts a frame, ten bits of DIV
            // cycles each; a reset wins over everything.
            let start = we.bit(0).to_bool() & !busy;
            let octet = serial_tx_data(wh.data);
            if rst {
                self.bits.set(0);
                self.tick.set(0);
            } else if start {
                self.shift.set(frame(octet));
                self.bits.set(10);
                self.tick.set(0);
                self.last.set(octet);
                self.sent.set(self.sent + 1);
            } else if busy {
                if last_cycle::<DIV>(self.tick.get()) {
                    self.tick.set(0);
                    self.bits.set(self.bits - 1);
                    self.shift.set(shifted(self.shift.get()));
                } else {
                    self.tick.set(self.tick + 1);
                }
            }
            if take_read.to_bool() {
                bus.r.send(LiteR {
                    data: serial_read(
                        sel,
                        serial_tx_pack(self.last.get()),
                        serial_status_pack(
                            Bit::from(busy),
                            Bit::from(rx_ready),
                            Bit::from(full),
                        ),
                        serial_rx_pack(rx_data),
                    ),
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            tx.set(mux(busy, self.shift.get().bit(0), Bit::One));
            // The receive side. A low on the resting line is a start
            // bit; from then on the line is sampled in the middle of
            // each bit, ten of them: a high where the start bit should
            // be is a false start and the frame is dropped, the eight
            // bits in between are taken into the shift register, and a
            // high at the stop bit lands the byte in the buffer, whose
            // oldest the read of the third word takes; the buffer full,
            // the byte is dropped and counted.
            self.line.set(rx.get());
            let receiving = self.rx_bits != 0;
            let sample = !rst & receiving & middle::<DIV>(self.rx_tick.get());
            let at_start = self.rx_bits == 10;
            let at_stop = self.rx_bits == 1;
            let line = self.line.to_bool();
            if rst {
                self.rx_bits.set(0);
                self.rx_tick.set(0);
                self.head.set(0);
                self.count.set(0);
            } else if !receiving {
                if !line {
                    self.rx_bits.set(10);
                    self.rx_tick.set(0);
                }
            } else if last_cycle::<DIV>(self.rx_tick.get()) {
                self.rx_tick.set(0);
                self.rx_bits.set(self.rx_bits - 1);
            } else {
                self.rx_tick.set(self.rx_tick + 1);
            }
            with!(self <= {
                sample & at_start & line ? rx_bits: 0,
                sample & !at_start & !at_stop ?
                    rx_shift: taken_in(self.rx_shift.get(), self.line.get()),
                sample & at_stop ? rx_bits: 0,
            });
            let landed = sample & at_stop & line;
            let push = landed & !full;
            let pop = read_rx & rx_ready;
            let tail = self.head + self.count.get().slice::<0, 3>();
            if landed {
                if full {
                    self.dropped.set(self.dropped + 1);
                } else {
                    self.fifo.at(tail).set(self.rx_shift);
                    self.received.set(self.received + 1);
                }
            }
            if !rst {
                if pop {
                    self.head.set(self.head + 1);
                }
                if push & !pop {
                    self.count.set(self.count + 1);
                } else if pop & !push {
                    self.count.set(self.count - 1);
                }
            }
            irq.set(rx_ready);
        }
    }
}
