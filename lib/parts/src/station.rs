// SPDX-License-Identifier: Apache-2.0
//! Reservation stations: `Station2` to `Station10`, one type per
//! count of inputs, each written out by `station!` with its line
//! struct beside it, and `Tagged`, what an input of any of them
//! carries. Each type says its rule.
use txhdl::station;
use txhdl::types::{Transaction, Value, U};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

// begin{part}
/// A tagged value: what a station's input carries, the tag above
/// the value.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default)]
pub struct Tagged<const TB: usize, T: Transaction + Value> {
    /// Which line this belongs to. `TB` bits, so a station has
    /// `1 << TB` lines and the tag is the line's index.
    pub tag: U<TB>,
    /// What this input contributes to that line.
    pub value: T,
}

station!(Station2, 2);
station!(Station3, 3);
station!(Station4, 4);
station!(Station5, 5);
station!(Station6, 6);
station!(Station7, 7);
station!(Station8, 8);
station!(Station9, 9);
station!(Station10, 10);
// end{part}

/// The stations against the rule written with loops: a model of the
/// same inputs, stepped on what the station sees, agrees with the
/// station's cells and with every line the sink takes.
#[cfg(test)]
mod tests {
    use super::{Line2, Station2, Station3, Tagged};
    use std::collections::{HashMap, VecDeque};
    use txhdl::comp::{chan, DefaultClock, Running, Unit};
    use txhdl::types::U;

    /// A line sent: the tag and the values, input by input.
    type Line = (u8, Vec<u128>);

    /// The rule. `occ[i]` holds input `i`'s cells as a mask, `cells[i]`
    /// their values.
    struct Model {
        occ: Vec<u64>,
        cells: Vec<HashMap<u8, u128>>,
    }

    impl Model {
        fn new(n: usize) -> Self {
            Model {
                occ: vec![0; n],
                cells: vec![HashMap::new(); n],
            }
        }

        /// One cycle: what each input offers, and whether the output
        /// has room. Returns the line sent, if one was.
        fn step(
            &mut self,
            heads: &[Option<(u8, u128)>],
            room: bool,
        ) -> Option<Line> {
            let n = heads.len();
            let held = |occ: &[u64], j: usize, t: u8| (occ[j] >> t) & 1 == 1;
            let offered: Vec<bool> =
                heads.iter().map(|h| h.is_some()).collect();
            let tag: Vec<u8> =
                heads.iter().map(|h| h.map_or(0, |x| x.0)).collect();
            let value: Vec<u128> =
                heads.iter().map(|h| h.map_or(0, |x| x.1)).collect();
            let free: Vec<bool> =
                (0..n).map(|i| !held(&self.occ, i, tag[i])).collect();
            let completes: Vec<bool> = (0..n)
                .map(|i| {
                    offered[i]
                        && free[i]
                        && (0..n).filter(|&j| j != i).all(|j| {
                            held(&self.occ, j, tag[i])
                                || (offered[j] && free[j] && tag[j] == tag[i])
                        })
                })
                .collect();
            let line = (0..n).find(|&i| completes[i]).map(|i| tag[i]);
            let send = line.filter(|_| room);
            let take: Vec<bool> = (0..n)
                .map(|i| {
                    offered[i]
                        && free[i]
                        && (!completes[i] || send == Some(tag[i]))
                })
                .collect();
            let out = send.map(|t| {
                let values = (0..n)
                    .map(|j| {
                        if held(&self.occ, j, t) {
                            self.cells[j][&t]
                        } else {
                            value[j]
                        }
                    })
                    .collect();
                (t, values)
            });
            for i in 0..n {
                if take[i] {
                    self.cells[i].insert(tag[i], value[i]);
                    self.occ[i] |= 1 << tag[i];
                }
            }
            if let Some(t) = send {
                for j in 0..n {
                    self.occ[j] &= !(1u64 << t);
                }
            }
            out
        }
    }

    /// The tags of one input, round by round: every tag once per
    /// round, in a random order, as operands of a stream of
    /// instructions arrive. Two inputs with independent random tags
    /// would deadlock, each waiting on a cell the other never fills,
    /// which the rule does not prevent and a real stream does not do.
    fn next_tag(pending: &mut Vec<u8>, x: u32) -> u8 {
        if pending.is_empty() {
            pending.extend(0..4u8);
            for i in (1..4).rev() {
                let j = ((x >> (i * 3)) as usize) % (i + 1);
                pending.swap(i, j);
            }
        }
        pending.pop().unwrap()
    }

    #[test]
    fn station2_follows_the_rule() {
        let (a_tx, a_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
        let (b_tx, b_rx) = chan::<Tagged<2, U<16>>, DefaultClock>();
        let (o_tx, o_rx) = chan::<Line2<2, U<8>, U<16>>, DefaultClock>();
        let (a_peek, b_peek) = (a_rx.clone(), b_rx.clone());
        let mut station = Station2::<2, 4, U<8>, U<16>>::default();
        let (occ0, occ1) = (station.occ0, station.occ1);
        let mut sim = Running::new(station.run((a_rx, b_rx), o_tx));
        let mut model = Model::new(2);
        let mut expected: VecDeque<Line> = VecDeque::new();
        let (mut a_tags, mut b_tags) = (Vec::new(), Vec::new());
        let mut in_chan = 0usize;
        let mut x = 0x9E37_79B9u32;
        let mut n = 1u32;
        let mut lines = 0;
        for _ in 0..400 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            // The sink takes at random.
            let mut took = 0;
            if x & 1 == 1 {
                if let Some(l) = o_rx.recv() {
                    let e = expected.pop_front().expect("a line nobody sent");
                    assert_eq!(
                        (l.tag.raw() as u8, vec![l.v0.raw(), l.v1.raw()]),
                        e
                    );
                    took = 1;
                }
            }
            // What the station sees this cycle, and the rule's answer.
            let heads = [
                a_peek.peek().map(|p| (p.tag.raw() as u8, p.value.raw())),
                b_peek.peek().map(|p| (p.tag.raw() as u8, p.value.raw())),
            ];
            let sent = model.step(&heads, in_chan < 2);
            if let Some(l) = sent.clone() {
                expected.push_back(l);
            }
            // New offers, each input's tags a round at a time.
            if x & 2 != 0 && a_tx.ready().to_bool() {
                let tag = U::from(next_tag(&mut a_tags, x));
                a_tx.send(Tagged {
                    tag,
                    value: U::from(n & 0xFF),
                });
                n += 1;
            }
            if x & 4 != 0 && b_tx.ready().to_bool() {
                let tag = U::from(next_tag(&mut b_tags, x >> 1));
                b_tx.send(Tagged {
                    tag,
                    value: U::from((n * 7) & 0xFFFF),
                });
                n += 1;
            }
            sim.cycle();
            in_chan = in_chan + sent.is_some() as usize - took;
            lines += sent.is_some() as usize;
            assert_eq!(occ0.get().raw() as u64, model.occ[0]);
            assert_eq!(occ1.get().raw() as u64, model.occ[1]);
        }
        assert!(lines > 60, "too few lines completed: {lines}");
        assert!(expected.len() <= 2);
    }

    /// Two lines complete in one cycle; the lower input's goes first
    /// and the other input waits a cycle.
    #[test]
    fn station3_lowest_input_wins() {
        let (a_tx, a_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
        let (b_tx, b_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
        let (c_tx, c_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
        let (o_tx, o_rx) =
            chan::<super::Line3<2, U<8>, U<8>, U<8>>, DefaultClock>();
        let mut station = Station3::<2, 4, U<8>, U<8>, U<8>>::default();
        let mut sim = Running::new(station.run((a_rx, b_rx, c_rx), o_tx));
        let send = |tx: &txhdl::comp::Tx<Tagged<2, U<8>>>, t: u8, v: u8| {
            tx.send(Tagged {
                tag: U::from(t),
                value: U::from(v),
            })
        };
        // Line 0 waits for input c; line 1 waits for inputs a and b.
        send(&a_tx, 0, 10);
        send(&b_tx, 0, 20);
        send(&c_tx, 1, 31);
        sim.cycle();
        sim.cycle();
        // Now a and b complete line 1 as c completes line 0, at once.
        send(&a_tx, 1, 11);
        send(&b_tx, 1, 21);
        send(&c_tx, 0, 30);
        sim.cycle();
        sim.cycle();
        sim.cycle();
        sim.cycle();
        let first = o_rx.recv().expect("a first line");
        sim.cycle();
        let second = o_rx.recv().expect("a second line");
        assert_eq!(first.tag.raw(), 1);
        assert_eq!(
            (first.v0.raw(), first.v1.raw(), first.v2.raw()),
            (11, 21, 31)
        );
        assert_eq!(second.tag.raw(), 0);
        assert_eq!(
            (second.v0.raw(), second.v1.raw(), second.v2.raw()),
            (10, 20, 30)
        );
    }
}
