// SPDX-License-Identifier: Apache-2.0
//! The link under fuzzing: a seeded host client and a seeded peripheral
//! client on the two ends of `AxiHost` and `AxiPer`, with a relay in
//! every one of the fifteen channels that stalls at random, sometimes
//! for a long time.
//!
//! The client issues bursts of one to sixteen beats, reads and writes
//! mixed, at pseudorandom addresses, keeps up to four in flight, and
//! awaits them in an order of its own, so identifiers come round again
//! while other answers stand uncollected. The peripheral serves four at
//! a time, each after a delay of its own, so answers leave out of
//! order, and answers a region of the address space with `SlvErr`.
//!
//! What is checked: every burst is answered exactly once; a read reads
//! what the last write to its addresses left; `last` falls on a burst's
//! final beat and nowhere else, on the write data and on the read data;
//! an identifier is never handed out while an answer stands uncollected
//! against it; and every run finishes within a bound, so nothing
//! deadlocks. What must have been seen, across the seeds: answers
//! awaited and answered out of order, identifiers recycled, error
//! responses, bursts of sixteen beats, and on every channel a
//! transaction held at a stalled relay for eight cycles or more.
//!
//! `AXI_FUZZ_SEEDS` sets how many seeds run; the build runs 24, and
//! the nightly run many more through `//lib/parts:axi_fuzz_test`.
use super::*;
use std::collections::HashSet;
use std::pin::Pin;
use txhdl::comp::{join2, Running};
use txhdl::types::Transaction;

const A: usize = 16;
const D: usize = 32;
const S: usize = 4;
const I: usize = 2;
const NIDS: usize = 4;

/// Where a burst may go: a region the peripheral serves, and a region
/// it answers with an error, both in words.
const MEMORY: u64 = 0x100;
const REFUSED: u64 = 0xe000;

/// The fifteen channels, in the order their relays are counted.
const CHANNELS: [&str; 15] = [
    "aw", "ar", "w", "b", "r", "issue", "wbeat", "release", "grant", "done",
    "rdata", "req", "wd", "ans", "rb",
];

type Boxed = Pin<Box<dyn Future<Output = ()>>>;

/// xorshift, as the soaks elsewhere in this crate use.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// What the runs saw, added up over every seed, for the coverage
/// assertions.
#[derive(Default)]
struct Seen {
    /// Per channel: transactions a relay held for eight cycles or more.
    held_long: [u64; 15],
    /// Per channel: transactions that passed.
    passed: [u64; 15],
    /// Per channel: cycles in which a relay's output had no room.
    full: [u64; 15],
    /// Identifiers granted again while another answer stood uncollected.
    recycled: u64,
    /// Handles awaited while an older one was still outstanding.
    awaited_out_of_order: u64,
    /// Transactions answered after one the peripheral accepted later.
    answered_out_of_order: u64,
    /// Error responses the client received.
    errors: u64,
    /// Bursts of sixteen beats.
    sixteen: u64,
    /// Bursts issued.
    bursts: u64,
}

// begin{relay}
/// A relay in one channel. Each cycle it either stalls, for one cycle
/// or for eight to forty-seven, or passes the transaction at its input
/// if its output has room, telling the monitor what passed.
async fn relay<T: Transaction>(
    k: usize,
    rx: Rx<T>,
    tx: Tx<T>,
    mut rng: Rng,
    seen: Rc<RefCell<Seen>>,
    on: Rc<dyn Fn(&T)>,
) {
    let mut stall = 0u64;
    let mut waiting = 0u64;
    loop {
        DefaultClock::rising().await;
        let offered = rx.peek().is_some();
        waiting = if offered { waiting + 1 } else { 0 };
        if waiting == 8 {
            seen.borrow_mut().held_long[k] += 1;
        }
        let room = tx.ready().to_bool();
        if !room {
            seen.borrow_mut().full[k] += 1;
        }
        if stall > 0 {
            stall -= 1;
            continue;
        }
        match rng.below(64) {
            0 => {
                stall = 8 + rng.below(40);
                continue;
            }
            1..=15 => continue,
            _ => {}
        }
        if offered && room {
            let v = rx.recv().unwrap();
            on(&v);
            tx.send(v);
            waiting = 0;
            seen.borrow_mut().passed[k] += 1;
        }
    }
}
// end{relay}

/// A channel with a relay in it: the end its sender holds, the end its
/// receiver holds, and the relay, kept to be run with the rest.
fn wire<T: Transaction>(
    relays: &mut Vec<Boxed>,
    rng: &mut Rng,
    seen: &Rc<RefCell<Seen>>,
    k: usize,
    on: Rc<dyn Fn(&T)>,
) -> (Tx<T>, Rx<T>) {
    let (up_tx, up_rx) = chan::<T, DefaultClock>();
    let (down_tx, down_rx) = chan::<T, DefaultClock>();
    let r = Rng::new(rng.next());
    relays.push(Box::pin(relay(k, up_rx, down_tx, r, seen.clone(), on)));
    (up_tx, down_rx)
}

// begin{monitor}
/// What passes on the AXI channels, against what AXI promises.
#[derive(Default)]
struct Monitor {
    /// Read beats still to come, by the identifier of the read.
    reads: HashMap<u128, usize>,
    /// Writes whose address phase has passed and whose response has
    /// not, by identifier.
    writes: HashSet<u128>,
    /// The beat counts of the writes, in the order the client issued
    /// them, which is the order their data travels in.
    lengths: VecDeque<usize>,
    /// Beats of the oldest write that have passed.
    beats: usize,
    /// Responses and last read beats that passed.
    answered: usize,
}

impl Monitor {
    fn ar(&mut self, a: &Ar<A, I>) {
        let id = a.id.raw();
        let fresh = self.reads.insert(id, a.len.raw() as usize + 1);
        assert!(fresh.is_none(), "read {id} issued while one is in flight");
    }
    fn aw(&mut self, a: &Aw<A, I>) {
        let id = a.id.raw();
        assert!(self.writes.insert(id), "write {id} issued twice");
    }
    fn w(&mut self, w: &W<D, S>) {
        let len = *self
            .lengths
            .front()
            .expect("a write beat for no write the client issued");
        self.beats += 1;
        assert_eq!(
            w.last.to_bool(),
            self.beats == len,
            "last on write beat {} of {len}",
            self.beats
        );
        if self.beats == len {
            self.lengths.pop_front();
            self.beats = 0;
        }
    }
    fn b(&mut self, b: &B<I>) {
        let id = b.id.raw();
        assert!(self.writes.remove(&id), "a response for no write, {id}");
        self.answered += 1;
    }
    fn r(&mut self, r: &R<D, I>) {
        let id = r.id.raw();
        let left = self
            .reads
            .get_mut(&id)
            .unwrap_or_else(|| panic!("a read beat for no read, {id}"));
        *left -= 1;
        assert_eq!(
            r.last.to_bool(),
            *left == 0,
            "last on read {id} with {} beats to come",
            *left
        );
        if *left == 0 {
            self.reads.remove(&id);
            self.answered += 1;
        }
    }
}
// end{monitor}

/// What the client expects of a burst it holds.
enum Expect {
    /// A write to memory: `Okay`.
    Wrote,
    /// A read of memory: `Okay`, and these words.
    Read(Vec<u128>),
    /// Either, in the refused region: `SlvErr`, and for a read this
    /// many beats.
    Refused(usize),
}

/// A burst in flight, as the client holds it.
struct Held {
    serial: usize,
    lo: u64,
    hi: u64,
    pending: Pending<D, I>,
    expect: Expect,
}

/// One seed: `bursts` bursts through the link. Panics on the first
/// broken promise, or if the run does not finish within the bound.
fn one_seed(seed: u64, bursts: usize, seen: &Rc<RefCell<Seen>>) {
    let mut rng = Rng::new(seed ^ 0x5eed_f00d);
    let mut relays: Vec<Boxed> = Vec::new();
    let mon = Rc::new(RefCell::new(Monitor::default()));

    // The five AXI channels, watched.
    let (m1, m2, m3, m4, m5) = (
        mon.clone(),
        mon.clone(),
        mon.clone(),
        mon.clone(),
        mon.clone(),
    );
    let (aw_tx, aw_rx) = wire::<Aw<A, I>>(
        &mut relays,
        &mut rng,
        seen,
        0,
        Rc::new(move |a| m1.borrow_mut().aw(a)),
    );
    let (ar_tx, ar_rx) = wire::<Ar<A, I>>(
        &mut relays,
        &mut rng,
        seen,
        1,
        Rc::new(move |a| m2.borrow_mut().ar(a)),
    );
    let (w_tx, w_rx) = wire::<W<D, S>>(
        &mut relays,
        &mut rng,
        seen,
        2,
        Rc::new(move |b| m3.borrow_mut().w(b)),
    );
    let (b_tx, b_rx) = wire::<B<I>>(
        &mut relays,
        &mut rng,
        seen,
        3,
        Rc::new(move |b| m4.borrow_mut().b(b)),
    );
    let (r_tx, r_rx) = wire::<R<D, I>>(
        &mut relays,
        &mut rng,
        seen,
        4,
        Rc::new(move |r| m5.borrow_mut().r(r)),
    );
    // The host client's six channels and the peripheral client's four.
    let (issue_tx, issue_rx) =
        wire::<Issue<A>>(&mut relays, &mut rng, seen, 5, Rc::new(|_| ()));
    let (wbeat_tx, wbeat_rx) =
        wire::<W<D, S>>(&mut relays, &mut rng, seen, 6, Rc::new(|_| ()));
    let (release_tx, release_rx) =
        wire::<Grant<I>>(&mut relays, &mut rng, seen, 7, Rc::new(|_| ()));
    let (grant_tx, grant_rx) =
        wire::<Grant<I>>(&mut relays, &mut rng, seen, 8, Rc::new(|_| ()));
    let (done_tx, done_rx) =
        wire::<Done<I>>(&mut relays, &mut rng, seen, 9, Rc::new(|_| ()));
    let (rdata_tx, rdata_rx) =
        wire::<R<D, I>>(&mut relays, &mut rng, seen, 10, Rc::new(|_| ()));
    let (req_tx, req_rx) =
        wire::<PerReq<A, I>>(&mut relays, &mut rng, seen, 11, Rc::new(|_| ()));
    let (wd_tx, wd_rx) =
        wire::<W<D, S>>(&mut relays, &mut rng, seen, 12, Rc::new(|_| ()));
    let (ans_tx, ans_rx) =
        wire::<Answer<I>>(&mut relays, &mut rng, seen, 13, Rc::new(|_| ()));
    let (rb_tx, rb_rx) =
        wire::<R<D, I>>(&mut relays, &mut rng, seen, 14, Rc::new(|_| ()));

    let mut host_unit = AxiHost::<A, D, S, I, NIDS>::default();
    let mut per_unit = AxiPer::<A, D, S, I>::default();
    let host: Host<A, D, S, I, NIDS> =
        host_end((issue_tx, wbeat_tx, release_tx, grant_rx, done_rx, rdata_rx));
    let per = Per {
        req: req_rx,
        wd: wd_rx,
        port: Rc::new(Port::new(ans_tx, rb_tx)),
        gathering: Rc::new(Cell::new(false)),
    };

    // The peripheral: a memory, four transactions at a time, each
    // after a delay of its own, and the refused region answered with
    // an error.
    let memory = Rc::new(RefCell::new(HashMap::<u64, u128>::new()));
    let prng = Rc::new(RefCell::new(Rng::new(rng.next())));
    let accepted = Rc::new(Cell::new(0usize));
    let last_answered = Rc::new(Cell::new(0usize));
    let seen_p = seen.clone();
    let server = serve(per, 4, move |x: Xact<A, D, S, I>| {
        let (memory, prng) = (memory.clone(), prng.clone());
        let (accepted, last_answered) =
            (accepted.clone(), last_answered.clone());
        let seen = seen_p.clone();
        async move {
            let serial = accepted.get() + 1;
            accepted.set(serial);
            let delay = prng.borrow_mut().below(24);
            for _ in 0..delay {
                DefaultClock::rising().await;
            }
            if serial < last_answered.get() {
                seen.borrow_mut().answered_out_of_order += 1;
            }
            last_answered.set(last_answered.get().max(serial));
            let addr = x.addr().raw() as u64;
            match x {
                Xact::Write(wx) if addr >= REFUSED => {
                    wx.err(Resp::SlvErr).await
                }
                Xact::Write(wx) => {
                    for (i, d) in wx.data().iter().enumerate() {
                        memory.borrow_mut().insert(addr + i as u64, d.raw());
                    }
                    wx.ok().await
                }
                Xact::Read(rx) if addr >= REFUSED => rx.err(Resp::SlvErr).await,
                Xact::Read(rx) => {
                    let words: Vec<U<D>> = (0..rx.words())
                        .map(|i| {
                            let m = memory.borrow();
                            U::new(*m.get(&(addr + i as u64)).unwrap_or(&0))
                        })
                        .collect();
                    rx.data(&words).await
                }
            }
        }
    });

    // The client: bursts at addresses no burst in flight overlaps, so
    // what a read must read is what the client wrote there last.
    let finished = Rc::new(Cell::new(false));
    let done_flag = finished.clone();
    let seen_c = seen.clone();
    let mon_c = mon.clone();
    let mut crng = Rng::new(rng.next());
    let client = async move {
        let mut model: HashMap<u64, u128> = HashMap::new();
        let mut held: Vec<Held> = Vec::new();
        let mut granted: HashSet<u128> = HashSet::new();
        let mut value = 1u128;
        for serial in 0..bursts {
            let words = 1 + crng.below(16) as usize;
            let refused = crng.below(8) == 0;
            let base = if refused { REFUSED } else { 0 };
            let lo = loop {
                let lo = base + crng.below(MEMORY);
                let hi = lo + words as u64;
                if held.iter().all(|h| hi <= h.lo || h.hi <= lo) {
                    break lo;
                }
            };
            let hi = lo + words as u64;
            let write = crng.below(2) == 0;
            let (pending, expect) = if write {
                let vs: Vec<U<D>> =
                    (0..words).map(|i| U::new(value + i as u128)).collect();
                mon_c.borrow_mut().lengths.push_back(words);
                let p = host.write(Wr::at(lo as u32), &vs).await;
                if refused {
                    (p, Expect::Refused(0))
                } else {
                    for (i, v) in vs.iter().enumerate() {
                        model.insert(lo + i as u64, v.raw());
                    }
                    (p, Expect::Wrote)
                }
            } else {
                let p = host.read(Rd::at(lo as u32, words)).await;
                if refused {
                    (p, Expect::Refused(words))
                } else {
                    let want = (lo..hi)
                        .map(|a| *model.get(&a).unwrap_or(&0))
                        .collect();
                    (p, Expect::Read(want))
                }
            };
            value += words as u128;
            let id = pending.id().raw();
            assert!(
                !held.iter().any(|h| h.pending.id().raw() == id),
                "seed {seed}: identifier {id} handed out while its answer \
                 stood uncollected"
            );
            {
                let mut s = seen_c.borrow_mut();
                s.bursts += 1;
                s.sixteen += (words == 16) as u64;
                if !held.is_empty() && granted.contains(&id) {
                    s.recycled += 1;
                }
            }
            granted.insert(id);
            held.push(Held {
                serial,
                lo,
                hi,
                pending,
                expect,
            });
            // Collect some of what is in flight, in an order of the
            // client's own: always when four are out, sometimes sooner.
            while !held.is_empty() && (held.len() == NIDS || crng.below(3) == 0)
            {
                let k = crng.below(held.len() as u64) as usize;
                collect(seed, held.remove(k), &held, &seen_c).await;
            }
        }
        while !held.is_empty() {
            let k = crng.below(held.len() as u64) as usize;
            collect(seed, held.remove(k), &held, &seen_c).await;
        }
        done_flag.set(true);
    };

    let hardware: Boxed = Box::pin(async move {
        join2(
            host_unit.run(
                (issue_rx, wbeat_rx, b_rx, r_rx, release_rx),
                (aw_tx, ar_tx, w_tx, grant_tx, done_tx, rdata_tx),
            ),
            per_unit.run(
                (aw_rx, ar_rx, w_rx, ans_rx, rb_rx),
                (req_tx, wd_tx, b_tx, r_tx),
            ),
        )
        .await;
    });
    relays.push(hardware);
    relays.push(Box::pin(server));
    relays.push(Box::pin(client));
    let mut sim = Running::new(join_all(relays));
    let bound = 400 * bursts;
    for _ in 0..bound {
        sim.cycle();
        if finished.get() {
            break;
        }
    }
    assert!(
        finished.get(),
        "seed {seed}: {bursts} bursts not all answered within {bound} cycles"
    );
    let m = mon.borrow();
    assert_eq!(m.answered, bursts, "seed {seed}: answers on the wire");
    assert!(m.reads.is_empty(), "seed {seed}: a read never finished");
    assert!(m.writes.is_empty(), "seed {seed}: a write never answered");
    assert!(m.lengths.is_empty(), "seed {seed}: write data never sent");
}

/// Wait for a burst's answer and check it against what the client
/// expected; `rest` is what is still in flight.
async fn collect(seed: u64, h: Held, rest: &[Held], seen: &Rc<RefCell<Seen>>) {
    if rest.iter().any(|o| o.serial < h.serial) {
        seen.borrow_mut().awaited_out_of_order += 1;
    }
    let (lo, serial) = (h.lo, h.serial);
    let reply = h.pending.done().await;
    let got: Vec<u128> = reply.data.iter().map(|d| d.raw()).collect();
    match h.expect {
        Expect::Wrote => {
            assert_eq!(reply.resp, Resp::Okay, "seed {seed}: write {serial}");
            assert!(got.is_empty(), "seed {seed}: a write came back with data");
        }
        Expect::Read(want) => {
            assert_eq!(reply.resp, Resp::Okay, "seed {seed}: read {serial}");
            assert_eq!(got, want, "seed {seed}: read {serial} at {lo:#x}");
        }
        Expect::Refused(beats) => {
            assert_eq!(reply.resp, Resp::SlvErr, "seed {seed}: burst {serial}");
            assert_eq!(
                got.len(),
                beats,
                "seed {seed}: refused {serial}'s beats"
            );
            seen.borrow_mut().errors += 1;
        }
    }
}

/// How many seeds to run: `AXI_FUZZ_SEEDS`, or 24.
fn seeds() -> u64 {
    std::env::var("AXI_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

#[test]
fn the_link_keeps_its_promises_under_fuzzing() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let n = seeds();
    for seed in 0..n {
        one_seed(seed, 64, &seen);
    }
    let s = seen.borrow();
    assert_eq!(s.bursts, 64 * n);
    assert!(s.awaited_out_of_order > 0, "no answer awaited out of order");
    assert!(s.answered_out_of_order > 0, "no answer given out of order");
    assert!(
        s.recycled > 0,
        "no identifier recycled while others were out"
    );
    assert!(s.errors > 0, "no error response");
    assert!(s.sixteen > 0, "no burst of sixteen beats");
    for (k, name) in CHANNELS.iter().enumerate() {
        assert!(s.passed[k] > 0, "nothing passed on {name}");
        assert!(
            s.held_long[k] > 0,
            "no transaction held eight cycles on {name}"
        );
    }
    assert!(
        s.full.iter().filter(|f| **f > 0).count() > 0,
        "no relay's output was ever full"
    );
}

/// A peripheral that answers a read with a beat too few. The link has
/// no way to finish such a read, so the answer is refused where it is
/// made, loudly, rather than left to wedge the client.
#[test]
#[should_panic(expected = "a read of 4 beats answered with 3")]
fn a_read_answered_with_the_wrong_beat_count_is_refused() {
    let Link {
        host,
        per,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi::<A, D, S, I, NIDS>();
    let client = async move {
        let r = host.read(Rd::at(0x10u32, 4)).await;
        let _ = r.done().await;
    };
    let server = async move {
        if let Xact::Read(r) = per.accept().await {
            let short = vec![U::<D>::from(0u8); 3];
            r.data(&short).await;
        }
    };
    let mut h = AxiHost::<A, D, S, I, NIDS>::default();
    let mut p = AxiPer::<A, D, S, I>::default();
    let mut sim = Running::new(join2(
        join2(h.run(host_in, host_out), p.run(per_in, per_out)),
        join2(client, server),
    ));
    for _ in 0..200 {
        sim.cycle();
    }
}
