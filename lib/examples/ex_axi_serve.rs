// SPDX-License-Identifier: Apache-2.0
//! Accepting many AXI transactions and forwarding them for
//! processing. A peripheral client takes transactions as they come
//! in, hands each to a pool of workers, holds them open while the
//! work runs, and answers each as its data becomes available, so the
//! answers leave in the order the work finished and not the order
//! they arrived.
//!
//! The same peripheral is written twice. The first is the shape by
//! hand: a process that accepts and forwards, a pool of workers
//! behind a channel, and a process that answers whatever the workers
//! finished, with `Open` holding the transactions in between. The
//! second is `axi::serve`, which is that shape as one call. The run
//! asserts the two answer alike.
//!
//! On the other end the host keeps three bursts in flight at all
//! times over ten bursts, so the four identifiers are recycled: an
//! identifier goes back to the tracker when the client has taken
//! that burst's answer, and never before, which is what lets an
//! answer wait uncollected.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, join_all, now, Clock, DefaultClock, Running, Rx, Tx, Unit,
};
use txhdl::pipeline::cycles;
use txhdl::types::U;
use txhdl::{Transaction, Value};
use txhdl_parts::bus::axi::{
    axi, serve, AxiHost, AxiPer, Link, Open, Per, Rd, Xact,
};

type HostUnit = AxiHost<16, 32, 4, 2, 4>;
type PerUnit = AxiPer<16, 32, 4, 2>;
type Job = Xact<16, 32, 4, 2>;

/// The bursts of the run: ten reads of one word, at ten addresses.
const READS: usize = 10;
/// How many the host keeps in flight, over four identifiers.
const IN_FLIGHT: usize = 3;
/// Workers in the pool, so three transactions may be open at once.
const WORKERS: usize = 3;

/// What a worker is given: which transaction, and what to read.
#[derive(Transaction, Value, Clone, Copy, Default)]
pub struct Ticket {
    pub id: U<2>,
    pub addr: U<16>,
}

/// What a worker gives back: the word, against the transaction it
/// belongs to.
#[derive(Transaction, Value, Clone, Copy, Default)]
pub struct Finished {
    pub id: U<2>,
    pub word: U<32>,
}

thread_local! {
    /// The tick the run being printed started at. Time runs on across
    /// the two runs, so each prints cycles of its own.
    static BASE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The cycle within the run being printed.
fn cyc() -> u64 {
    (now() - BASE.with(|b| b.get())) / 2
}

/// The word at an address: what the peripheral's memory holds.
fn word_at(addr: u128) -> U<32> {
    U::from(0x1000 + (addr as u32) * 0x11)
}

/// How long a worker takes over an address. Four different times, so
/// the answers finish in an order of their own.
fn work_cycles(addr: u128) -> usize {
    1 + (addr as usize % 4) * 3
}

/// The host client: ten one-word reads, three in flight at a time.
/// `taken` collects what came back, in the order it was collected.
async fn host_client(
    host: txhdl_parts::bus::axi::Host<16, 32, 4, 2, 4>,
    taken: Rc<RefCell<Vec<(u128, u128)>>>,
    say: bool,
) {
    let mut flying: VecDeque<(u128, _)> = VecDeque::new();
    for a in 0..READS as u128 {
        if flying.len() == IN_FLIGHT {
            let (addr, p): (u128, txhdl_parts::bus::axi::Pending<32, 2>) =
                flying.pop_front().unwrap();
            let r = p.done().await;
            taken.borrow_mut().push((addr, r.data[0].raw()));
        }
        let p = host.read(Rd::at(a as u32, 1)).await;
        if say {
            println!(
                "t={:>3}  issue read id {} at {:#04x}",
                cyc(),
                p.id().raw(),
                a
            );
        }
        flying.push_back((a, p));
    }
    while let Some((addr, p)) = flying.pop_front() {
        let r = p.done().await;
        taken.borrow_mut().push((addr, r.data[0].raw()));
    }
}

/// The peripheral, by hand: accept and forward, a pool of workers,
/// and a process that answers whatever finished.
async fn forward_by_hand(per: Per<16, 32, 4, 2>, say: bool) {
    let (job_tx, job_rx) = chan::<Ticket, DefaultClock>();
    let (fin_tx, fin_rx) = chan::<Finished, DefaultClock>();
    let open = Open::<16, 32, 4, 2>::new();
    let held = open.clone();
    // One process accepts whatever arrives and forwards it, holding
    // the transaction open until somebody has its data.
    let accept = async move {
        loop {
            let x = per.accept().await;
            let (id, addr) = (x.id(), x.addr());
            open.put(x);
            loop {
                DefaultClock::rising().await;
                if job_tx.ready().to_bool() {
                    job_tx.send(Ticket { id, addr });
                    break;
                }
            }
        }
    };
    // A pool of workers, each taking a ticket when it is free.
    let job_rx = Rc::new(job_rx);
    let fin_tx = Rc::new(fin_tx);
    let workers = join_all((0..WORKERS).map(|_| {
        let jobs: Rc<Rx<Ticket>> = job_rx.clone();
        let out: Rc<Tx<Finished>> = fin_tx.clone();
        async move {
            loop {
                let t = jobs.wait().await;
                cycles(work_cycles(t.addr.raw())).await;
                loop {
                    DefaultClock::rising().await;
                    if out.ready().to_bool() {
                        out.send(Finished {
                            id: t.id,
                            word: word_at(t.addr.raw()),
                        });
                        break;
                    }
                }
            }
        }
    }));
    // And one process answers, taking back the transaction the
    // finished work belongs to. It never called accept.
    let answer = async move {
        loop {
            let f = fin_rx.wait().await;
            if let Some(Xact::Read(r)) = held.take(f.id) {
                if say {
                    println!(
                        "t={:>3}  answer id {} word {:#x}",
                        cyc(),
                        f.id.raw(),
                        f.word.raw()
                    );
                }
                r.data(&[f.word]).await;
            }
        }
    };
    join2(accept, join2(workers, answer)).await
}

/// The same peripheral, as one call.
async fn forward_with_serve(per: Per<16, 32, 4, 2>, say: bool) {
    serve(per, WORKERS, move |x: Job| async move {
        let addr = x.addr().raw();
        cycles(work_cycles(addr)).await;
        if let Xact::Read(r) = x {
            if say {
                println!(
                    "t={:>3}  answer id {} word {:#x}",
                    cyc(),
                    r.id().raw(),
                    word_at(addr).raw()
                );
            }
            r.data(&[word_at(addr)]).await;
        }
    })
    .await
}

/// Drive one link for `cycles` cycles with the peripheral given.
fn drive(
    peripheral: impl FnOnce(Per<16, 32, 4, 2>, bool) -> BoxedFuture,
    say: bool,
    trace: bool,
) -> Vec<(u128, u128)> {
    let Link {
        host,
        per,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi::<16, 32, 4, 2, 4>();
    let mut host_unit = HostUnit::default();
    let mut per_unit = PerUnit::default();
    if trace {
        if let Some(mut w) = Wave::from_env() {
            w.clock::<DefaultClock>();
            w.add("issue", &host_in.0);
            w.add("release", &host_in.4);
            w.add("grant", &host_out.3);
            w.add("ar", &host_out.1);
            w.add("r", &host_in.3);
            w.add("req", &per_out.0);
            w.add("rb", &per_in.4);
            w.add("host", &host_unit);
            w.start();
        }
    }
    BASE.with(|b| b.set(now()));
    let taken = Rc::new(RefCell::new(Vec::new()));
    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            per_unit.run(per_in, per_out),
        ),
        join2(host_client(host, taken.clone(), say), peripheral(per, say)),
    ));
    for _ in 0..260 {
        sim.cycle();
    }
    if trace {
        stop();
    }
    let out = taken.borrow().clone();
    out
}

type BoxedFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>;

fn main() {
    println!("  t  what                       (the peripheral by hand)");
    let by_hand = drive(|p, say| Box::pin(forward_by_hand(p, say)), true, true);
    println!("\n  t  what                       (the same, with serve)");
    let by_serve =
        drive(|p, say| Box::pin(forward_with_serve(p, say)), true, false);

    println!("\ncollected, in order: {by_hand:x?}");
    assert_eq!(
        by_hand.len(),
        READS,
        "the bursts were not all collected by hand"
    );
    assert_eq!(by_hand, by_serve, "the two peripherals answered alike");
    for (addr, word) in &by_hand {
        assert_eq!(*word, word_at(*addr).raw(), "a read read the wrong word");
    }
    // Four identifiers over ten bursts: they were handed back and
    // taken again, which only happens once an answer has been read.
    println!("{READS} bursts over four identifiers, {IN_FLIGHT} in flight");
}
