// SPDX-License-Identifier: Apache-2.0
//! A process that waits more than once, lowered to a state machine.
//!
//! A loop of one wait lowers to one clocked block, and every protocol
//! of several steps in the tree so far was written as a hand-numbered
//! machine: a `step` register, and `step == 3` at every statement that
//! belongs to the third step. Here the loop waits three times, and the
//! lowering numbers the waits instead (issue 501). A register the unit
//! does not declare, `at_wait`, holds the wait the process is at; each
//! wait's statements happen at the edge that leaves it, and the
//! register moves on at that edge, back to the first wait after the
//! last.
//!
//! The stepper takes a word, adds one at the next edge, and holds the
//! sum until `go`, when it sends it. Three waits of three kinds: on a
//! channel, on the clock, and on a condition. The run is checked
//! against the netlist under nvc and Verilator, and the netlist is
//! printed, with the machine in it.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, now, signal, until, Clock, DefaultClock, In, Reg, Running, Rx, Tx,
    Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// Takes a word, adds one, and sends the sum when told to.
#[derive(Trace, Default)]
pub struct Stepper {
    /// The word, then the sum.
    pub held: Reg<U<8>>,
}

#[lower]
impl Unit<(Rx<U<8>>, In<Bit>), Tx<U<8>>> for Stepper {
    async fn run(&mut self, (words, go): (Rx<U<8>>, In<Bit>), sums: Tx<U<8>>) {
        loop {
            // The first wait: a word, taken as it comes.
            let w = words.wait().await;
            self.held.set(w);
            // The second: the next edge, and one is added.
            DefaultClock::rising().await;
            self.held.set(self.held + 1);
            // The third: `go`, and the sum is offered.
            until(DefaultClock::rising, || go.get().to_bool()).await;
            sums.send(self.held.get());
        }
    }
}
// end{unit}

fn main() {
    let (word_tx, words) = chan::<U<8>, DefaultClock>();
    let (go_out, go) = signal::<Bit, DefaultClock>();
    let (sums, sum_rx) = chan::<U<8>, DefaultClock>();
    let mut stepper = Stepper::default();
    let held = stepper.held;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("words", &words);
        w.add("go", &go);
        w.add("stepper", &stepper);
        w.add("sums", &sums);
        w.start();
    }
    let mut sim = Running::new(stepper.run((words, go), sums));
    // Three words, each offered until taken; `go` comes late for the
    // first, at once for the second, and never for the third, which
    // the run ends holding.
    let mut next = 0usize;
    let mut taken = 0;
    println!(" t word go held sum");
    for t in 0..16u32 {
        let word = if next < 3 && word_tx.ready().to_bool() {
            let v = U::<8>::from([5u8, 40, 200][next]);
            word_tx.send(v);
            next += 1;
            format!("{:>3}", v.raw())
        } else {
            "  -".to_string()
        };
        let go = matches!(t, 4 | 5 | 8);
        go_out.set(go);
        let sum = match sum_rx.recv() {
            Some(s) => {
                taken += 1;
                format!("{:>3}", s.raw())
            }
            None => "  -".to_string(),
        };
        sim.cycle();
        println!(
            "{:2} {word} {:>2} {:>4} {sum}",
            now(),
            u8::from(go),
            held.get().raw()
        );
    }
    assert_eq!(taken, 2, "two sums were told to go");
    stop();
    let net = Stepper::lowered("stepper");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
