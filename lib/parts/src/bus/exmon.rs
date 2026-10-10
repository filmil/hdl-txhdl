// SPDX-License-Identifier: Apache-2.0
//! The exclusive monitor: what makes `lr.w`, `sc.w` and the AMOs of
//! two harts atomic against each other and against every other host
//! on the memory (issue 1408).
//!
//! It stands on one peripheral's port, behind the arbiter and the
//! router, where every write to that peripheral passes in the order
//! the peripheral takes it. A read with `lock` set from one of the two
//! ports it watches makes a reservation for that port, of the line of
//! sixteen bytes the read is in. A write that passes clears every
//! reservation whose line its burst covers, whoever made it. A write
//! with `lock` set is the other half: it passes when its port's
//! reservation stands for its line and it is one beat, and its answer
//! goes back as `ExOkay`, which is how the host learns that it stored.
//! Otherwise it fails: the monitor takes its beats, writes nothing,
//! and answers `Okay` itself, so the write never reaches the
//! peripheral or anything between, the data cache's snoop among them.
//!
//! An exclusive write, whether it stores or not, uses its port's
//! reservation up, as `sc.w` does.
//!
//! The monitor keeps exclusives for one range of addresses, the
//! memory's: an address `a` with `a & RM == RB`. An exclusive read
//! outside it makes no reservation, and an exclusive write outside it
//! fails. An exclusive read waits until every write into the range the
//! monitor has passed is answered, so that it reads what they wrote and
//! no write that was in flight when it was taken can land after it;
//! writes elsewhere, which a slow peripheral may hold for long, it does
//! not wait for. The arbiter holds new
//! writes from every other host while an exclusive pair is open, so
//! the wait is for the writes already granted and ends. The failed
//! write's answer waits the same way, so it never overtakes an answer
//! to a write the host sent before it.
//!
//! The monitor takes one write burst at a time, as the arbiter grants
//! them, and passes `lock` on as zero, since the peripheral behind it
//! does not keep exclusives itself. Reads pass as they come otherwise,
//! and their data goes back around it.
use crate::bus::axi::{Ar, Aw, Resp, B, W};
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// The monitor for the two ports `P0` and `P1` of an arbiter whose
/// identifiers are `J` bits, at most five, the host's own `I` of them
/// below the port's, on a link of 32-bit addresses and words, for the
/// addresses `a` with `a & RM == RB`.
#[derive(Trace, Default)]
pub struct ExMon<
    const J: usize,
    const I: usize,
    const P0: usize,
    const P1: usize,
    const RB: usize,
    const RM: usize,
> {
    /// Whether port `P0` has a reservation.
    pub v0: Reg<Bit>,
    /// The line it is for: the address above its four low bits.
    pub line0: Reg<U<28>>,
    /// Whether port `P1` has a reservation.
    pub v1: Reg<Bit>,
    /// The line it is for.
    pub line1: Reg<U<28>>,
    /// The identifiers of the writes into the range passed on and not
    /// yet answered, a bit each: an identifier is one transaction's
    /// until its answer.
    pub wids: Reg<U<32>>,
    /// Set for good when a watched port sent a write under the
    /// identifier of its exclusive write whose answer is still to
    /// come, which would make that answer ambiguous; a test reads it.
    pub xdup: Reg<Bit>,
    /// The exclusive writes that failed, which a test reads to know a
    /// host had to try again.
    pub fails: Reg<U<16>>,
    /// A test's: with it set, every other exclusive write fails as if
    /// its reservation were gone, the first of them included, so a host
    /// tries each pair twice (issue 1474). Nothing in the design sets
    /// it, so it stays zero and synthesis folds it away with `flip`.
    pub alternate: Reg<Bit>,
    /// Whether the last exclusive write taken was one `alternate` failed.
    pub flip: Reg<Bit>,
    /// The last write passed on, for its clears a cycle later: whether
    /// there was one into the range, its first line, and how many lines
    /// it covers.
    pub fwv: Reg<Bit>,
    /// Its first line.
    pub fwline: Reg<U<28>>,
    /// The lines it covers.
    pub fwcnt: Reg<U<28>>,
    /// The exclusive write at the head was decided last cycle: whether
    /// it was, whether it stores, and whether by port `P0`'s reservation.
    pub xk_v: Reg<Bit>,
    /// Whether it stores.
    pub xk_ok: Reg<Bit>,
    /// Whether by port `P0`'s reservation.
    pub xk_p0: Reg<Bit>,
    /// A write burst taken whose beats are still coming.
    pub wpend: Reg<Bit>,
    /// Whether those beats are a failed exclusive write's, taken and
    /// dropped rather than passed on.
    pub wdrop: Reg<Bit>,
    /// A failed exclusive write's answer, waiting to be given.
    pub binj: Reg<Bit>,
    /// Its identifier.
    pub binjid: Reg<U<J>>,
    /// An exclusive write of port `P0` passed on, whose answer goes
    /// back as `ExOkay`.
    pub x0: Reg<Bit>,
    /// Its identifier.
    pub x0id: Reg<U<J>>,
    /// The same for port `P1`.
    pub x1: Reg<Bit>,
    /// Its identifier.
    pub x1id: Reg<U<J>>,
}

#[lower]
impl<
        const J: usize,
        const I: usize,
        const P0: usize,
        const P1: usize,
        const RB: usize,
        const RM: usize,
    > Unit for ExMon<J, I, P0, P1, RB, RM>
{
    async fn run(
        &mut self,
        (aw_in, ar_in, w_in, b_in): (
            Rx<Aw<32, J>>,
            Rx<Ar<32, J>>,
            Rx<W<32, 4>>,
            Rx<B<J>>,
        ),
        (aw_out, ar_out, w_out, b_out): (
            Tx<Aw<32, J>>,
            Tx<Ar<32, J>>,
            Tx<W<32, 4>>,
            Tx<B<J>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let (v0, v1) = (self.v0.get(), self.v1.get());
            let (line0, line1) = (self.line0.get(), self.line1.get());
            let wids = self.wids.get();
            let wpend = self.wpend.get();
            let binj = self.binj.get();
            let quiet = Bit::from(wids == 0);
            let (rb, rm) = (U::<32>::from(RB as u32), U::<32>::from(RM as u32));
            // A read passes, an exclusive one once every write passed
            // on is answered, and its port takes the reservation.
            let ar = ar_in.head();
            let ar_here = Bit::from(ar_in.peek().is_some());
            let ar_port = ar.id >> I;
            let ar_in_range = Bit::from((ar.addr & rm) == rb);
            let ar_x = ar.lock & ar_in_range;
            let ar_go = ar_here & (!ar_x | quiet) & ar_out.ready();
            let _ = ar_in.recv_if(ar_go);
            if ar_go.to_bool() {
                ar_out.send(Ar {
                    id: ar.id,
                    addr: ar.addr,
                    len: ar.len,
                    size: ar.size,
                    burst: ar.burst,
                    lock: Bit::Zero,
                    cache: ar.cache,
                    prot: ar.prot,
                    qos: ar.qos,
                    region: ar.region,
                });
            }
            let ar_line = ar.addr.slice::<4, 28>();
            let set0 = ar_go & ar_x & Bit::from(ar_port == P0);
            let set1 = ar_go & ar_x & Bit::from(ar_port == P1);
            // A write: one burst at a time, and none while a failed
            // one's answer waits. An exclusive one passes when its
            // port's reservation stands for its line and it is one
            // beat; otherwise it is taken and dropped.
            let aw = aw_in.head();
            let aw_here = Bit::from(aw_in.peek().is_some());
            let aw_port = aw.id >> I;
            let aw_line = aw.addr.slice::<4, 28>();
            // The clears of the last write passed on, from its line and
            // span in registers, a cycle after it: the compares stay off
            // the way to the router.
            let fwv = self.fwv.get();
            let (fwline, fwcnt) = (self.fwline.get(), self.fwcnt.get());
            let hit0 = fwv & Bit::from((line0 - fwline) < fwcnt);
            let hit1 = fwv & Bit::from((line1 - fwline) < fwcnt);
            let ok0 = Bit::from(aw_port == P0)
                & v0
                & !hit0
                & Bit::from(line0 == aw_line);
            let ok1 = Bit::from(aw_port == P1)
                & v1
                & !hit1
                & Bit::from(line1 == aw_line);
            let aw_in_range = Bit::from((aw.addr & rm) == rb);
            let ex_ok = (ok0 | ok1) & Bit::from(aw.len == 0) & aw_in_range;
            // An exclusive write waits a cycle at the head, where it is
            // decided into a register, and goes on that the next: only
            // registers stand before the router. Nothing else can pass
            // while it waits, so the decision holds.
            let fail = aw.lock
                & (!self.xk_ok.get()
                    | (self.alternate.get() & !self.flip.get()));
            let aw_take =
                aw_here & !wpend & !binj & (!aw.lock | self.xk_v.get());
            let aw_fwd = aw_take & !fail & aw_out.ready();
            let aw_abs = aw_take & fail;
            let _ = aw_in.recv_if(aw_fwd | aw_abs);
            if aw_fwd.to_bool() {
                aw_out.send(Aw {
                    id: aw.id,
                    addr: aw.addr,
                    len: aw.len,
                    size: aw.size,
                    burst: aw.burst,
                    lock: Bit::Zero,
                    cache: aw.cache,
                    prot: aw.prot,
                    qos: aw.qos,
                    region: aw.region,
                });
            }
            // The lines a write passed on covers, from its first: a
            // reservation in them is cleared, the cycle after.
            let span = aw.addr.slice::<2, 2>().zext::<9>() + aw.len.zext::<9>();
            let cnt = span.slice::<2, 7>().zext::<28>() + 1;
            let xok0 = aw_fwd & aw.lock & self.xk_p0.get();
            let xok1 = aw_fwd & aw.lock & !self.xk_p0.get();
            // The beats: passed on, or a failed write's taken and
            // dropped; its answer waits from its last beat.
            let wh = w_in.head();
            let w_here = Bit::from(w_in.peek().is_some());
            let wdrop = self.wdrop.get();
            let w_fwd = wpend & !wdrop & w_here & w_out.ready();
            let w_dropped = wpend & wdrop & w_here;
            let _ = w_in.recv_if(w_fwd | w_dropped);
            if w_fwd.to_bool() {
                w_out.send(W {
                    data: wh.data,
                    strb: wh.strb,
                    last: wh.last,
                });
            }
            let w_end = (w_fwd | w_dropped) & wh.last;
            // The answers: a failed write's, once every write passed on
            // is answered, so it overtakes none; else the peripheral's,
            // an exclusive write's as `ExOkay`.
            let inj = binj & quiet & b_out.ready();
            let bh = b_in.head();
            let b_here = Bit::from(b_in.peek().is_some());
            let b_go = b_here & !inj & b_out.ready();
            let _ = b_in.recv_if(b_go);
            let is0 = self.x0.get() & Bit::from(bh.id == self.x0id.get());
            let is1 = self.x1.get() & Bit::from(bh.id == self.x1id.get());
            let exo = (is0 | is1) & Bit::from(bh.resp == Resp::Okay);
            if (inj | b_go).to_bool() {
                b_out.send(B {
                    id: mux(inj, self.binjid.get(), bh.id),
                    resp: mux(inj, Resp::Okay, mux(exo, Resp::ExOkay, bh.resp)),
                });
            }
            // The writes into the range out, by identifier.
            let one = U::<32>::from(1u32);
            let zero = U::<32>::from(0u32);
            let w_set =
                mux(aw_fwd & aw_in_range, one << (aw.id.raw() as usize), zero);
            let w_clr = mux(b_go, one << (bh.id.raw() as usize), zero);
            // An exclusive write uses its port's reservation up, whether
            // it stored or not.
            let used0 = aw_abs & Bit::from(aw_port == P0);
            let used1 = aw_abs & Bit::from(aw_port == P1);
            // A watched port's write under the identifier its exclusive
            // write's answer will come back with.
            let dup = aw_fwd
                & ((self.x0.get() & Bit::from(aw.id == self.x0id.get()))
                    | (self.x1.get() & Bit::from(aw.id == self.x1id.get())));
            with!(self <= {
                wids: (wids | w_set) & !w_clr,
                dup ? xdup: Bit::One,
                aw_abs ? fails: self.fails.get() + 1,
                (aw_fwd | aw_abs) & aw.lock ? flip: !self.flip.get(),
                aw_fwd | aw_abs ? { wpend: Bit::One, wdrop: aw_abs },
                aw_abs ? binjid: aw.id,
                w_end ? wpend: Bit::Zero,
                (w_end & wdrop) ? binj: Bit::One,
                inj ? binj: Bit::Zero,
                fwv: aw_fwd & aw_in_range,
                fwline: aw_line,
                fwcnt: cnt,
                xk_v: aw_here & aw.lock & !(aw_fwd | aw_abs),
                xk_ok: ex_ok,
                xk_p0: ok0,
                // A write's clear comes the cycle after it, and wins
                // over a set made in the write's own cycle, since the
                // two reach the peripheral in no known order; a read in
                // the cycle after waits for the write's answer.
                v0: (v0 | set0) & !hit0 & !used0,
                v1: (v1 | set1) & !hit1 & !used1,
                set0 ? line0: ar_line,
                set1 ? line1: ar_line,
                xok0 ? { x0: Bit::One, x0id: aw.id },
                xok1 ? { x1: Bit::One, x1id: aw.id },
                (b_go & is0) ? x0: Bit::Zero,
                (b_go & is1) ? x1: Bit::Zero,
            });
        }
    }
}

/// The monitor between an arbiter of two hosts and a memory: a pair
/// that keeps, a pair another host's write breaks, a read that waits
/// for a write in flight, and a failed write's answer in its place.
#[cfg(test)]
mod tests {
    use super::ExMon;
    use crate::bus::arbiter::Arbiter;
    use crate::bus::axi::{axi, AxiHost, Host, Rd, Resp, Wr};
    use crate::bus::axi_per_pins::sim::pins;
    use crate::bus::axi_per_pins::AxiPerPins;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, now, Clock, DefaultClock, Reg, Running, Unit};
    use txhdl::types::{Bit, U};

    type Client = Host<32, 32, 4, 2, 8>;

    /// The memory's words after a run, by word index.
    struct Mem(Rc<RefCell<std::collections::HashMap<u128, U<32>>>>);

    impl Mem {
        fn word(&self, i: u128) -> U<32> {
            self.0.borrow().get(&i).copied().unwrap_or_default()
        }
    }
    type Boxed = Box<dyn std::future::Future<Output = ()> + Unpin>;

    /// `n` cycles of the clock, in a client.
    async fn wait(n: usize) {
        for _ in 0..n {
            DefaultClock::rising().await;
        }
    }

    /// An exclusive read of one word.
    fn xrd(at: u32) -> Rd<32> {
        Rd {
            lock: Bit::One,
            ..Rd::at(at, 1)
        }
    }

    /// An exclusive write.
    fn xwr(at: u32) -> Wr<32> {
        Wr {
            lock: Bit::One,
            ..Wr::at(at)
        }
    }

    std::thread_local! {
        // Whether this thread's rigs have the monitor fail every other
        // exclusive write (issue 1474).
        static ALTERNATE: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
    }

    /// With `alternate` set, the first exclusive write fails though its
    /// reservation holds, answered OKAY with nothing stored, and the same
    /// pair again stores, answered EXOKAY (issue 1474).
    #[test]
    fn alternate_fails_every_other_exclusive_write() {
        let got = Rc::new(RefCell::new((Resp::SlvErr, Resp::SlvErr)));
        let g = got.clone();
        ALTERNATE.with(|a| a.set(true));
        let ram = rig::<1>(
            move |host| {
                Box::new(Box::pin(async move {
                    let seven = [U::from(7u32)];
                    host.read(xrd(0x100)).await.done().await;
                    let a = host.write(xwr(0x100), &seven).await.done().await;
                    host.read(xrd(0x100)).await.done().await;
                    let b = host.write(xwr(0x100), &seven).await.done().await;
                    *g.borrow_mut() = (a.resp, b.resp);
                }))
            },
            |_| Box::new(Box::pin(async {})),
            300,
        );
        ALTERNATE.with(|a| a.set(false));
        assert_eq!(
            *got.borrow(),
            (Resp::Okay, Resp::ExOkay),
            "the first failed, the second stored"
        );
        assert_eq!(ram.word(0x100 / 4).raw(), 7, "stored by the second");
    }

    /// Two hosts on ports 0 and 1, the arbiter with the hold built or
    /// not, the monitor watching both, and a memory of 1024 words that
    /// answers a read 8 cycles after its address and takes its words
    /// then, as a controller does, so a read passed on while a write's
    /// beats are still coming misses them; the memory is returned to
    /// read after `n` cycles.
    fn rig<const HOLD: usize>(
        a: impl FnOnce(Client) -> Boxed,
        b: impl FnOnce(Client) -> Boxed,
        n: usize,
    ) -> Mem {
        let l0 = axi::<32, 32, 4, 2, 8>();
        let l1 = axi::<32, 32, 4, 2, 8>();
        let la = axi::<32, 32, 4, 5, 8>();
        let lp = axi::<32, 32, 4, 5, 8>();
        let mut h0 = AxiHost::<32, 32, 4, 2, 8>::default();
        let mut h1 = AxiHost::<32, 32, 4, 2, 8>::default();
        let mut pinned = AxiPerPins::<32, 32, 4, 5>::default();
        let (paw, par, pw, _, _) = lp.per_in;
        let (_, _, pb, pr) = lp.per_out;
        let (ram, inp, outp) = pins::<32, 32, 4, 5>(paw, par, pw, pb, pr, 1024);
        let ram = ram.reading_at_address().timed(8, 2, 0);
        let words = ram.memory();
        let mut arb = Arbiter::<2, 32, 32, 4, 2, 5, 0, HOLD>::default();
        let mut mon = ExMon::<5, 2, 0, 1, 0, 0>::default();
        if ALTERNATE.with(|a| a.get()) {
            mon.alternate = Reg::new(Bit::One);
        }
        let xdup = mon.xdup;
        // The arbiter's link goes into the monitor, whose outputs are a
        // link of their own to the memory's unit; the read data goes
        // straight back.
        let hardware = join2(
            join2(
                h0.run(l0.host_in, l0.host_out),
                h1.run(l1.host_in, l1.host_out),
            ),
            join2(
                arb.run(
                    (
                        [l0.per_in.0, l1.per_in.0],
                        [l0.per_in.1, l1.per_in.1],
                        [l0.per_in.2, l1.per_in.2],
                        la.host_in.2,
                        lp.host_in.3,
                    ),
                    (
                        la.host_out.0,
                        la.host_out.1,
                        la.host_out.2,
                        [l0.per_out.2, l1.per_out.2],
                        [l0.per_out.3, l1.per_out.3],
                    ),
                ),
                join2(
                    mon.run(
                        (la.per_in.0, la.per_in.1, la.per_in.2, lp.host_in.2),
                        (
                            lp.host_out.0,
                            lp.host_out.1,
                            lp.host_out.2,
                            la.per_out.2,
                        ),
                    ),
                    pinned.run(inp, outp),
                ),
            ),
        );
        let clients = join2(a(l0.host), b(l1.host));
        let mut sim =
            Running::new(join2(join2(hardware, ram.serve()), clients));
        for _ in 0..n {
            sim.cycle();
        }
        assert!(
            !xdup.get().to_bool(),
            "a watched port reused its exclusive write's identifier"
        );
        Mem(words)
    }

    /// An exclusive write uses its port's reservation up whether it
    /// stores or not: after `lr` of one word, a failed `sc` of another,
    /// then `sc` of the first fails too, as Linux's `sc` on the way out of
    /// a trap relies on.
    #[test]
    fn a_failed_exclusive_write_uses_the_reservation_up() {
        let got = Rc::new(RefCell::new((Resp::SlvErr, Resp::SlvErr)));
        let g = got.clone();
        let ram = rig::<1>(
            move |host| {
                Box::new(Box::pin(async move {
                    host.read(xrd(0x100)).await.done().await;
                    let seven = [U::from(7u32)];
                    let b = host.write(xwr(0x200), &seven).await.done().await;
                    let a = host.write(xwr(0x100), &seven).await.done().await;
                    *g.borrow_mut() = (b.resp, a.resp);
                }))
            },
            |_| Box::new(Box::pin(async {})),
            200,
        );
        assert_eq!(*got.borrow(), (Resp::Okay, Resp::Okay), "both failed");
        assert_eq!(ram.word(0x100 / 4).raw(), 0, "nothing stored at A");
        assert_eq!(ram.word(0x200 / 4).raw(), 0, "nor at B");
    }

    /// The reply to one write and the cycle it came in, from `t0`,
    /// put in `into`.
    async fn timed(
        w: impl std::future::Future<Output = crate::bus::axi::Reply<32>>,
        t0: u64,
        into: Rc<RefCell<(Resp, u64)>>,
    ) {
        let r = w.await;
        *into.borrow_mut() = (r.resp, (now() - t0) / DefaultClock::PERIOD);
    }

    /// A pair with nothing between: the write answers `ExOkay` and the
    /// word is written.
    #[test]
    fn a_pair_with_nothing_between_stores() {
        let got = Rc::new(RefCell::new(Resp::SlvErr));
        let g = got.clone();
        let ram = rig::<1>(
            move |host| {
                Box::new(Box::pin(async move {
                    host.read(xrd(0x100)).await.done().await;
                    let seven = [U::from(7u32)];
                    let r = host.write(xwr(0x100), &seven).await.done().await;
                    *g.borrow_mut() = r.resp;
                }))
            },
            |_| Box::new(Box::pin(async {})),
            200,
        );
        assert_eq!(*got.borrow(), Resp::ExOkay, "the pair kept");
        assert_eq!(ram.word(0x100 / 4).raw(), 7, "and the word is stored");
    }

    /// Another host's write to the line between the two halves breaks
    /// the pair, here with the arbiter's hold not built so that the
    /// write can come between: the exclusive write answers `Okay` and
    /// writes nothing, and the other host's word stands.
    #[test]
    fn a_write_between_breaks_the_pair() {
        let got = Rc::new(RefCell::new(Resp::SlvErr));
        let g = got.clone();
        let ram = rig::<0>(
            move |host| {
                Box::new(Box::pin(async move {
                    host.read(xrd(0x100)).await.done().await;
                    wait(30).await;
                    let seven = [U::from(7u32)];
                    let r = host.write(xwr(0x100), &seven).await.done().await;
                    *g.borrow_mut() = r.resp;
                }))
            },
            |host| {
                Box::new(Box::pin(async move {
                    wait(10).await;
                    let nine = [U::from(9u32)];
                    host.write(Wr::at(0x104u32), &nine).await.done().await;
                }))
            },
            300,
        );
        assert_eq!(*got.borrow(), Resp::Okay, "the pair broke");
        assert_eq!(ram.word(0x100 / 4).raw(), 0, "nothing stored");
        assert_eq!(ram.word(0x104 / 4).raw(), 9, "the other write stands");
    }

    /// The other host's burst of 32 over the line is granted first and its
    /// beats still going when the exclusive read comes: the read waits
    /// for the burst's answer, reads what the burst wrote, and the pair
    /// keeps, with the hold keeping the other host's next write out.
    #[test]
    fn an_exclusive_read_waits_for_a_write_in_flight() {
        let got = Rc::new(RefCell::new((0u32, Resp::SlvErr)));
        let g = got.clone();
        let ram = rig::<1>(
            move |host| {
                Box::new(Box::pin(async move {
                    wait(3).await;
                    let r = host.read(xrd(0x17c)).await.done().await;
                    let old = r.data[0].raw() as u32;
                    let new = [U::from(old + 1)];
                    let w = host.write(xwr(0x17c), &new).await.done().await;
                    *g.borrow_mut() = (old, w.resp);
                }))
            },
            |host| {
                Box::new(Box::pin(async move {
                    let burst: Vec<U<32>> =
                        (0..32u32).map(|i| U::from(0x50 + i)).collect();
                    let first = host.write(Wr::at(0x100u32), &burst).await;
                    let one = [U::from(1u32)];
                    let second = host.write(Wr::at(0x200u32), &one).await;
                    first.done().await;
                    second.done().await;
                }))
            },
            400,
        );
        let (old, resp) = *got.borrow();
        assert_eq!(old, 0x6f, "the read saw the burst's word");
        assert_eq!(resp, Resp::ExOkay, "and the pair kept");
        assert_eq!(ram.word(0x17c / 4).raw(), 0x70, "the word went up by one");
    }

    /// A failed exclusive write's answer, which the monitor gives
    /// itself, never comes before the answer to a write its host sent
    /// before it: the earlier write is answered first.
    #[test]
    fn a_failed_writes_answer_overtakes_nothing() {
        let plain_at = Rc::new(RefCell::new((Resp::SlvErr, 0)));
        let ex_at = Rc::new(RefCell::new((Resp::SlvErr, 0)));
        let (pa, xa) = (plain_at.clone(), ex_at.clone());
        rig::<1>(
            move |host| {
                Box::new(Box::pin(async move {
                    let t0 = now();
                    let burst: Vec<U<32>> = (0..8u32).map(U::from).collect();
                    let plain = host.write(Wr::at(0x200u32), &burst).await;
                    let seven = [U::from(7u32)];
                    let ex = host.write(xwr(0x300), &seven).await;
                    join2(
                        timed(plain.done(), t0, pa),
                        timed(ex.done(), t0, xa),
                    )
                    .await;
                }))
            },
            |_| Box::new(Box::pin(async {})),
            300,
        );
        let ((p, pt), (x, xt)) = (*plain_at.borrow(), *ex_at.borrow());
        assert_eq!(p, Resp::Okay, "the plain write stored");
        assert_eq!(x, Resp::Okay, "the exclusive one, with no read, failed");
        assert!(pt <= xt, "the earlier answer first: {pt} then {xt}");
    }
}
