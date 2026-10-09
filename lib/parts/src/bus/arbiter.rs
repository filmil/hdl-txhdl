// SPDX-License-Identifier: Apache-2.0
//! The AXI arbiter: several hosts, one peripheral link. It is the
//! mirror of the [`router`](super::router), which is one host and
//! several peripherals, and the two go together: an arbiter merges
//! the hosts onto one link and a router fans that link out again.
//!
//! Until there was one, the core was the only thing on this bus that
//! could be a host. Everything else, an Ethernet MAC with a frame to
//! store or a rasteriser with a scene to draw, had to be given its
//! bytes a word at a time by the core through a register.
//!
//! The problem an arbiter has that a router does not is that the
//! answers must find their way home. AXI4 puts an identifier on each
//! address phase and carries it back on the answer, so the arbiter
//! puts the host's port number above the host's own identifier on the
//! way out and takes it off again on the way back. That is why the
//! peripheral side's identifier is `J` bits where a host's is `I`:
//! `J` has to be at least `I` plus the bits the port number needs.
//!
//! The write data channel has no identifier at all, which is the
//! other half of the same problem. A beat belongs to the oldest
//! address phase that has not finished, so the arbiter locks that
//! channel to whichever host won the write address phase until its
//! last beat. Reads need no lock, since each carries its identifier
//! and may be answered out of order.
//!
//! One unit serves every count of hosts: its hosts are arrays of
//! ports, and the lowering unrolls the loops over them when `lowered`
//! runs (issue 500). `Arbiter2` to `Arbiter8` name the counts the
//! tree has always used.
use crate::bus::axi::{Ar, Aw, B, R, W};
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{part}
/// An AXI4 arbiter of `N` hosts and one peripheral link, the mirror of
/// the router: it merges rather than fans out.
///
/// Each host's address phases go out carrying its port number above
/// its own identifier, so the peripheral side's identifier is `J` bits
/// where a host's is `I`; the answers come back with that tag and are
/// given to the host it names, with the tag taken off again. `J` must
/// be at least `I` plus the bits the port number needs.
///
/// The write data channel is locked to the host whose write address
/// phase was granted until that burst's last beat, since AXI4 puts no
/// identifier on `w` and a beat belongs to the oldest address phase
/// that has not finished. Reads need no such lock: each carries its
/// identifier.
///
/// Arbitration is round robin, the turn moving past the host that
/// won. With `FIXED` not zero it is fixed priority instead, and the
/// lowest numbered host that is offering always wins. At most eight
/// hosts, since the turn is three bits.
///
/// With `HOLD` not zero the arbiter keeps an exclusive pair whole
/// (issue 1408). When it grants a read with `lock` set, it takes the
/// hold for that host: no other host's write address phase is granted
/// until it grants that host's write with `lock` set, or 128 cycles
/// pass. A write already granted keeps its beats, so the writes in
/// flight drain while the hold stands. Reads are never held. A host
/// that takes an exclusive read while another holds goes without the
/// hold; the same host's next exclusive read takes it again. With
/// `HOLD` zero, as everywhere but where two harts share memory, the
/// hold never starts and synthesis folds its registers away.
///
/// `PRIO` names a host whose read address phase wins whenever it offers
/// one, ahead of the round robin, which goes on as before among the
/// others (issue 1523): the flagship's scanout, whose line has a
/// deadline the others' reads do not. Eight, the default, names none.
/// Writes are not affected.
#[derive(Trace, Default)]
pub struct Arbiter<
    const N: usize,
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
    const HOLD: usize = 0,
    const PRIO: usize = 8,
> {
    /// A write burst's beats are going out; another host's address
    /// phase waits, since AXI4 puts no identifier on `w`.
    pub wbusy: Reg<Bit>,
    /// Whose beats they are, one bit per host.
    pub wsel: Reg<U<N>>,
    /// Where the round robin starts looking for the next read address
    /// phase.
    pub rturn: Reg<U<3>>,
    /// The same, for the next write address phase.
    pub wturn: Reg<U<3>>,
    /// Whether the exclusive hold stands.
    pub hold: Reg<Bit>,
    /// The host the hold is for.
    pub howner: Reg<U<3>>,
    /// The cycles the hold has left.
    pub hleft: Reg<U<7>>,
    /// How many holds ran out rather than ending at their host's
    /// exclusive write, which a test reads.
    pub htimeouts: Reg<U<16>>,
}

/// Two hosts.
pub type Arbiter2<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<2, A, D, S, I, J, FIXED>;
/// Three hosts.
pub type Arbiter3<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<3, A, D, S, I, J, FIXED>;
/// Four hosts.
pub type Arbiter4<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<4, A, D, S, I, J, FIXED>;
/// Five hosts.
pub type Arbiter5<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<5, A, D, S, I, J, FIXED>;
/// Six hosts.
pub type Arbiter6<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<6, A, D, S, I, J, FIXED>;
/// Seven hosts.
pub type Arbiter7<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<7, A, D, S, I, J, FIXED>;
/// Eight hosts.
pub type Arbiter8<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
    const J: usize,
    const FIXED: usize,
> = Arbiter<8, A, D, S, I, J, FIXED>;

// The lowering reads a loop over an array of ports as `aws[i]`, so the
// index is what it is written with, and Clippy would rather it were an
// iterator.
#[allow(clippy::needless_range_loop)]
#[lower]
impl<
        const N: usize,
        const A: usize,
        const D: usize,
        const S: usize,
        const I: usize,
        const J: usize,
        const FIXED: usize,
        const HOLD: usize,
        const PRIO: usize,
    > Unit for Arbiter<N, A, D, S, I, J, FIXED, HOLD, PRIO>
{
    async fn run(
        &mut self,
        (aws, ars, ws, b, r): (
            [Rx<Aw<A, I>>; N],
            [Rx<Ar<A, I>>; N],
            [Rx<W<D, S>>; N],
            Rx<B<J>>,
            Rx<R<D, J>>,
        ),
        (aw, ar, w, bs, rs): (
            Tx<Aw<A, J>>,
            Tx<Ar<A, J>>,
            Tx<W<D, S>>,
            [Tx<B<I>>; N],
            [Tx<R<D, I>>; N],
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let fixed = FIXED != 0;
            let rturn = self.rturn.get();
            let wturn = self.wturn.get();
            let wbusy = self.wbusy.get();
            let wsel = self.wsel.get();
            // The exclusive hold, only where `HOLD` builds it.
            let held = self.hold.get() & Bit::from(HOLD != 0);
            let howner = self.howner.get();
            // Round robin: a host is eligible when it is at or after
            // the turn, and the grant is the first eligible one that
            // offers, or the first of any when none is eligible. Under
            // fixed priority every host is eligible always, so the
            // lowest offering one always wins.
            let mut ar_hi = Bit::Zero;
            let mut ar_hi_who = U::<3>::from(0u8);
            let mut ar_lo = Bit::Zero;
            let mut ar_lo_who = U::<3>::from(0u8);
            let mut aw_hi = Bit::Zero;
            let mut aw_hi_who = U::<3>::from(0u8);
            let mut aw_lo = Bit::Zero;
            let mut aw_lo_who = U::<3>::from(0u8);
            // The host `PRIO` names, offering a read.
            let mut ar_prio = Bit::Zero;
            for i in 0..N {
                let ar_off = Bit::from(ars[i].peek().is_some());
                ar_prio = ar_prio | (ar_off & Bit::from(i == PRIO));
                let ar_first = ar_off & !ar_hi & (fixed | (rturn <= i));
                ar_hi_who = mux(ar_first, U::<3>::from(i), ar_hi_who);
                ar_hi = ar_hi | ar_first;
                let ar_any = ar_off & !ar_lo;
                ar_lo_who = mux(ar_any, U::<3>::from(i), ar_lo_who);
                ar_lo = ar_lo | ar_any;
                // A host's write waits while another holds.
                let aw_off = Bit::from(aws[i].peek().is_some())
                    & !(held & (howner != i));
                let aw_first = aw_off & !aw_hi & (fixed | (wturn <= i));
                aw_hi_who = mux(aw_first, U::<3>::from(i), aw_hi_who);
                aw_hi = aw_hi | aw_first;
                let aw_any = aw_off & !aw_lo;
                aw_lo_who = mux(aw_any, U::<3>::from(i), aw_lo_who);
                aw_lo = aw_lo | aw_any;
            }
            let ar_who = mux(
                ar_prio,
                U::<3>::from(PRIO & 7),
                mux(ar_hi, ar_hi_who, ar_lo_who),
            );
            let aw_who = mux(aw_hi, aw_hi_who, aw_lo_who);
            // Several reads may be outstanding at once, each carrying
            // its own identifier, so a host wins on any cycle the
            // peripheral side has room. Writes go one at a time: the
            // beats that follow carry no identifier, so the next host
            // waits until this burst has ended.
            let ar_go = ar_lo & ar.ready();
            let aw_go = aw_lo & aw.ready() & !wbusy;
            let w_room = w.ready();
            // The granted host's address phases and its beats, one
            // field at a time, and a take from each host that is the
            // one granted.
            let mut ar_id = ars[0].head().id;
            let mut ar_addr = ars[0].head().addr;
            let mut ar_len = ars[0].head().len;
            let mut ar_size = ars[0].head().size;
            let mut ar_burst = ars[0].head().burst;
            let mut ar_lock = ars[0].head().lock;
            let mut ar_cache = ars[0].head().cache;
            let mut ar_prot = ars[0].head().prot;
            let mut ar_qos = ars[0].head().qos;
            let mut ar_region = ars[0].head().region;
            let mut aw_id = aws[0].head().id;
            let mut aw_addr = aws[0].head().addr;
            let mut aw_len = aws[0].head().len;
            let mut aw_size = aws[0].head().size;
            let mut aw_burst = aws[0].head().burst;
            let mut aw_lock = aws[0].head().lock;
            let mut aw_cache = aws[0].head().cache;
            let mut aw_prot = aws[0].head().prot;
            let mut aw_qos = aws[0].head().qos;
            let mut aw_region = aws[0].head().region;
            let mut w_data = ws[0].head().data;
            let mut w_strb = ws[0].head().strb;
            let mut w_last = ws[0].head().last;
            let mut w_off = Bit::Zero;
            let mut won_aw = U::<N>::from(0u8);
            for i in 0..N {
                let ar_me = ar_who == i;
                let aw_me = aw_who == i;
                let w_me = wsel.bit(i);
                ar_id = mux(ar_me, ars[i].head().id, ar_id);
                ar_addr = mux(ar_me, ars[i].head().addr, ar_addr);
                ar_len = mux(ar_me, ars[i].head().len, ar_len);
                ar_size = mux(ar_me, ars[i].head().size, ar_size);
                ar_burst = mux(ar_me, ars[i].head().burst, ar_burst);
                ar_lock = mux(ar_me, ars[i].head().lock, ar_lock);
                ar_cache = mux(ar_me, ars[i].head().cache, ar_cache);
                ar_prot = mux(ar_me, ars[i].head().prot, ar_prot);
                ar_qos = mux(ar_me, ars[i].head().qos, ar_qos);
                ar_region = mux(ar_me, ars[i].head().region, ar_region);
                aw_id = mux(aw_me, aws[i].head().id, aw_id);
                aw_addr = mux(aw_me, aws[i].head().addr, aw_addr);
                aw_len = mux(aw_me, aws[i].head().len, aw_len);
                aw_size = mux(aw_me, aws[i].head().size, aw_size);
                aw_burst = mux(aw_me, aws[i].head().burst, aw_burst);
                aw_lock = mux(aw_me, aws[i].head().lock, aw_lock);
                aw_cache = mux(aw_me, aws[i].head().cache, aw_cache);
                aw_prot = mux(aw_me, aws[i].head().prot, aw_prot);
                aw_qos = mux(aw_me, aws[i].head().qos, aw_qos);
                aw_region = mux(aw_me, aws[i].head().region, aw_region);
                w_data = mux(w_me, ws[i].head().data, w_data);
                w_strb = mux(w_me, ws[i].head().strb, w_strb);
                w_last = mux(w_me, ws[i].head().last, w_last);
                let w_has = Bit::from(ws[i].peek().is_some());
                w_off = w_off | (w_me & w_has);
                let _ = ars[i].recv_if(ar_me & ar_go);
                let _ = aws[i].recv_if(aw_me & aw_go);
                let one = U::<N>::from(1u8) << i;
                won_aw = mux(aw_me, one, won_aw);
                let _ = ws[i].recv_if(wbusy & w_me & w_room);
            }
            if ar_go.to_bool() {
                ar.send(Ar {
                    id: ar_id.zext::<J>() | (ar_who.zext::<J>() << I),
                    addr: ar_addr,
                    len: ar_len,
                    size: ar_size,
                    burst: ar_burst,
                    lock: ar_lock,
                    cache: ar_cache,
                    prot: ar_prot,
                    qos: ar_qos,
                    region: ar_region,
                });
            }
            if aw_go.to_bool() {
                aw.send(Aw {
                    id: aw_id.zext::<J>() | (aw_who.zext::<J>() << I),
                    addr: aw_addr,
                    len: aw_len,
                    size: aw_size,
                    burst: aw_burst,
                    lock: aw_lock,
                    cache: aw_cache,
                    prot: aw_prot,
                    qos: aw_qos,
                    region: aw_region,
                });
            }
            // The beats themselves, from the host that won the address
            // phase and from no other.
            let w_go = wbusy & w_off & w_room;
            if w_go.to_bool() {
                w.send(W {
                    data: w_data,
                    strb: w_strb,
                    last: w_last,
                });
            }
            let w_done = w_go & w_last;
            // The answers, each to the host its identifier's top bits
            // name, with the tag taken off again.
            let rh = r.head();
            let r_off = Bit::from(r.peek().is_some());
            let r_who = rh.id >> I;
            let bh = b.head();
            let b_off = Bit::from(b.peek().is_some());
            let b_who = bh.id >> I;
            let mut r_taken = Bit::Zero;
            let mut b_taken = Bit::Zero;
            for i in 0..N {
                let to_r = r_off & (r_who == i) & rs[i].ready();
                if to_r.to_bool() {
                    rs[i].send(R {
                        id: rh.id.slice::<0, I>(),
                        data: rh.data,
                        resp: rh.resp,
                        last: rh.last,
                    });
                }
                r_taken = r_taken | to_r;
                let to_b = b_off & (b_who == i) & bs[i].ready();
                if to_b.to_bool() {
                    bs[i].send(B {
                        id: bh.id.slice::<0, I>(),
                        resp: bh.resp,
                    });
                }
                b_taken = b_taken | to_b;
            }
            let _ = r.recv_if(r_taken);
            let _ = b.recv_if(b_taken);
            // The turn moves past the host that won, so the next cycle
            // starts looking at the one after it.
            let last = U::<3>::from(N - 1);
            let ar_next = mux(ar_who == last, U::<3>::from(0u8), ar_who + 1);
            let aw_next = mux(aw_who == last, U::<3>::from(0u8), aw_who + 1);
            // The hold ends at its host's exclusive write or when its
            // cycles run out, and starts at an exclusive read from a
            // host when nobody else holds; a host's second exclusive
            // read starts it again.
            let hleft = self.hleft.get();
            let h_end = held & aw_go & (aw_who == howner) & aw_lock;
            let h_out = held & !h_end & Bit::from(hleft == 0);
            let h_start = (ar_go & ar_lock & Bit::from(HOLD != 0))
                & (!held | h_end | h_out | (ar_who == howner));
            with!(self <= {
                (ar_go & !fixed) ? { rturn: ar_next },
                (aw_go & !fixed) ? { wturn: aw_next },
                aw_go ? { wbusy: Bit::One, wsel: won_aw },
                w_done ? { wbusy: Bit::Zero },
                held ? hleft: hleft - 1,
                (h_end | h_out) ? hold: Bit::Zero,
                h_out ? htimeouts: self.htimeouts.get() + 1,
                h_start ? {
                    hold: Bit::One,
                    howner: ar_who,
                    hleft: U::<7>::from(127u8)
                },
            });
        }
    }
}
// end{part}

/// The arbiter against the rules it has to keep: every host's burst
/// reaches the peripheral, every answer comes back to the host that
/// asked, the beats of two write bursts do not interleave, and no
/// host is starved while another keeps asking.
#[cfg(test)]
mod tests {
    use super::{Arbiter, Arbiter4};
    use crate::bus::axi::{
        axi, AxiHost, AxiPer, Host, Per, Rd, Resp, Wr, Xact,
    };
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, now, Clock, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};

    /// A host's identifier is two bits, the peripheral side's five,
    /// which leaves three for the port number: more than the two
    /// hosts need, and exactly what four need.
    type HostUnit = AxiHost<16, 32, 4, 2, 8>;
    type PerUnit = AxiPer<16, 32, 4, 5>;
    type Job = Xact<16, 32, 4, 5>;
    type Client = Host<16, 32, 4, 2, 8>;

    /// A client, boxed so that two of different shapes can be passed
    /// to one driver.
    type Boxed = Box<dyn std::future::Future<Output = ()> + Unpin>;

    /// What the peripheral was asked to write, in the order the beats
    /// arrived, and what it was asked to read.
    #[derive(Clone, Default)]
    struct Seen(Rc<RefCell<Vec<(u128, Vec<u128>)>>>);

    /// A memory that answers a read with the address it was asked
    /// for, so a host can tell its own answers from another's, and
    /// records every write burst whole.
    fn memory(
        per: Per<16, 32, 4, 5>,
        seen: Seen,
    ) -> impl std::future::Future<Output = ()> {
        crate::bus::axi::serve(per, 4, move |x: Job| {
            let seen = seen.clone();
            async move {
                match x {
                    Xact::Read(rd) => {
                        let at = rd.addr().raw();
                        let words: Vec<U<32>> = (0..rd.words())
                            .map(|i| U::from((at + i as u128 * 4) as u32))
                            .collect();
                        rd.data(&words).await
                    }
                    Xact::Write(wr) => {
                        let beats: Vec<u128> =
                            wr.data().iter().map(|w| w.raw()).collect();
                        seen.0.borrow_mut().push((wr.addr().raw(), beats));
                        wr.ok().await
                    }
                }
            }
        })
    }

    /// Two hosts, an arbiter between them and one memory. Each host's
    /// client is built by the caller; the run is `n` cycles.
    fn two(
        a: impl FnOnce(Client) -> Boxed,
        b: impl FnOnce(Client) -> Boxed,
        n: usize,
    ) -> Seen {
        two_with::<0>(a, b, n).0
    }

    /// The same with the exclusive hold built or not, and the holds
    /// that ran out.
    fn two_with<const HOLD: usize>(
        a: impl FnOnce(Client) -> Boxed,
        b: impl FnOnce(Client) -> Boxed,
        n: usize,
    ) -> (Seen, u16) {
        let l0 = axi::<16, 32, 4, 2, 8>();
        let l1 = axi::<16, 32, 4, 2, 8>();
        let lp = axi::<16, 32, 4, 5, 8>();
        let seen = Seen::default();
        let mut h0 = HostUnit::default();
        let mut h1 = HostUnit::default();
        let mut pu = PerUnit::default();
        let mut arb = Arbiter::<2, 16, 32, 4, 2, 5, 0, HOLD>::default();
        let timeouts = arb.htimeouts;
        let hardware = join2(
            join2(
                h0.run(l0.host_in, l0.host_out),
                h1.run(l1.host_in, l1.host_out),
            ),
            join2(
                pu.run(lp.per_in, lp.per_out),
                arb.run(
                    (
                        [l0.per_in.0, l1.per_in.0],
                        [l0.per_in.1, l1.per_in.1],
                        [l0.per_in.2, l1.per_in.2],
                        lp.host_in.2,
                        lp.host_in.3,
                    ),
                    (
                        lp.host_out.0,
                        lp.host_out.1,
                        lp.host_out.2,
                        [l0.per_out.2, l1.per_out.2],
                        [l0.per_out.3, l1.per_out.3],
                    ),
                ),
            ),
        );
        let clients = join2(a(l0.host), b(l1.host));
        let mut sim = Running::new(join2(
            join2(hardware, memory(lp.per, seen.clone())),
            clients,
        ));
        for _ in 0..n {
            sim.cycle();
        }
        (seen, timeouts.get().raw() as u16)
    }

    /// `n` cycles of the clock, in a client.
    async fn wait(n: usize) {
        for _ in 0..n {
            DefaultClock::rising().await;
        }
    }

    /// The exclusive hold (issue 1408). Host 1's burst of eight is
    /// granted first and host 0's exclusive read comes while its beats
    /// are going out: the burst still finishes, since only a new grant
    /// is held. Host 1's next write is offered while host 0 computes,
    /// and with the hold it waits for host 0's exclusive write; without
    /// it, it goes first. No hold runs out.
    #[test]
    fn an_exclusive_pair_holds_the_other_hosts_writes() {
        let order = |held: bool| {
            let a = |host: Client| -> Boxed {
                Box::new(Box::pin(async move {
                    wait(3).await;
                    let rd = Rd {
                        lock: Bit::One,
                        ..Rd::at(0x100u32, 1)
                    };
                    host.read(rd).await.done().await;
                    wait(30).await;
                    let wr = Wr {
                        lock: Bit::One,
                        ..Wr::at(0x100u32)
                    };
                    host.write(wr, &[U::from(7u32)]).await.done().await;
                }))
            };
            let b = |host: Client| -> Boxed {
                Box::new(Box::pin(async move {
                    let burst: Vec<U<32>> = (0..8u32).map(U::from).collect();
                    let first = host.write(Wr::at(0x900u32), &burst).await;
                    wait(20).await;
                    let nine = [U::from(9u32)];
                    let second = host.write(Wr::at(0x980u32), &nine);
                    second.await.done().await;
                    first.done().await;
                }))
            };
            let (seen, timeouts) = if held {
                two_with::<1>(a, b, 400)
            } else {
                two_with::<0>(a, b, 400)
            };
            let at: Vec<u128> = seen.0.borrow().iter().map(|w| w.0).collect();
            (at, timeouts)
        };
        let (held, timeouts) = order(true);
        assert_eq!(held, vec![0x900, 0x100, 0x980], "held: the pair whole");
        assert_eq!(timeouts, 0, "the hold ended at the exclusive write");
        let (free, _) = order(false);
        assert_eq!(free, vec![0x900, 0x980, 0x100], "free: the write between");
    }

    /// A hold whose host never writes runs out after 128 cycles, and the
    /// other host's write is granted then, not before.
    #[test]
    fn a_hold_without_its_write_runs_out() {
        let done_at = Rc::new(RefCell::new(0u64));
        let d = done_at.clone();
        let (seen, timeouts) = two_with::<1>(
            |host| {
                Box::new(Box::pin(async move {
                    let rd = Rd {
                        lock: Bit::One,
                        ..Rd::at(0x100u32, 1)
                    };
                    host.read(rd).await.done().await;
                }))
            },
            move |host| {
                Box::new(Box::pin(async move {
                    // The clock runs on across runs in one thread.
                    let t0 = now();
                    wait(10).await;
                    let nine = [U::from(9u32)];
                    let w = host.write(Wr::at(0x980u32), &nine);
                    w.await.done().await;
                    *d.borrow_mut() = (now() - t0) / DefaultClock::PERIOD;
                }))
            },
            600,
        );
        assert_eq!(timeouts, 1, "one hold ran out");
        assert_eq!(seen.0.borrow().len(), 1, "and the write went then");
        assert!(*done_at.borrow() > 120, "not before: {}", done_at.borrow());
    }

    /// Two hosts read at once. Each gets its own answer back: the
    /// memory answers with the address, so a host that was given
    /// another's beats would see an address it never asked for.
    #[test]
    fn each_answer_goes_home() {
        let got0 = Rc::new(RefCell::new(Vec::new()));
        let got1 = Rc::new(RefCell::new(Vec::new()));
        let (r0, r1) = (got0.clone(), got1.clone());
        two(
            move |host| {
                Box::new(Box::pin(async move {
                    for at in [0x100u32, 0x200, 0x300] {
                        let p = host.read(Rd::at(at, 2)).await;
                        let r = p.done().await;
                        r0.borrow_mut().push((
                            r.resp,
                            r.data.iter().map(|w| w.raw()).collect::<Vec<_>>(),
                        ));
                    }
                }))
            },
            move |host| {
                Box::new(Box::pin(async move {
                    for at in [0x900u32, 0xa00, 0xb00] {
                        let p = host.read(Rd::at(at, 2)).await;
                        let r = p.done().await;
                        r1.borrow_mut().push((
                            r.resp,
                            r.data.iter().map(|w| w.raw()).collect::<Vec<_>>(),
                        ));
                    }
                }))
            },
            400,
        );
        let a = got0.borrow().clone();
        let b = got1.borrow().clone();
        assert_eq!(a.len(), 3, "the first host was answered three times");
        assert_eq!(b.len(), 3, "and so was the second");
        assert_eq!(a[0], (Resp::Okay, vec![0x100, 0x104]), "its own address");
        assert_eq!(a[1], (Resp::Okay, vec![0x200, 0x204]));
        assert_eq!(a[2], (Resp::Okay, vec![0x300, 0x304]));
        assert_eq!(b[0], (Resp::Okay, vec![0x900, 0x904]), "and its own");
        assert_eq!(b[1], (Resp::Okay, vec![0xa00, 0xa04]));
        assert_eq!(b[2], (Resp::Okay, vec![0xb00, 0xb04]));
    }

    /// Two hosts write multi-beat bursts at the same time. AXI4 puts
    /// no identifier on the write data channel, so the arbiter must
    /// hold that channel for the burst it granted; if it did not, the
    /// memory would be given a mixture of the two.
    #[test]
    fn write_beats_do_not_interleave() {
        let seen = two(
            |host| {
                Box::new(Box::pin(async move {
                    let p = host
                        .write(
                            Wr::at(0x1000u32),
                            &[
                                U::from(0xa0u32),
                                U::from(0xa1u32),
                                U::from(0xa2u32),
                                U::from(0xa3u32),
                            ],
                        )
                        .await;
                    assert_eq!(p.done().await.resp, Resp::Okay);
                }))
            },
            |host| {
                Box::new(Box::pin(async move {
                    let p = host
                        .write(
                            Wr::at(0x2000u32),
                            &[
                                U::from(0xb0u32),
                                U::from(0xb1u32),
                                U::from(0xb2u32),
                                U::from(0xb3u32),
                            ],
                        )
                        .await;
                    assert_eq!(p.done().await.resp, Resp::Okay);
                }))
            },
            400,
        );
        let mut bursts = seen.0.borrow().clone();
        bursts.sort_by_key(|(at, _)| *at);
        assert_eq!(bursts.len(), 2, "two bursts, whole");
        assert_eq!(
            bursts[0],
            (0x1000, vec![0xa0, 0xa1, 0xa2, 0xa3]),
            "the first host's beats, in order and unmixed"
        );
        assert_eq!(
            bursts[1],
            (0x2000, vec![0xb0, 0xb1, 0xb2, 0xb3]),
            "the second host's beats, in order and unmixed"
        );
    }

    /// Four hosts, all reading as fast as they can. Round robin means
    /// none of them is starved: the run is long enough for each to
    /// get several bursts through, and the counts are within one of
    /// each other.
    #[test]
    fn four_hosts_share_the_link() {
        let links: Vec<_> = (0..4).map(|_| axi::<16, 32, 4, 2, 8>()).collect();
        let lp = axi::<16, 32, 4, 5, 8>();
        let counts: Vec<Rc<RefCell<usize>>> =
            (0..4).map(|_| Rc::new(RefCell::new(0))).collect();
        let mut hs: Vec<HostUnit> =
            (0..4).map(|_| HostUnit::default()).collect();
        let mut pu = PerUnit::default();
        let mut arb = Arbiter4::<16, 32, 4, 2, 5, 0>::default();
        // Every host asks for one word, over and over, for as long as
        // the run lasts.
        let client = |host: Client, n: Rc<RefCell<usize>>| async move {
            loop {
                let p = host.read(Rd::at(0x10u32, 1)).await;
                let r = p.done().await;
                assert_eq!(r.resp, Resp::Okay);
                *n.borrow_mut() += 1;
            }
        };
        let mut it = links.into_iter();
        let (l0, l1, l2, l3) = (
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap(),
            it.next().unwrap(),
        );
        let (mut h3, mut h2, mut h1, mut h0) = (
            hs.pop().unwrap(),
            hs.pop().unwrap(),
            hs.pop().unwrap(),
            hs.pop().unwrap(),
        );
        let hardware = join2(
            join2(
                join2(
                    h0.run(l0.host_in, l0.host_out),
                    h1.run(l1.host_in, l1.host_out),
                ),
                join2(
                    h2.run(l2.host_in, l2.host_out),
                    h3.run(l3.host_in, l3.host_out),
                ),
            ),
            join2(
                pu.run(lp.per_in, lp.per_out),
                arb.run(
                    (
                        [l0.per_in.0, l1.per_in.0, l2.per_in.0, l3.per_in.0],
                        [l0.per_in.1, l1.per_in.1, l2.per_in.1, l3.per_in.1],
                        [l0.per_in.2, l1.per_in.2, l2.per_in.2, l3.per_in.2],
                        lp.host_in.2,
                        lp.host_in.3,
                    ),
                    (
                        lp.host_out.0,
                        lp.host_out.1,
                        lp.host_out.2,
                        [
                            l0.per_out.2,
                            l1.per_out.2,
                            l2.per_out.2,
                            l3.per_out.2,
                        ],
                        [
                            l0.per_out.3,
                            l1.per_out.3,
                            l2.per_out.3,
                            l3.per_out.3,
                        ],
                    ),
                ),
            ),
        );
        let clients = join2(
            join2(
                client(l0.host, counts[0].clone()),
                client(l1.host, counts[1].clone()),
            ),
            join2(
                client(l2.host, counts[2].clone()),
                client(l3.host, counts[3].clone()),
            ),
        );
        let mut sim = Running::new(join2(
            join2(hardware, memory(lp.per, Seen::default())),
            clients,
        ));
        for _ in 0..1200 {
            sim.cycle();
        }
        let got: Vec<usize> = counts.iter().map(|c| *c.borrow()).collect();
        let least = *got.iter().min().unwrap();
        let most = *got.iter().max().unwrap();
        assert!(least > 3, "every host got several bursts through: {got:?}");
        assert!(
            most - least <= 1,
            "and none was starved while another kept asking: {got:?}"
        );
    }
}
