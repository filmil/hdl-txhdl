// SPDX-License-Identifier: Apache-2.0
//! A state machine and its run under a reset in the middle (#1883).
//!
//! `ex_seq`'s stepper, reset twice: for two cycles while it waits for
//! `go`, after it has taken a word and added one, and for one cycle
//! just after it has taken another, while it adds one. Each time it goes
//! back to waiting for a word, as its netlist's machine does, and in
//! between it takes a word and sends its sum, to show that it does.
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
    chan, now, set_reset, signal, until, Clock, DefaultClock, In, Reg, Running,
    Rx, Tx, Unit,
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
    // Words offered until taken: 5, 40 and 200 before the first
    // reset, which empties the channel of the two the stepper had not
    // taken; 77 after it, to show the stepper waiting for a word again
    // and sending its sum; then 9, caught by a reset of one cycle.
    let words_at = |t: u32| match t {
        0..=2 => Some(()),
        7 | 13 => Some(()),
        _ => None,
    };
    let mut next = 0usize;
    let mut taken = 0;
    println!(" t word go held sum");
    for t in 0..24u32 {
        let word = if words_at(t).is_some() && word_tx.ready().to_bool() {
            let v = U::<8>::from([5u8, 40, 200, 77, 9][next]);
            word_tx.send(v);
            next += 1;
            format!("{:>3}", v.raw())
        } else {
            "  -".to_string()
        };
        // The reset at cycles 3 and 4, when the stepper has taken 5,
        // added one and waits for `go`: its third wait. And for one
        // cycle at 15, when it has taken 9 and is adding one: its
        // second.
        set_reset(matches!(t, 3 | 4 | 15));
        let go = matches!(t, 6 | 10 | 18);
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
    set_reset(false);
    println!("sums taken: {taken}");
    stop();
    let net = Stepper::lowered("stepper_r");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
