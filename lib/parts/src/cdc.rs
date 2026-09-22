// SPDX-License-Identifier: Apache-2.0
//! A channel from one clock to another, in the lowered subset.
//!
//! Two processes, one per clock, sharing a memory and four registers.
//! The writing side owns `wbin` and `wgray`, the reading side owns
//! `rbin` and `rgray`, and each side keeps the other's Gray pointer
//! through two flip-flops of its own clock. That pair of flops is the
//! crossing: everything else on each side reads only its own domain.
//!
//! The pointers are Gray coded because a binary pointer changes
//! several bits at once and the other side samples it at an unrelated
//! moment. `7 -> 8` is `0111 -> 1000`, four bits at once, and a sample
//! caught between them can read any of sixteen values, including ones
//! the pointer never held. A Gray code changes one bit per step, so a
//! sample caught mid-change reads either the old value or the new one,
//! and both are safe: the reader concludes the queue is emptier than
//! it is, the writer that it is fuller, and each is wrong in the
//! direction that refuses work rather than corrupting it.
//!
//! The pointers are one bit wider than the address so that full and
//! empty can be told apart. Empty is the two pointers equal. Full is
//! the two pointers equal in every bit but the top two, which in Gray
//! is what one lap apart means.
//!
//! What this cannot say: nothing here models metastability. Both
//! sides are deterministic, the sampling clock reads a register that
//! has settled, and the two flops are never seen to do anything. A
//! passing co-simulation says the two implementations agree on the
//! protocol, cycle by cycle, and says nothing at all about whether
//! the silicon is safe. What the second flop is for is exactly the
//! thing the test cannot see.
//!
//! `lib/board/chan_cdc.v` is the hand-written Verilog this is shaped
//! after, five instances of which are in the flagship. Replacing it
//! is a separate decision (issue 370): that file is proven on
//! hardware and this one is not.
use txhdl::comp::{join2, Clock, Mem, Reg, Rx, Tx, Unit};
use txhdl::types::{Transaction, Value, U};
use txhdl::{lower, with, Trace};

/// A channel of `D` words of `T` from clock `W` to clock `R`.
///
/// `AW` is the address width, `D` is `1 << AW`, and `PW` is `AW + 1`,
/// the width of the pointers. All three are stated because an
/// expression in a const parameter's position needs nightly Rust,
/// which is the same reason [`crate::fifo::Fifo`] takes both `AW` and
/// `D`.
// begin{state}
#[derive(Trace)]
pub struct ChanCdc<
    T: Transaction + Value,
    const AW: usize,
    const D: usize,
    const PW: usize,
    W: Clock,
    R: Clock,
> {
    /// The words. Written on `W`, read on `R`, as a dual-port memory.
    pub mem: Mem<T, D, W>,
    /// Where the next word lands, in binary, on the writing clock.
    pub wbin: Reg<U<PW>, W>,
    /// The same pointer Gray coded, which is what the other side
    /// samples.
    pub wgray: Reg<U<PW>, W>,
    /// The reading side's Gray pointer, caught once on the writing
    /// clock. Possibly mid-change, which is what the second flop is
    /// for.
    pub rgray_w1: Reg<U<PW>, W>,
    /// The same, a cycle later and settled: the one the writing side
    /// reads.
    pub rgray_w2: Reg<U<PW>, W>,
    /// Where the oldest word is, in binary, on the reading clock.
    pub rbin: Reg<U<PW>, R>,
    /// The same pointer Gray coded.
    pub rgray: Reg<U<PW>, R>,
    /// The writing side's Gray pointer, caught once on the reading
    /// clock.
    pub wgray_r1: Reg<U<PW>, R>,
    /// The same, settled: the one the reading side reads.
    pub wgray_r2: Reg<U<PW>, R>,
}
// end{state}

/// Written out rather than derived, because a derived `Default` would
/// ask the two clocks to be `Default` as well, and a clock is a type
/// with nothing in it.
impl<
        T: Transaction + Value,
        const AW: usize,
        const D: usize,
        const PW: usize,
        W: Clock,
        R: Clock,
    > Default for ChanCdc<T, AW, D, PW, W, R>
{
    fn default() -> Self {
        ChanCdc {
            mem: Mem::default(),
            wbin: Reg::default(),
            wgray: Reg::default(),
            rgray_w1: Reg::default(),
            rgray_w2: Reg::default(),
            rbin: Reg::default(),
            rgray: Reg::default(),
            wgray_r1: Reg::default(),
            wgray_r2: Reg::default(),
        }
    }
}

#[lower]
impl<
        T: Transaction + Value,
        const AW: usize,
        const D: usize,
        const PW: usize,
        W: Clock,
        R: Clock,
    > Unit<Rx<T, W>, Tx<T, R>> for ChanCdc<T, AW, D, PW, W, R>
{
    async fn run(&mut self, inp: Rx<T, W>, out: Tx<T, R>) {
        join2(
            async {
                loop {
                    W::rising().await;
                    let (wbin, wgray) = (self.wbin.get(), self.wgray.get());
                    let rgray = self.rgray_w2.get();
                    // Full: every bit but the top two equal, and both
                    // of those differing. Shifting two out is how the
                    // low bits are compared without naming a width,
                    // which would want `PW - 2` in a const position.
                    let low = (wgray << 2u32) == (rgray << 2u32);
                    let top = wgray.bit(PW - 1) ^ rgray.bit(PW - 1);
                    let nxt = wgray.bit(PW - 2) ^ rgray.bit(PW - 2);
                    let full = low & top & nxt;
                    let push = inp.peek().is_some() & !full;
                    let word = inp.recv_if(!full).unwrap_or_default();
                    let wbin1 = wbin + 1;
                    with!(self <= {
                        push ? mem.at(wbin.slice::<0, AW>()): word,
                        push ? wbin: wbin1,
                        push ? wgray: wbin1 ^ (wbin1 >> 1u32),
                        // The other side's pointer, every cycle and
                        // whether or not anything moved: a
                        // synchroniser that only ran on a push would
                        // stop telling the writer that room had
                        // appeared.
                        rgray_w1: self.rgray.get(),
                        rgray_w2: self.rgray_w1.get(),
                    });
                }
            },
            async {
                loop {
                    R::rising().await;
                    let (rbin, rgray) = (self.rbin.get(), self.rgray.get());
                    let empty = rgray == self.wgray_r2.get();
                    let pop = !empty & out.ready();
                    let rbin1 = rbin + 1;
                    with!(self <= {
                        pop ? rbin: rbin1,
                        pop ? rgray: rbin1 ^ (rbin1 >> 1u32),
                        wgray_r1: self.wgray.get(),
                        wgray_r2: self.wgray_r1.get(),
                    });
                    if pop.to_bool() {
                        out.send(self.mem.read(rbin.slice::<0, AW>()));
                    }
                }
            },
        )
        .await;
    }
}
