// SPDX-License-Identifier: Apache-2.0
//! A set-associative table in block RAM (#1839): a key-to-value store
//! of `W` ways of `D = 1 << B` rows, with lookup, insert, update and
//! remove, a key's set chosen by a multiplicative hash.
//!
//! # The set
//!
//! A key's set is `(key * 0x9E3779B1) >> (32 - B)` over the key widened
//! to 32 bits: the top `B` bits of the product, a Fibonacci hash. A
//! key's low bits would crowd keys allocated in strides into a few
//! sets; the product spreads them. [`set_of`] is the one definition,
//! which the part and [`SetTableModel`] both use.
//!
//! # The ways
//!
//! Each way is three memories: its keys, its values and whether each
//! row is used, `Mems` of `W` (#1845). An operation reads every way at
//! its set at once, into registers at the edge, the synchronous read a
//! block RAM has; the cycle after, a loop over the ways finds a hit,
//! the first free way and whether the set is full, answers, and writes
//! the one way that changes back at that set. So every memory is read
//! at one address and written at one, which a block RAM holds.
//!
//! # Read-modify-write
//!
//! One operation is taken a cycle. One that reads the set the operation
//! before it is writing, in the same cycle, would read the row from
//! before that write; so the way written is forwarded into the row it
//! reads instead, and nothing stalls.
//!
//! # The operations
//!
//! * Lookup: the value of a key present (`HIT`), or `MISS`.
//! * Insert: a key not present goes into the lowest-numbered free way
//!   of its set (`HIT`); a key present changes nothing, and answers the
//!   value it already has (`EXISTS`); a full set takes nothing, answers
//!   `FULL`, and counts one refusal.
//! * Update: the value of a key present is replaced, and the new value
//!   answered (`HIT`), or `MISS`. A function of the old value is the
//!   user's: look up, work it out, update.
//! * Remove: a key present is freed and its value answered (`HIT`), or
//!   `MISS`.
//!
//! An operation is taken when its answer has room: when nothing is
//! being answered, or the answer is taken that cycle. An operation's
//! write is made once, in the cycle its answer goes out, however long
//! the answer waits.
//!
//! # Limits
//!
//! `B` is from 1 to 32, and `D` must be `1 << B`: stable Rust cannot
//! write the one from the other in a type, so both are given, and the
//! plain-Rust [`set_of`] and [`SetTableModel::new`] check the first. A
//! key is at most 32 bits, since the hash is over 32; `W` is at most
//! 256, since a way's number is eight bits.
use txhdl::comp::{
    mux, Clock, DefaultClock, Mems, Out, Reg, Regs, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl::{Transaction as TransactionDerive, Value as ValueDerive};

/// Look a key up.
pub const LOOKUP: u32 = 0;
/// Put a key and its value in.
pub const INSERT: u32 = 1;
/// Replace the value of a key present.
pub const UPDATE: u32 = 2;
/// Free a key.
pub const REMOVE: u32 = 3;

/// The key was there, or went in, or was replaced or freed.
pub const HIT: u32 = 0;
/// The key was not there.
pub const MISS: u32 = 1;
/// An insert of a key already there.
pub const EXISTS: u32 = 2;
/// An insert into a full set: refused and counted.
pub const FULL: u32 = 3;

/// The hash's multiplier, 2^32 over the golden ratio, odd.
pub const GOLDEN: u32 = 0x9E37_79B1;

/// An operation on the table.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct TableOp<const KW: usize, const VW: usize> {
    /// `LOOKUP`, `INSERT`, `UPDATE` or `REMOVE`.
    pub op: U<2>,
    /// The key.
    pub key: U<KW>,
    /// The value an insert or an update puts in.
    pub val: U<VW>,
}

/// An operation's answer.
#[derive(TransactionDerive, ValueDerive, Clone, Copy, Default, Debug)]
pub struct TableResult<const VW: usize> {
    /// `HIT`, `MISS`, `EXISTS` or `FULL`.
    pub res: U<2>,
    /// On a `HIT`, the value found, put in or freed; on `EXISTS`, the
    /// value the key already has; else nought.
    pub val: U<VW>,
}

/// The set of `key` in a table of `1 << b` sets: the top `b` bits of
/// `key * GOLDEN`, over the key widened to 32 bits. `b` is from 1 to 32.
pub fn set_of(key: u32, b: usize) -> usize {
    assert!((1..=32).contains(&b), "a table of 2^{b} sets: b is 1 to 32");
    (key.wrapping_mul(GOLDEN) >> (32 - b)) as usize
}

// begin{state}
/// `W` ways, at most 256, of `D = 1 << B` rows, `B` from 1 to 32, of a
/// key of `KW` bits, at most 32, and a value of `VW` bits.
#[derive(Trace, Default)]
pub struct SetTable<
    const KW: usize,
    const VW: usize,
    const B: usize,
    const D: usize,
    const W: usize,
> {
    /// Each way's keys, a memory per way.
    pub key: Mems<U<KW>, D, W>,
    /// Each way's values.
    pub val: Mems<U<VW>, D, W>,
    /// Whether each way's row is used.
    pub used: Mems<Bit, D, W>,
    /// An operation was read last cycle and is decided this one.
    pub busy: Reg<Bit>,
    /// Its kind: `LOOKUP`, `INSERT`, `UPDATE` or `REMOVE`.
    pub kind: Reg<U<2>>,
    /// Its key.
    pub okey: Reg<U<KW>>,
    /// Its value.
    pub oval: Reg<U<VW>>,
    /// Its set.
    pub set: Reg<U<B>>,
    /// Each way's key at that set, as read or as forwarded.
    pub rkey: Regs<U<KW>, W>,
    /// Each way's value there.
    pub rval: Regs<U<VW>, W>,
    /// Whether each way's row there is used.
    pub rused: Regs<Bit, W>,
    /// Inserts refused for a full set.
    pub refused: Reg<U<32>>,
}
// end{state}

// begin{run}
#[lower]
impl<
        const KW: usize,
        const VW: usize,
        const B: usize,
        const D: usize,
        const W: usize,
    > Unit<Rx<TableOp<KW, VW>>, (Tx<TableResult<VW>>, Out<U<32>>)>
    for SetTable<KW, VW, B, D, W>
{
    async fn run(
        &mut self,
        ops: Rx<TableOp<KW, VW>>,
        (results, refusals): (Tx<TableResult<VW>>, Out<U<32>>),
    ) {
        loop {
            DefaultClock::rising().await;
            refusals.set(self.refused.get());

            // Stage 2: the operation read last cycle, decided now.
            let busy = self.busy.get();
            let kind = self.kind.get();
            let okey = self.okey.get();
            let oval = self.oval.get();
            let set = self.set.get();
            let is_ins = Bit::from(kind == INSERT);
            let is_upd = Bit::from(kind == UPDATE);
            let is_rem = Bit::from(kind == REMOVE);
            // A hit, its way and value; the first free way.
            let mut hit = Bit::Zero;
            let mut hway = U::<8>::from(0u8);
            let mut hval = U::<VW>::from(0u8);
            let mut free = Bit::Zero;
            let mut fway = U::<8>::from(0u8);
            for w in 0..W {
                let u = self.rused[w].get();
                let here = u & Bit::from(self.rkey[w].get() == okey);
                let first_hit = here & !hit;
                hway = mux(first_hit, U::<8>::from(w as u8), hway);
                hval = mux(first_hit, self.rval[w].get(), hval);
                hit = hit | here;
                let first_free = !u & !free;
                fway = mux(first_free, U::<8>::from(w as u8), fway);
                free = free | !u;
            }
            // What the answer is.
            let full = is_ins & !hit & !free;
            let res = mux(
                hit,
                mux(is_ins, U::<2>::from(EXISTS as u8), U::<2>::from(0u8)),
                mux(
                    is_ins,
                    mux(free, U::<2>::from(0u8), U::<2>::from(FULL as u8)),
                    U::<2>::from(MISS as u8),
                ),
            );
            // The value answered: the one found, or for an update the
            // new one; an insert's own when it went in; else nought.
            let ins_go = is_ins & !hit & free;
            let rval = mux(
                hit,
                mux(is_upd, oval, hval),
                mux(ins_go, oval, U::<VW>::from(0u8)),
            );
            // The way written, and what goes into it: an insert's free
            // way, an update's or a remove's hit.
            let upd_go = is_upd & hit;
            let rem_go = is_rem & hit;
            let change = ins_go | upd_go | rem_go;
            let wway = mux(ins_go, fway, hway);
            let wkey = okey;
            let wval = mux(rem_go, hval, oval);
            let wused = !rem_go;

            // The answer goes out when there is room for it, and the
            // way changes once, as it does.
            let send = busy & results.ready();
            let write = send & change;

            // Stage 1: an operation taken when stage 2 is free or
            // leaving, its set hashed and every way read at it.
            let room = !busy | send;
            let take = room & Bit::from(ops.peek().is_some());
            let n = ops.head();
            let k32 = n.key.resize::<32>();
            let prod = k32 * U::<32>::from(GOLDEN);
            let nset = (prod >> (32 - B)).resize::<B>();
            // The same set as the way being written this cycle: its
            // row is forwarded rather than read from before the write.
            let fwd = write & Bit::from(nset == set);
            let _ = ops.recv_if(take);

            for w in 0..W {
                let mine = write & Bit::from(wway == w as u8);
                let fw = fwd & Bit::from(wway == w as u8);
                with!(self <= {
                    mine ? key[w].at(set): wkey,
                    mine ? val[w].at(set): wval,
                    mine ? used[w].at(set): wused,
                    take ? {
                        rkey[w]: mux(fw, wkey, self.key[w].read(nset)),
                        rval[w]: mux(fw, wval, self.val[w].read(nset)),
                        rused[w]: mux(fw, wused, self.used[w].read(nset)),
                    },
                });
            }
            with!(self <= {
                take ? {
                    kind: n.op,
                    okey: n.key,
                    oval: n.val,
                    set: nset,
                },
                busy: take | (busy & !send),
                send & full ? refused: self.refused.get() + 1,
            });

            if send.to_bool() {
                results.send(TableResult { res, val: rval });
            }
        }
    }
}
// end{run}

/// The table as plain Rust, the rule the part keeps, for a design's
/// tests: the same hash, the same way chosen, the same refusals.
#[derive(Clone, Debug)]
pub struct SetTableModel {
    b: usize,
    /// `rows[set][way]`: the key and value there, if used.
    rows: Vec<Vec<Option<(u32, u128)>>>,
    /// Inserts refused for a full set.
    pub refused: u32,
}

impl SetTableModel {
    /// An empty table of `ways` ways of `1 << b` rows, `b` from 1 to 32.
    pub fn new(b: usize, ways: usize) -> Self {
        assert!((1..=32).contains(&b), "a table of 2^{b} sets: b is 1 to 32");
        SetTableModel {
            b,
            rows: vec![vec![None; ways]; 1 << b],
            refused: 0,
        }
    }

    /// Apply one operation and give its answer, `(res, val)`.
    pub fn apply(&mut self, op: u32, key: u32, val: u128) -> (u32, u128) {
        let row = &mut self.rows[set_of(key, self.b)];
        let at = row
            .iter()
            .position(|e| matches!(e, Some((k, _)) if *k == key));
        match (op, at) {
            (LOOKUP, Some(w)) => (HIT, row[w].unwrap().1),
            (LOOKUP, None) => (MISS, 0),
            (INSERT, Some(w)) => (EXISTS, row[w].unwrap().1),
            (INSERT, None) => match row.iter().position(|e| e.is_none()) {
                Some(w) => {
                    row[w] = Some((key, val));
                    (HIT, val)
                }
                None => {
                    self.refused += 1;
                    (FULL, 0)
                }
            },
            (UPDATE, Some(w)) => {
                row[w] = Some((key, val));
                (HIT, val)
            }
            (UPDATE, None) => (MISS, 0),
            (REMOVE, Some(w)) => {
                let v = row[w].unwrap().1;
                row[w] = None;
                (HIT, v)
            }
            _ => (MISS, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use txhdl::comp::{chan, signal, Running};

    /// Eight sets of four ways, keys of 16 bits and values of 8.
    type Small = SetTable<16, 8, 3, 8, 4>;

    /// Run `ops` through a `Small`, each offered on the cycles `give`
    /// says and the answers taken on the cycles `take` says; the
    /// answers, and the refusals counted.
    fn run_paced(
        ops: &[(u32, u32, u32)],
        give: &mut dyn FnMut(usize) -> bool,
        take: &mut dyn FnMut(usize) -> bool,
    ) -> (Vec<(u32, u32)>, u32) {
        let (op_tx, op_rx) = chan::<TableOp<16, 8>, DefaultClock>();
        let (res_tx, res_rx) = chan::<TableResult<8>, DefaultClock>();
        let (ref_o, ref_i) = signal::<U<32>, DefaultClock>();
        let mut t = Small::default();
        let mut sim = Running::new(t.run(op_rx, (res_tx, ref_o)));
        let mut queue: Vec<TableOp<16, 8>> = ops
            .iter()
            .map(|&(op, key, val)| TableOp {
                op: U::from(op),
                key: U::from(key),
                val: U::from(val),
            })
            .collect();
        let mut got = vec![];
        let mut cycle = 0;
        while got.len() < ops.len() {
            if !queue.is_empty() && op_tx.ready().to_bool() && give(cycle) {
                op_tx.send(queue.remove(0));
            }
            if take(cycle) {
                if let Some(r) = res_rx.recv() {
                    got.push((r.res.raw() as u32, r.val.raw() as u32));
                }
            }
            sim.cycle();
            cycle += 1;
            assert!(cycle < 20 * ops.len() + 100, "the table stopped");
        }
        for _ in 0..4 {
            sim.cycle();
        }
        (got, ref_i.get().raw() as u32)
    }

    fn run(
        ops: &[(u32, u32, u32)],
        take: &mut dyn FnMut(usize) -> bool,
    ) -> (Vec<(u32, u32)>, u32) {
        run_paced(ops, &mut |_| true, take)
    }

    fn model(ops: &[(u32, u32, u32)]) -> (Vec<(u32, u32)>, u32) {
        let mut m = SetTableModel::new(3, 4);
        let got = ops
            .iter()
            .map(|&(op, key, val)| {
                let (r, v) = m.apply(op, key, val as u128);
                (r, v as u32)
            })
            .collect();
        (got, m.refused)
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn the_parameters_agree() {
        assert_eq!(1 << 3, 8, "Small's D is 1 << B");
        for k in 0..1000u32 {
            assert!(set_of(k, 3) < 8);
        }
    }

    /// Random operations on few keys, so that every answer occurs,
    /// against the model, with the answers taken on random cycles.
    #[test]
    fn random_operations_agree_with_the_model() {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        let keys: Vec<u32> = (0..48).map(|_| r.below(1 << 16) as u32).collect();
        let ops: Vec<(u32, u32, u32)> = (0..2000)
            .map(|_| {
                let op = match r.below(10) {
                    0..=3 => INSERT,
                    4..=6 => LOOKUP,
                    7 => UPDATE,
                    _ => REMOVE,
                };
                (op, keys[r.below(48) as usize], r.below(256) as u32)
            })
            .collect();
        let want = model(&ops);
        let kinds =
            |a: &[(u32, u32)], k: u32| a.iter().filter(|x| x.0 == k).count();
        for k in [HIT, MISS, EXISTS, FULL] {
            assert!(kinds(&want.0, k) > 20, "answer {k} occurs");
        }
        // Operations offered on three cycles in four, and answers taken
        // in runs: held up for up to eight cycles at a time.
        let mut gap = Rng(3);
        let mut stall = Rng(7);
        let mut held = 0;
        let got = run_paced(&ops, &mut |_| gap.below(4) != 0, &mut |_| {
            if held > 0 {
                held -= 1;
                return false;
            }
            if stall.below(6) == 0 {
                held = stall.below(8);
            }
            true
        });
        assert_eq!(got.1, want.1, "the refusals counted");
        for (i, (g, w)) in got.0.iter().zip(&want.0).enumerate() {
            assert_eq!(g, w, "operation {i}: {:?}", ops[i]);
        }
    }

    /// Operations on one set back to back: each reads the row the one
    /// before it is writing, through the forward.
    #[test]
    fn back_to_back_on_one_set_see_each_other() {
        let s = set_of(1, 3);
        let same: Vec<u32> = (1..4000u32)
            .filter(|&k| set_of(k, 3) == s)
            .take(6)
            .collect();
        let mut ops = vec![];
        for (i, &k) in same.iter().enumerate() {
            ops.push((INSERT, k, i as u32 + 1));
            ops.push((LOOKUP, k, 0));
        }
        ops.push((REMOVE, same[0], 0));
        ops.push((INSERT, same[5], 9));
        ops.push((UPDATE, same[1], 77));
        ops.push((LOOKUP, same[1], 0));
        let want = model(&ops);
        assert_eq!(run(&ops, &mut |_| true), want);
        assert_eq!(want.1, 2, "two inserts found the set full");
    }

    /// Keys in strides would crowd a few sets by their low bits; the
    /// hash spreads them over most of the sets.
    #[test]
    fn strided_keys_spread_over_the_sets() {
        for stride in [64u32, 4096] {
            let mut sets = std::collections::HashSet::new();
            for i in 0..256u32 {
                sets.insert(set_of(i * stride, 6));
            }
            assert!(sets.len() >= 48, "stride {stride}: {} sets", sets.len());
        }
    }

    #[test]
    fn each_way_is_a_memory_read_through_a_register() {
        let v = Small::lowered("small").verilog();
        for w in 0..4 {
            for m in ["key", "val", "used"] {
                assert!(v.contains(&format!("{m}_{w} [0:7]")), "{m}_{w}:\n{v}");
                assert!(v.contains(&format!("r{m}_{w} <= ")), "r{m}_{w}:\n{v}");
            }
        }
    }

    /// A table whose values are block RAMs, 1024 rows of 32 bits a way:
    /// each memory is read at one address and written at one, which the
    /// check that a memory of more than 4096 bits fits a block RAM's
    /// ports takes (issue 1285).
    #[test]
    fn a_large_table_fits_block_rams() {
        let net = SetTable::<32, 32, 10, 1024, 4>::lowered("large");
        let v = net.verilog();
        assert!(v.contains("val_3 [0:1023]"), "{v}");
    }
}
