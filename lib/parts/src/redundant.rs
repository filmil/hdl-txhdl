// SPDX-License-Identifier: Apache-2.0
//! Two parts for running one design twice: a stream sent to both
//! copies, and a stream taken from both and checked.
//!
//! A machine that must not be quietly wrong runs two of itself and
//! compares. The comparison is only worth something if both copies see
//! exactly the same inputs, in the same cycles, and if neither can run
//! ahead of the other; both of those are what these two parts are for.
//!
//! `Tee` sends one stream to two receivers, and takes a word only when
//! both have room, so neither copy is a cycle ahead. `Check` takes a
//! word from each copy, in the same cycle, passes the first copy's on,
//! and raises a line that stays raised when the two differ.
//!
//! Neither holds the stream up otherwise. A `Tee` registers the word
//! it is passing, so that its netlist has no combinational path from
//! one side's handshake to the other's, and a `Check` holds one bit,
//! which is the disagreement it saw.
use std::marker::PhantomData;

use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, Transaction, Value};
use txhdl::{lower, with, Trace};

// begin{tee}
/// One stream to two receivers, in step: a word is taken into the tee
/// when both receivers have room, and both are offered it in the next
/// cycle.
///
/// The word is registered rather than passed through. A unit that read
/// a channel and drove one in the same cycle would be a combinational
/// path from one side's handshake to the other's, which is the shape
/// this repository keeps out of its channels, and its netlist could
/// not be replayed against the run cycle by cycle.
#[derive(Trace, Default)]
pub struct Tee<T: Transaction + Value> {
    /// The word being offered to both, when `full` says there is one.
    pub word: Reg<T>,
    /// Whether the tee holds a word neither side has taken.
    pub full: Reg<Bit>,
    /// Whether the first side has taken the word it holds.
    pub took_a: Reg<Bit>,
    /// Whether the second side has.
    pub took_b: Reg<Bit>,
}

#[lower]
impl<T: Transaction + Value> Unit for Tee<T> {
    async fn run(&mut self, inp: Rx<T>, (a, b): (Tx<T>, Tx<T>)) {
        loop {
            DefaultClock::rising().await;
            let full = self.full.get();
            let took_a = self.took_a.get();
            let took_b = self.took_b.get();
            // What each side does with the word this cycle: it takes
            // it if it has not already and has room now.
            let takes_a = full & !took_a & a.ready();
            let takes_b = full & !took_b & b.ready();
            let done = (took_a | takes_a) & (took_b | takes_b);
            // A new word is taken in the cycle the old one is done
            // with, or when there is none.
            let room = !full | done;
            let word = inp.head();
            let take = inp.peek().is_some() & room;
            with!(self <= {
                take ? word: word,
                full: mux(take, Bit::One, mux(done, Bit::Zero, full)),
                took_a: mux(take, Bit::Zero, took_a | takes_a),
                took_b: mux(take, Bit::Zero, took_b | takes_b),
            });
            let _ = inp.recv_if(take);
            if takes_a.to_bool() {
                a.send(self.word.get());
            }
            if takes_b.to_bool() {
                b.send(self.word.get());
            }
        }
    }
}
// end{tee}

// begin{check}
/// Two streams to one, with a line that says they differed.
///
/// A word moves when both copies offer one and the receiver has room.
/// What goes on is the first copy's; the line is raised in the cycle
/// the two differ and stays raised until `rst`.
#[derive(Trace, Default)]
pub struct Check<T: Transaction + Value + PartialEq> {
    /// Whether a disagreement has been seen since the last reset.
    pub differed: Reg<Bit>,
    /// What it compares. The check keeps no word: it compares the two
    /// heads as they pass and holds only the bit. The payload is named
    /// here because a type parameter a struct does not use is one Rust
    /// refuses, and `PhantomData` is neither state nor an end of a
    /// wire, so it is nothing in the netlist.
    _t: PhantomData<T>,
}

#[lower]
impl<T: Transaction + Value + PartialEq> Unit for Check<T> {
    async fn run(
        &mut self,
        (rst, one, two): (In<Bit>, Rx<T>, Rx<T>),
        (out, differs): (Tx<T>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let go = one.peek().is_some() & two.peek().is_some() & out.ready();
            let a = one.head();
            let b = two.head();
            let _ = one.recv_if(go);
            let _ = two.recv_if(go);
            let bad = go & (a != b);
            with!(self <= {
                rst.get() ? differed: Bit::Zero,
                bad & !rst.get() ? differed: Bit::One,
            });
            if go.to_bool() {
                out.send(a);
            }
            differs.set(self.differed.get() | bad);
        }
    }
}
// end{check}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, signal, Running};
    use txhdl::types::U;
    use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

    /// What the tests send: one byte.
    #[derive(
        TransactionDerive, ValueDerive, Clone, Copy, Default, Debug, PartialEq,
    )]
    struct Byte {
        v: U<8>,
    }

    /// Sends `words` into a tee and answers what each side received.
    fn tee_sees(words: &[u8], slow: bool) -> (Vec<u8>, Vec<u8>) {
        let (src_tx, src_rx) = chan::<Byte, DefaultClock>();
        let (a_tx, a_rx) = chan::<Byte, DefaultClock>();
        let (b_tx, b_rx) = chan::<Byte, DefaultClock>();
        let got_a: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let got_b: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let (ka, kb) = (got_a.clone(), got_b.clone());
        let mut tee = Tee::<Byte>::default();
        let sink_a = async move {
            loop {
                DefaultClock::rising().await;
                if let Some(w) = a_rx.recv() {
                    ka.borrow_mut().push(w.v.raw() as u8);
                }
            }
        };
        // The slow side takes a word every other cycle, which the tee
        // must hold the other side to.
        let sink_b = async move {
            let mut turn = 0;
            loop {
                DefaultClock::rising().await;
                turn += 1;
                if !slow || turn % 2 == 0 {
                    if let Some(w) = b_rx.recv() {
                        kb.borrow_mut().push(w.v.raw() as u8);
                    }
                }
            }
        };
        let list: Vec<u8> = words.to_vec();
        let client = async move {
            let mut i = 0;
            while i < list.len() {
                DefaultClock::rising().await;
                if src_tx.ready().to_bool() {
                    src_tx.send(Byte {
                        v: U::from(list[i]),
                    });
                    i += 1;
                }
            }
        };
        let mut sim = Running::new(join2(
            join2(sink_a, sink_b),
            join2(tee.run(src_rx, (a_tx, b_tx)), client),
        ));
        for _ in 0..80 {
            sim.cycle();
        }
        let a = got_a.borrow().clone();
        let b = got_b.borrow().clone();
        (a, b)
    }

    #[test]
    fn both_sides_of_a_tee_see_every_word() {
        let (a, b) = tee_sees(&[1, 2, 3, 4, 5], false);
        assert_eq!(a, vec![1, 2, 3, 4, 5]);
        assert_eq!(b, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_slow_side_holds_the_other_back() {
        // Neither side may be ahead: what the fast side has taken is
        // never more than what the slow one has.
        let (a, b) = tee_sees(&[1, 2, 3, 4, 5], true);
        assert_eq!(a, b, "one side ran ahead of the other");
        assert!(!a.is_empty(), "nothing went through at all");
    }

    /// Runs a check over the two lists and answers what came out and
    /// whether the line was up at the end.
    fn check_sees(one: &[u8], two: &[u8]) -> (Vec<u8>, bool) {
        let (p_tx, p_rx) = chan::<Byte, DefaultClock>();
        let (q_tx, q_rx) = chan::<Byte, DefaultClock>();
        let (o_tx, o_rx) = chan::<Byte, DefaultClock>();
        let (rst_o, rst) = signal::<Bit, DefaultClock>();
        let (bad_o, bad) = signal::<Bit, DefaultClock>();
        let out: Rc<RefCell<Vec<u8>>> = Rc::new(RefCell::new(Vec::new()));
        let kept = out.clone();
        let mut check = Check::<Byte>::default();
        let sink = async move {
            loop {
                DefaultClock::rising().await;
                if let Some(w) = o_rx.recv() {
                    kept.borrow_mut().push(w.v.raw() as u8);
                }
            }
        };
        let (l1, l2) = (one.to_vec(), two.to_vec());
        let client = async move {
            let (mut i, mut j) = (0, 0);
            while i < l1.len() || j < l2.len() {
                DefaultClock::rising().await;
                if i < l1.len() && p_tx.ready().to_bool() {
                    p_tx.send(Byte { v: U::from(l1[i]) });
                    i += 1;
                }
                if j < l2.len() && q_tx.ready().to_bool() {
                    q_tx.send(Byte { v: U::from(l2[j]) });
                    j += 1;
                }
            }
        };
        let mut sim = Running::new(join2(
            join2(sink, check.run((rst, p_rx, q_rx), (o_tx, bad_o))),
            client,
        ));
        rst_o.set(Bit::One);
        sim.cycle();
        rst_o.set(Bit::Zero);
        for _ in 0..80 {
            sim.cycle();
        }
        let got = out.borrow().clone();
        (got, bad.get().to_bool())
    }

    #[test]
    fn two_that_agree_pass_through_with_the_line_down() {
        let (out, bad) = check_sees(&[1, 2, 3], &[1, 2, 3]);
        assert_eq!(out, vec![1, 2, 3]);
        assert!(!bad, "the line went up on two that agreed");
    }

    #[test]
    fn a_difference_raises_the_line_and_keeps_it_up() {
        let (out, bad) = check_sees(&[1, 2, 3], &[1, 9, 3]);
        assert_eq!(out, vec![1, 2, 3], "the first copy's is what goes on");
        assert!(bad, "the line stayed down on two that differed");
    }
}
