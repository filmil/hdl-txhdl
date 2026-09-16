// SPDX-License-Identifier: Apache-2.0
//! A unit's ports as named structs rather than tuples. `Diff` takes
//! the two operands of a subtraction, channels of one type, so a swap
//! of the two in a tuple would compile and be wrong; as fields of a
//! struct each is named where it is declared, where it is passed and
//! where it is read. `Top` takes its operands as a role of an
//! `interface!`, and passes them to `Diff` in a struct literal whose
//! fields are joined by name. The netlist of the top is checked
//! against this run under both simulators, at its ports.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, signal, Chan, Clock, DefaultClock, Out, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{interface, lower, with, Trace};

// begin{unit}
/// The operands of a subtraction.
pub struct DiffIn<const W: usize> {
    pub minuend: Rx<U<W>>,
    pub subtrahend: Rx<U<W>>,
}

/// The difference, and whether the subtraction borrowed.
pub struct DiffOut<const W: usize> {
    pub diff: Tx<U<W>>,
    pub borrow: Out<Bit>,
}

/// Takes one of each operand when both are there and the output has
/// room, and counts the differences it sent.
#[derive(Trace, Default)]
pub struct Diff<const W: usize> {
    pub count: Reg<U<8>>,
}

#[lower]
impl<const W: usize> Unit for Diff<W> {
    async fn run(
        &mut self,
        DiffIn {
            minuend,
            subtrahend,
        }: DiffIn<W>,
        out: DiffOut<W>,
    ) {
        loop {
            DefaultClock::rising().await;
            let fire = out.diff.ready()
                & minuend.peek().is_some()
                & subtrahend.peek().is_some();
            let a = minuend.recv_if(fire).unwrap_or_default();
            let b = subtrahend.recv_if(fire).unwrap_or_default();
            with!(self <= { fire ? count: self.count + 1 });
            if fire.to_bool() {
                out.diff.send(a - b);
            }
            out.borrow.set(fire & (b > a));
        }
    }
}

interface! {
    Operands {
        minuend: Chan<U<8>>,
        subtrahend: Chan<U<8>>,
    }
    role Source { out minuend, out subtrahend }
    role Sink { in minuend, in subtrahend }
}

/// The operands come in as the sink of an interface, and go to the
/// child by name.
#[derive(Trace, Default)]
pub struct Top {
    pub diff: Diff<8>,
}

#[lower]
impl Unit for Top {
    async fn run(&mut self, inp: Sink, out: DiffOut<8>) {
        self.diff
            .run(
                DiffIn {
                    subtrahend: inp.subtrahend,
                    minuend: inp.minuend,
                },
                out,
            )
            .await;
    }
}
// end{unit}

/// The subtractions of the run: a minuend and a subtrahend.
const PAIRS: [(u8, u8); 6] =
    [(9, 4), (3, 7), (200, 56), (0, 1), (15, 15), (128, 1)];

fn main() {
    let (source, sink) = Operands::new();
    let (res_tx, res_rx) = chan::<U<8>, DefaultClock>();
    let (borrow_out, borrow) = signal::<Bit, DefaultClock>();
    let mut top = Top::default();
    let count = top.diff.count;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("minuend", &sink.minuend);
        w.add("subtrahend", &sink.subtrahend);
        w.add("top", &top);
        w.add("diff", &res_rx);
        w.add("borrow", &borrow);
        w.start();
    }
    let out = DiffOut {
        diff: res_tx,
        borrow: borrow_out,
    };
    let mut sim = Running::new(top.run(sink, out));
    // The minuends come every cycle, the subtrahends every other
    // cycle, and the sink takes nothing for three cycles.
    let (mut mi, mut si) = (0, 0);
    let mut results = 0;
    println!(" t minuend subtrahend count diff borrow");
    for t in 0..16 {
        let m = if mi < PAIRS.len() && source.minuend.ready().to_bool() {
            source.minuend.send(U::from(PAIRS[mi].0));
            mi += 1;
            format!("{}", PAIRS[mi - 1].0)
        } else {
            "-".to_string()
        };
        let s = if si < PAIRS.len()
            && t % 2 == 0
            && source.subtrahend.ready().to_bool()
        {
            source.subtrahend.send(U::from(PAIRS[si].1));
            si += 1;
            format!("{}", PAIRS[si - 1].1)
        } else {
            "-".to_string()
        };
        let taken = if (6..9).contains(&t) {
            None
        } else {
            res_rx.recv()
        };
        if let Some(d) = taken {
            let (a, b) = PAIRS[results];
            assert_eq!(d.raw() as u8, a.wrapping_sub(b), "pair {results}");
            results += 1;
        }
        sim.cycle();
        println!(
            "{:2} {:>7} {:>10} {:5} {:>4} {:6}",
            t,
            m,
            s,
            count.get().raw(),
            taken.map_or("-".to_string(), |d| d.raw().to_string()),
            borrow.get().to_bool() as u8,
        );
    }
    assert_eq!(results, PAIRS.len(), "every pair answered");
    stop();
    txhdl::netlist::write_vhdl_from_env(&Top::lowered("ports"));
    print!("\n{}", Top::verilog("ports"));
}
