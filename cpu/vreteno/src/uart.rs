// SPDX-License-Identifier: Apache-2.0
//! The serial port: a device on the bus with a line each way. A byte
//! written to its first word goes out on the line as a start bit,
//! eight data bits least significant first and a stop bit, each
//! `DIV` cycles long; a frame coming in on the other line, sampled in
//! the middle of each bit, lands in a buffer of eight behind its third
//! word, where a load takes the oldest, and the port's interrupt line
//! is high while the buffer holds any. The second word is the status:
//! bit 0 busy while a byte is going out, bit 1 a byte received and
//! not yet read, bit 2 the buffer full. A byte written while busy is
//! dropped, and a byte received when the buffer is full is dropped
//! and counted, so a program polls the status and keeps up. The
//! lines rest high. `DIV` is 868 for
//! 115200 baud at 100 MHz, and 4 in the runs that are checked, so
//! that a byte takes forty cycles rather than nine thousand.
use crate::bus::{REQ_ADDR, REQ_WDATA};
use crate::isa::UART_BASE;
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, when, Trace};

// The pieces of the step, each a function of its own, inlined by the
// lowering where the step calls it.

/// Whether an address is one of this device's sixteen bytes.
#[lower]
fn hit(addr: U<32>) -> bool {
    addr.slice::<4, 28>() == UART_BASE >> 4
}

/// The frame a byte goes out as, least significant bit first: a start
/// bit low, the byte, a stop bit high.
#[lower]
fn frame(octet: U<8>) -> U<10> {
    U::<1>::from(1u8)
        .concat::<8, 9>(octet)
        .concat::<1, 10>(U::<1>::from(0u8))
}

/// The frame after a bit has gone out: shifted down, the line's rest
/// state shifted in at the top.
#[lower]
fn shifted(shift: U<10>) -> U<10> {
    U::<1>::from(1u8).concat::<9, 10>(shift.slice::<1, 9>())
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
    line.zext::<1>().concat::<7, 8>(shift.slice::<1, 7>())
}

/// What a load reads, by the word: the last byte sent, the status,
/// the oldest byte received.
#[lower]
fn word(
    sel: U<2>,
    busy: bool,
    ready: bool,
    full: bool,
    last: U<8>,
    data: U<8>,
) -> U<32> {
    let status = U::<29>::from(0u32)
        .concat::<1, 30>(Bit::from(full).zext::<1>())
        .concat::<1, 31>(Bit::from(ready).zext::<1>())
        .concat::<1, 32>(Bit::from(busy).zext::<1>());
    select!(sel.raw() => {
        0 => last.zext::<32>(),
        1 => status,
        2 => data.zext::<32>(),
        _ => U::<32>::from(0u32),
    })
}

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
impl<const DIV: u32>
    Unit<(In<Bit>, In<Bit>, Rx<U<69>>), (Tx<U<32>>, Out<Bit>, Out<Bit>)>
    for Uart<DIV>
{
    async fn run(
        &mut self,
        (rst, rx, req): (In<Bit>, In<Bit>, Rx<U<69>>),
        (resp, tx, irq): (Tx<U<32>>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get().to_bool();
            let rx_ready = self.count != 0;
            let full = self.count == 8;
            let rx_data = self.fifo.read(self.head.get());
            let busy = self.bits != 0;
            // Every request is taken; the ones for this device are the
            // ones whose address falls in its sixteen bytes.
            let offered = req.peek().is_some();
            let r = req.recv().unwrap_or_default();
            let addr = r.slice::<REQ_ADDR, 32>();
            let mine = hit(addr);
            let sel = addr.slice::<2, 2>();
            let we = r.bit(0).to_bool();
            let write = offered & we & mine;
            let read = offered & !we;
            let read_rx = read & mine & (sel == 2);
            // A byte written while idle starts a frame, ten bits of DIV
            // cycles each; a reset wins over everything.
            let start = write & (sel == 0) & !busy;
            let octet = r.slice::<REQ_WDATA, 8>();
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
            when!(read => {
                resp.send(mux(
                    mine,
                    word(sel, busy, rx_ready, full, self.last.get(), rx_data),
                    U::<32>::from(0u32)
                ))
            });
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
            when!(sample & at_start & line => { self.rx_bits <= 0 });
            when!(sample & !at_start & !at_stop => {
                self.rx_shift <= taken_in(self.rx_shift.get(), self.line.get())
            });
            when!(sample & at_stop => { self.rx_bits <= 0 });
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
