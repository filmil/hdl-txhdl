// SPDX-License-Identifier: Apache-2.0
//! An output port in a process of several waits holds what a state
//! set until a state sets it again.
//!
//! The run's wire keeps its value between sets, so an output set in
//! one state and left alone in the next is still what it was. A wire
//! of the netlist cannot do that, so the lowering keeps such an
//! output in a register the unit does not declare, `<port>_held`,
//! set inside the state's arm; the port is the state's expression
//! while the state is about to leave and the register otherwise,
//! which is the run's wire seen a cycle ahead, as every wire of the
//! netlist is.
//!
//! The stepper of `ex_seq` again, with a `phase` output that says
//! which wait it is at, set in two of its three states, and a `busy`
//! output raised when a word is taken and dropped when the sum goes
//! out. The run is checked against the netlist under nvc and
//! Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, now, signal, until, Clock, DefaultClock, In, Out, Reg, Running, Rx,
    Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// Takes a word, adds one, and sends the sum when told to, saying
/// where it is.
#[derive(Trace, Default)]
pub struct Stepper {
    /// The word, then the sum.
    pub held: Reg<U<8>>,
}

#[lower]
impl Unit<(Rx<U<8>>, In<Bit>), (Tx<U<8>>, Out<U<2>>, Out<Bit>)> for Stepper {
    async fn run(
        &mut self,
        (words, go): (Rx<U<8>>, In<Bit>),
        (sums, phase, busy): (Tx<U<8>>, Out<U<2>>, Out<Bit>),
    ) {
        loop {
            let w = words.wait().await;
            self.held.set(w);
            phase.set(U::<2>::from(1u8));
            busy.set(Bit::One);
            DefaultClock::rising().await;
            self.held.set(self.held + 1);
            phase.set(U::<2>::from(2u8));
            until(DefaultClock::rising, || go.get().to_bool()).await;
            sums.send(self.held.get());
            busy.set(Bit::Zero);
        }
    }
}
// end{unit}

fn main() {
    let (word_tx, words) = chan::<U<8>, DefaultClock>();
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (sums, sum_rx) = chan::<U<8>, DefaultClock>();
    let (phase_out, phase) = signal::<U<2>, DefaultClock>();
    let (busy_out, busy) = signal::<Bit, DefaultClock>();
    let mut stepper = Stepper::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("words", &words);
        w.add("go", &go);
        w.add("stepper", &stepper);
        w.add("sums", &sums);
        w.add("phase", &phase);
        w.add("busy", &busy);
        w.start();
    }
    let mut sim =
        Running::new(stepper.run((words, go), (sums, phase_out, busy_out)));
    let mut next = 0usize;
    println!(" t word go phase busy sum");
    for t in 0..14u32 {
        let word = if next < 2 && word_tx.ready().to_bool() {
            let v = U::<8>::from([5u8, 40][next]);
            word_tx.send(v);
            next += 1;
            format!("{:>3}", v.raw())
        } else {
            "  -".to_string()
        };
        let go = matches!(t, 4 | 5 | 9);
        go_out.set(go);
        let sum = match sum_rx.recv() {
            Some(s) => format!("{:>3}", s.raw()),
            None => "  -".to_string(),
        };
        sim.cycle();
        println!(
            "{:2} {word} {:>2} {:>5} {:>4} {sum}",
            now(),
            u8::from(go),
            phase.get().raw(),
            u8::from(busy.get().to_bool())
        );
    }
    stop();
    let net = Stepper::lowered("hold");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
