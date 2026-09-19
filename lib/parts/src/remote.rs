// SPDX-License-Identifier: Apache-2.0
//! A peripheral whose behaviour is a program somewhere else.
//!
//! [`Remote`] sits on AXI-Lite like any other peripheral, and answers
//! nothing itself. Every transaction it accepts leaves on a channel
//! as an [`Ask`], and the answer comes back on another as an
//! [`Answer`]. What is at the other end of those two channels is not
//! the peripheral's business: in a simulation it is a model in the
//! same program, and on a board it is a frame on a wire and a program
//! on another machine.
//!
//! That is what it is for. A peripheral being designed can be written
//! as software first, against the bus it will really sit on, with the
//! rest of the system running as it really runs; when the software is
//! right, the hardware replaces it and nothing else changes. See
//! issue 297.
//!
//! What the bus sees is a slow device. One transaction is outstanding
//! at a time, and the channel that carries it is held until the
//! answer arrives, which is what AXI-Lite allows and what a program
//! on the other side of a wire needs.
//!
//! A device that never answers cannot be allowed to stop the bus, so
//! a transaction that goes unanswered for `T` cycles is answered
//! `SlvErr` here. Each carries a tag, and an answer whose tag is not
//! the one outstanding is dropped, which is what makes the timeout
//! safe: the answer that arrives too late belongs to a transaction
//! the bus has already finished with.

pub mod eth;
use txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};

// begin{wire}
/// What leaves the peripheral: one AXI-Lite transaction, whole.
///
/// `write` says which kind it is. A write carries `data` and `strb`;
/// a read leaves both at zero and wants `data` in the answer. `tag`
/// is the sequence number of the transaction, so an answer can be
/// matched to it and a late one told apart.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Ask {
    /// The transaction's sequence number, which its answer repeats.
    pub tag: U<8>,
    /// One for a write, zero for a read.
    pub write: Bit,
    /// The byte address, as the bus gave it.
    pub addr: U<32>,
    /// The word to write; zero on a read.
    pub data: U<32>,
    /// Which bytes of `data` the write covers; zero on a read.
    pub strb: U<4>,
}

/// What comes back: the answer to one [`Ask`].
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct Answer {
    /// The tag of the transaction being answered. An answer whose tag
    /// is not the one outstanding is dropped.
    pub tag: U<8>,
    /// The word a read read; ignored on a write.
    pub data: U<32>,
    /// One if the device says the transaction failed, which the bus
    /// is told as `SlvErr`.
    pub err: Bit,
}
// end{wire}

// begin{state}
/// An AXI-Lite peripheral whose transactions are answered elsewhere.
///
/// `T` is how many cycles a transaction may go unanswered before the
/// bus is told `SlvErr`. It is a number of cycles rather than a time,
/// so a design chooses it from its clock and from how far away the
/// program at the other end is: a model in the same simulation
/// answers in one or two, a program across a network takes thousands.
#[derive(Trace, Default)]
pub struct Remote<const T: usize> {
    /// Whether a transaction is outstanding.
    pub busy: Reg<Bit>,
    /// Whether the outstanding transaction is a write.
    pub writing: Reg<Bit>,
    /// The tag of the outstanding transaction, and of the next one:
    /// it counts up as each leaves.
    pub tag: Reg<U<8>>,
    /// How many cycles the outstanding transaction has waited.
    pub waited: Reg<U<32>>,
    /// The word the answer brought, held for the cycle the read is
    /// answered in.
    pub word: Reg<U<32>>,
    /// Whether the answer said the transaction failed, or the wait
    /// ran out.
    pub failed: Reg<Bit>,
    /// Whether an answer for the outstanding transaction is in hand.
    pub answered: Reg<Bit>,
}
// end{state}

// A struct literal's field is `name: value` to the lowering, which
// reads no shorthand (issue 235), so `tag: tag` stays as written.
#[allow(clippy::redundant_field_names)]
// begin{run}
#[lower]
impl<const T: usize> Unit for Remote<T> {
    async fn run(
        &mut self,
        (aw, ar, w, back): (
            Rx<LiteAw<32>>,
            Rx<LiteAr<32>>,
            Rx<LiteW<32, 4>>,
            Rx<Answer>,
        ),
        (b, r, out): (Tx<LiteB>, Tx<LiteR<32>>, Tx<Ask>),
    ) {
        loop {
            DefaultClock::rising().await;
            let busy = self.busy.get();
            let writing = self.writing.get();
            let tag = self.tag.get();
            let answered = self.answered.get();
            let awh = aw.head();
            let arh = ar.head();
            let wh = w.head();
            // A transaction leaves when nothing is outstanding, the
            // channel that carries it has room, and the bus is
            // offering one whole: a write wants its address and its
            // beat together, a read only its address. A write is
            // taken first, so a read cannot starve one.
            let wr = !busy & aw.peek().is_some() & w.peek().is_some();
            let rd = !busy & !wr & ar.peek().is_some();
            let go = (wr | rd) & out.ready();
            let _ = aw.recv_if(go & wr);
            let _ = w.recv_if(go & wr);
            let _ = ar.recv_if(go & rd);
            // The answer. One is taken whenever it is offered, so a
            // late one cannot block the channel, and it counts only
            // when its tag is the outstanding transaction's.
            let ans = back.head();
            let taking = back.peek().is_some();
            let _ = back.recv_if(taking);
            let mine = taking & busy & (ans.tag == tag);
            // The wait, and its end. A transaction answered late is
            // answered `SlvErr` here rather than held any longer.
            let waited = self.waited.get();
            let late = busy & (waited == U::<32>::from(T as u32));
            let have = (answered | mine) | late;
            // The bus is answered when there is something to say and
            // the channel it goes on has room.
            let say_b = busy & writing & have & b.ready();
            let say_r = busy & !writing & have & r.ready();
            let done = say_b | say_r;
            let bad = mux(mine, ans.err, self.failed.get()) | late;
            let word = mux(mine, ans.data, self.word.get());
            with!(self <= {
                go ? { busy: Bit::One, writing: wr,
                       waited: 0, answered: Bit::Zero, failed: Bit::Zero },
                // The tag moves on when the transaction is finished
                // with, not when it leaves: while it is outstanding,
                // `tag` is the tag an answer to it must carry.
                done ? { busy: Bit::Zero, answered: Bit::Zero,
                         tag: tag + 1 },
                mine & !done ? { answered: Bit::One, word: ans.data,
                                 failed: ans.err },
                busy & !go & !done ? waited: waited + 1,
            });
            if go.to_bool() {
                out.send(Ask {
                    tag: tag,
                    write: wr,
                    addr: mux(wr, awh.addr, arh.addr),
                    data: mux(wr, wh.data, U::<32>::from(0u32)),
                    strb: mux(wr, wh.strb, U::<4>::from(0u8)),
                });
            }
            if say_b.to_bool() {
                b.send(LiteB {
                    resp: mux(bad, Resp::SlvErr, Resp::Okay),
                });
            }
            if say_r.to_bool() {
                r.send(LiteR {
                    data: word,
                    resp: mux(bad, Resp::SlvErr, Resp::Okay),
                });
            }
        }
    }
}
// end{run}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteHost};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use txhdl::comp::{chan, join2, Running};

    type Host = LiteHost<32, 32, 4>;

    /// A write of one word, and the wait for its response.
    async fn write(h: &Host, addr: u32, data: u32) -> Resp {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(data),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(v) = b.recv() {
                return v.resp;
            }
        }
    }

    /// A read of one word, and the wait for its answer.
    async fn read(h: &Host, addr: u32) -> (Resp, u32) {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAr {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(v) = r.recv() {
                return (v.resp, v.data.raw() as u32);
            }
        }
    }

    /// The device, in software: a map of words, which answers after
    /// `delay` cycles and refuses an address it has never been given
    /// a word for. `deaf` makes it answer nothing at all, which is
    /// what the timeout is for.
    async fn device(
        asks: Rx<Ask>,
        answers: Tx<Answer>,
        delay: usize,
        deaf: bool,
    ) {
        {
            let mut words: HashMap<u32, u32> = HashMap::new();
            loop {
                DefaultClock::rising().await;
                let Some(ask) = asks.recv() else { continue };
                if deaf {
                    continue;
                }
                for _ in 0..delay {
                    DefaultClock::rising().await;
                }
                let addr = ask.addr.raw() as u32;
                let (data, err) = if ask.write.to_bool() {
                    words.insert(addr, ask.data.raw() as u32);
                    (0, false)
                } else {
                    match words.get(&addr) {
                        Some(w) => (*w, false),
                        None => (0, true),
                    }
                };
                loop {
                    if answers.ready().to_bool() {
                        answers.send(Answer {
                            tag: ask.tag,
                            data: U::from(data),
                            err: Bit::from(err),
                        });
                        break;
                    }
                    DefaultClock::rising().await;
                }
            }
        }
    }

    /// Runs `client` against the peripheral with a device behind it.
    fn run<F>(delay: usize, deaf: bool, client: impl FnOnce(Host) -> F)
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let (aw, ar, w, b, r) = link.per;
        let (ask_tx, ask_rx) = chan::<Ask, DefaultClock>();
        let (ans_tx, ans_rx) = chan::<Answer, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let body = client(link.host);
        // Forty cycles is long against a device that answers in one
        // or two, and short enough that a test of the timeout is not
        // a test of patience.
        let mut remote = Remote::<40>::default();
        let mut sim = Running::new(join2(
            join2(
                async move {
                    body.await;
                    *d.borrow_mut() = true;
                },
                device(ask_rx, ans_tx, delay, deaf),
            ),
            remote.run((aw, ar, w, ans_rx), (b, r, ask_tx)),
        ));
        for _ in 0..4000 {
            sim.cycle();
            if *done.borrow() {
                return;
            }
        }
        panic!("the client did not finish");
    }

    #[test]
    fn a_word_written_to_the_program_is_read_back_from_it() {
        run(1, false, |h| async move {
            assert_eq!(write(&h, 0x40, 0xc0ffee).await, Resp::Okay);
            assert_eq!(read(&h, 0x40).await, (Resp::Okay, 0xc0ffee));
        });
    }

    #[test]
    fn the_program_says_which_transactions_fail() {
        run(1, false, |h| async move {
            // The device refuses an address nothing has written.
            assert_eq!(read(&h, 0x80).await, (Resp::SlvErr, 0));
            assert_eq!(write(&h, 0x80, 7).await, Resp::Okay);
            assert_eq!(read(&h, 0x80).await, (Resp::Okay, 7));
        });
    }

    #[test]
    fn a_slow_program_is_waited_for() {
        // Twenty cycles a transaction, which is slow for a bus and
        // fast for a wire, and every answer still arrives.
        run(20, false, |h| async move {
            for i in 0..4u32 {
                assert_eq!(write(&h, 0x100 + 4 * i, i * 11).await, Resp::Okay);
            }
            for i in 0..4u32 {
                assert_eq!(read(&h, 0x100 + 4 * i).await, (Resp::Okay, i * 11));
            }
        });
    }

    #[test]
    fn a_program_that_never_answers_does_not_stop_the_bus() {
        run(0, true, |h| async move {
            // Each of these waits out the forty cycles and is told
            // the device failed, rather than holding the bus for ever.
            assert_eq!(write(&h, 0x40, 1).await, Resp::SlvErr);
            assert_eq!(read(&h, 0x40).await, (Resp::SlvErr, 0));
            assert_eq!(write(&h, 0x44, 2).await, Resp::SlvErr);
        });
    }
}
