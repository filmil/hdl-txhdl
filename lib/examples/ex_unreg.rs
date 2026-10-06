// SPDX-License-Identifier: Apache-2.0
//! Unregistered channels (issue 1293). A request goes through a
//! forwarding stage to a stage that answers from a register, and the
//! answer through a second forwarding stage, twice: in `Chain` every
//! channel between the children is registered, as every channel is
//! by default; in `ChainU` the two between the forwarding stages and
//! the answering one are marked `#[unregistered]`, so a transaction
//! crosses each in the cycle it is offered while the channel is
//! empty. Both take one request a cycle; the unregistered chain
//! answers each two cycles sooner.
//!
//! A forwarding stage passes its input's head to its output in the
//! same step it reads it, so through an unregistered channel the
//! receiver's logic follows the sender's in one cycle: every
//! unregistered hop lengthens a path, and is for hops whose two ends
//! are short. The answering stage holds its answer in a register,
//! which is what keeps the two unregistered channels from making one
//! combinational path end to end. Each child is joined before the
//! one it sends to, since the receiver of an unregistered channel has
//! to run after its sender in the step.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{units}
/// A forwarding stage: its input's head to its output, a transaction a
/// cycle, held while the output has no room.
#[derive(Trace, Default)]
pub struct Fwd {}

#[lower]
impl Unit for Fwd {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let h = inp.head();
            let go = inp.peek().is_some() & out.ready();
            let _ = inp.recv_if(out.ready());
            if go.to_bool() {
                out.send(h);
            }
        }
    }
}

/// A stage that answers each request from a register, the cycle after
/// it took it: the request plus one hundred.
#[derive(Trace, Default)]
pub struct Answer {
    pub v: Reg<U<8>>,
    pub full: Reg<Bit>,
}

#[lower]
impl Unit for Answer {
    async fn run(&mut self, inp: Rx<U<8>>, out: Tx<U<8>>) {
        loop {
            DefaultClock::rising().await;
            let send = self.full.get() & out.ready();
            if send.to_bool() {
                out.send(self.v.get());
            }
            let take = inp.peek().is_some() & (!self.full.get() | out.ready());
            let q = inp.head();
            let _ = inp.recv_if(take);
            with!(self <= {
                take ? {
                    v: q + U::<8>::from(100u8),
                    full: Bit::One,
                } else {
                    send ? full: Bit::Zero,
                },
            });
        }
    }
}
// end{units}

// begin{chains}
/// The three stages joined by registered channels.
#[derive(Trace, Default)]
pub struct Chain {
    pub fa: Fwd,
    pub ans: Answer,
    pub fb: Fwd,
}

#[lower]
impl Unit for Chain {
    async fn run(&mut self, req: Rx<U<8>>, res: Tx<U<8>>) {
        let (a_tx, a_rx) = chan::<U<8>, DefaultClock>();
        let (b_tx, b_rx) = chan::<U<8>, DefaultClock>();
        join2(
            self.fa.run(req, a_tx),
            join2(self.ans.run(a_rx, b_tx), self.fb.run(b_rx, res)),
        )
        .await;
    }
}

/// The same three, the two channels inside unregistered, each sender
/// joined before its receiver.
#[derive(Trace, Default)]
pub struct ChainU {
    pub fa: Fwd,
    pub ans: Answer,
    pub fb: Fwd,
}

#[lower]
impl Unit for ChainU {
    async fn run(&mut self, requ: Rx<U<8>>, resu: Tx<U<8>>) {
        #[unregistered]
        let (a_tx, a_rx) = chan::<U<8>, DefaultClock>();
        #[unregistered]
        let (b_tx, b_rx) = chan::<U<8>, DefaultClock>();
        join2(
            self.fa.run(requ, a_tx),
            join2(self.ans.run(a_rx, b_tx), self.fb.run(b_rx, resu)),
        )
        .await;
    }
}
// end{chains}

/// The requests: eight, one a cycle while there is room.
const REQUESTS: u8 = 8;

/// The cycles in which the sink takes nothing, so that answers back
/// up and an unregistered channel keeps an offer it was not taken.
const STALL: std::ops::Range<u32> = 7..10;

fn main() {
    let (req_tx, req_rx) = chan::<U<8>, DefaultClock>();
    let (res_tx, res_rx) = chan::<U<8>, DefaultClock>();
    let (requ_tx, requ_rx) = chan::<U<8>, DefaultClock>();
    let (resu_tx, resu_rx) = chan::<U<8>, DefaultClock>();
    let mut chain = Chain::default();
    let mut chainu = ChainU::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("req", &req_rx);
        w.add("res", &res_rx);
        w.add("requ", &requ_rx);
        w.add("resu", &resu_rx);
        w.add("chain", &chain);
        w.add("chainu", &chainu);
        w.start();
    }
    let mut sim = Running::new(join2(
        chain.run(req_rx, res_tx),
        chainu.run(requ_rx, resu_tx),
    ));
    let (mut sent, mut sentu) = (0u8, 0u8);
    let (mut got, mut gotu) = (Vec::new(), Vec::new());
    println!(" t  sent  answered registered  answered unregistered");
    for t in 0..24u32 {
        if sent < REQUESTS && req_tx.ready().to_bool() {
            req_tx.send(U::<8>::from(sent));
            sent += 1;
        }
        if sentu < REQUESTS && requ_tx.ready().to_bool() {
            requ_tx.send(U::<8>::from(sentu));
            sentu += 1;
        }
        let (r, ru) = if STALL.contains(&t) {
            (None, None)
        } else {
            (res_rx.recv(), resu_rx.recv())
        };
        if let Some(v) = r {
            got.push((t, v.raw() as u8));
        }
        if let Some(v) = ru {
            gotu.push((t, v.raw() as u8));
        }
        sim.cycle();
        let show = |v: Option<U<8>>| {
            v.map_or("-".to_string(), |v| format!("{}", v.raw()))
        };
        println!("{t:2}  {sent:4}  {:>19}  {:>21}", show(r), show(ru));
    }
    let want: Vec<u8> = (0..REQUESTS).map(|k| k + 100).collect();
    assert_eq!(got.iter().map(|g| g.1).collect::<Vec<_>>(), want);
    assert_eq!(gotu.iter().map(|g| g.1).collect::<Vec<_>>(), want);
    let (first, firstu) = (got[0].0, gotu[0].0);
    println!(
        "\nfirst answer: cycle {first} registered, cycle {firstu} \
         unregistered; nothing lost while the sink stalled"
    );
    assert_eq!(first, firstu + 2, "two hops fewer");
    stop();
    txhdl::netlist::write_netlists_from_env(&[
        &Chain::lowered("chain"),
        &ChainU::lowered("chain_u"),
    ]);
    print!("\n{}", ChainU::verilog("chain_u"));
}
