// SPDX-License-Identifier: Apache-2.0
//! The serial port, transmit side: a device on the bus that takes a
//! byte written to its first word and shifts it out on a line, a start
//! bit, eight data bits least significant first and a stop bit, each
//! `DIV` cycles long; its second word is the status, bit 0 busy while
//! a byte is going out. A byte written while busy is dropped, so a
//! program polls the status first. The line rests high. `DIV` is 868
//! for 115200 baud at 100 MHz, and 4 in the runs that are checked, so
//! that a byte takes forty cycles rather than nine thousand.
use crate::bus::{REQ_ADDR, REQ_WDATA};
use crate::isa::UART_BASE;
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::funcs::{eq, is_zero};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// The pieces of the step, each a function of its own, inlined by the
// lowering where the step calls it.

/// Whether an address is one of this device's eight bytes.
#[lower]
fn hit(addr: U<32>) -> Bit {
    eq(addr.slice::<3, 29>(), U::from(UART_BASE >> 3))
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

/// What a load reads: the status at the second word, the last byte
/// at the first.
#[lower]
fn word(sel: Bit, busy: Bit, last: U<8>) -> U<32> {
    mux(sel, busy.zext::<32>(), last.zext::<32>())
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
}

#[lower]
impl<const DIV: u32> Unit<(In<Bit>, Rx<U<69>>), (Tx<U<32>>, Out<Bit>)>
    for Uart<DIV>
{
    async fn run(
        &mut self,
        (rst, req): (In<Bit>, Rx<U<69>>),
        (resp, tx): (Tx<U<32>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get();
            let (shift, bits, tick) =
                (self.shift.get(), self.bits.get(), self.tick.get());
            let (last, sent) = (self.last.get(), self.sent.get());
            let busy = is_zero(bits).not();
            // Every request is taken; the ones for this device are the
            // ones whose address falls in its eight bytes.
            let offered = Bit::from_bool(req.peek().is_some());
            let r = req.recv().unwrap_or_default();
            let addr = r.slice::<REQ_ADDR, 32>();
            let mine = hit(addr);
            let sel = addr.bit(2);
            let we = r.bit(0);
            let write = offered.and(we).and(mine);
            let read = offered.and(we.not());
            // A byte written while idle starts a frame, ten bits of DIV
            // cycles each.
            let start = write.and(sel.not()).and(busy.not());
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
                resp.send(mux(mine, word(sel, busy, last), U::<32>::from(0u32)))
            });
            tx.set(mux(busy, shift.bit(0), Bit::One));
        }
    }
}
