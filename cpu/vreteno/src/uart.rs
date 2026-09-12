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
use txhdl::funcs::{eq, is_zero};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, when, Trace};

// The pieces of the step, each a function of its own, inlined by the
// lowering where the step calls it.

/// Whether an address is one of this device's sixteen bytes.
#[lower]
fn hit(addr: U<32>) -> Bit {
    eq(addr.slice::<4, 28>(), U::from(UART_BASE >> 4))
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
fn last_cycle<const DIV: u32>(tick: U<32>) -> Bit {
    eq(tick, U::<32>::from(DIV - 1))
}

/// Whether the current bit's middle cycle has come, where the line is
/// sampled.
#[lower]
fn middle<const DIV: u32>(tick: U<32>) -> Bit {
    eq(tick, U::<32>::from(DIV / 2))
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
    busy: Bit,
    ready: Bit,
    full: Bit,
    last: U<8>,
    data: U<8>,
) -> U<32> {
    let status = U::<29>::from(0u32)
        .concat::<1, 30>(full.zext::<1>())
        .concat::<1, 31>(ready.zext::<1>())
        .concat::<1, 32>(busy.zext::<1>());
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
            let rst = rst.get();
            let (shift, bits, tick) =
                (self.shift.get(), self.bits.get(), self.tick.get());
            let (last, sent) = (self.last.get(), self.sent.get());
            let (line, rx_shift) = (self.line.get(), self.rx_shift.get());
            let (rx_bits, rx_tick) = (self.rx_bits.get(), self.rx_tick.get());
            let (head, count) = (self.head.get(), self.count.get());
            let (received, dropped) = (self.received.get(), self.dropped.get());
            let rx_ready = is_zero(count).not();
            let full = eq(count, U::from(8u8));
            let rx_data = self.fifo.read(head);
            let busy = is_zero(bits).not();
            // Every request is taken; the ones for this device are the
            // ones whose address falls in its sixteen bytes.
            let offered = Bit::from_bool(req.peek().is_some());
            let r = req.recv().unwrap_or_default();
            let addr = r.slice::<REQ_ADDR, 32>();
            let mine = hit(addr);
            let sel = addr.slice::<2, 2>();
            let we = r.bit(0);
            let write = offered.and(we).and(mine);
            let read = offered.and(we.not());
            let read_rx = read.and(mine).and(eq(sel, U::from(2u8)));
            // A byte written while idle starts a frame, ten bits of DIV
            // cycles each.
            let start = write.and(eq(sel, U::from(0u8))).and(busy.not());
            let octet = r.slice::<REQ_WDATA, 8>();
            let done = last_cycle::<DIV>(tick);
            when!(rst => {
                self.bits <= U::from(0u8);
                self.tick <= U::from(0u8)
            });
            when!(rst.not().and(start) => {
                self.shift <= frame(octet);
                self.bits <= U::from(10u8);
                self.tick <= U::from(0u8);
                self.last <= octet;
                self.sent <= sent.wrapping_add(U::from(1u8))
            });
            when!(rst.not().and(busy).and(done.not()) => {
                self.tick <= tick.wrapping_add(U::<32>::from(1u32))
            });
            when!(rst.not().and(busy).and(done) => {
                self.tick <= U::from(0u8);
                self.bits <= bits.wrapping_sub(U::from(1u8));
                self.shift <= shifted(shift)
            });
            when!(read => {
                resp.send(mux(
                    mine,
                    word(sel, busy, rx_ready, full, last, rx_data),
                    U::<32>::from(0u32)
                ))
            });
            tx.set(mux(busy, shift.bit(0), Bit::One));
            // The receive side. A low on the resting line is a start
            // bit; from then on the line is sampled in the middle of
            // each bit, ten of them: a high where the start bit should
            // be is a false start and the frame is dropped, the eight
            // bits in between are taken into the shift register, and a
            // high at the stop bit lands the byte in the buffer, whose
            // oldest the read of the third word takes; the buffer full,
            // the byte is dropped and counted.
            self.line.set(rx.get());
            let receiving = is_zero(rx_bits).not();
            let rx_last = last_cycle::<DIV>(rx_tick);
            let sample = rst.not().and(receiving).and(middle::<DIV>(rx_tick));
            let at_start = eq(rx_bits, U::from(10u8));
            let at_stop = eq(rx_bits, U::from(1u8));
            when!(rst => {
                self.rx_bits <= U::from(0u8);
                self.rx_tick <= U::from(0u8);
                self.head <= U::from(0u8);
                self.count <= U::from(0u8)
            });
            when!(rst.not().and(receiving.not()).and(line.not()) => {
                self.rx_bits <= U::from(10u8);
                self.rx_tick <= U::from(0u8)
            });
            when!(rst.not().and(receiving).and(rx_last.not()) => {
                self.rx_tick <= rx_tick.wrapping_add(U::<32>::from(1u32))
            });
            when!(rst.not().and(receiving).and(rx_last) => {
                self.rx_tick <= U::from(0u8);
                self.rx_bits <= rx_bits.wrapping_sub(U::from(1u8))
            });
            when!(sample.and(at_start).and(line) => {
                self.rx_bits <= U::from(0u8)
            });
            when!(sample.and(at_start.not()).and(at_stop.not()) => {
                self.rx_shift <= taken_in(rx_shift, line)
            });
            when!(sample.and(at_stop) => { self.rx_bits <= U::from(0u8) });
            let landed = sample.and(at_stop).and(line);
            let push = landed.and(full.not());
            let pop = read_rx.and(rx_ready).and(rst.not());
            let tail = head.wrapping_add(count.slice::<0, 3>());
            when!(push => {
                self.fifo.at(tail) <= rx_shift;
                self.received <= received.wrapping_add(U::from(1u8))
            });
            when!(landed.and(full) => {
                self.dropped <= dropped.wrapping_add(U::from(1u8))
            });
            when!(pop => { self.head <= head.wrapping_add(U::from(1u8)) });
            when!(rst.not().and(push).and(pop.not()) => {
                self.count <= count.wrapping_add(U::from(1u8))
            });
            when!(rst.not().and(pop).and(push.not()) => {
                self.count <= count.wrapping_sub(U::from(1u8))
            });
            irq.set(rx_ready);
        }
    }
}
