// SPDX-License-Identifier: Apache-2.0
//! A counted loop whose bound is a register's value.
//!
//! `ex_serial` counted a loop of eight turns. A loop whose length is
//! only known when it is entered, a frame's byte count say, is a
//! `for` bounded by a register, `0..self.times.get().raw() as usize`.
//! The lowering compares the counter with the register as it stands, so
//! the register must hold still while the loop runs, which a register
//! the sequence itself sets before the loop does; and the counter is
//! as wide as the register, which the netlist reads off the unit's
//! fields when `lowered` runs.
//!
//! The repeater takes a word and a count on one channel and sends the
//! word that many times, one a cycle. The run is checked against the
//! netlist under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, now, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace, Transaction as TransactionDerive, Value};

// begin{unit}
/// A word and how many times to send it.
#[derive(TransactionDerive, Value, Clone, Copy, Default)]
pub struct Job {
    /// The word.
    pub word: U<8>,
    /// How many copies, one to fifteen.
    pub times: U<4>,
}

/// Sends a job's word as many times as it asks.
#[derive(Trace, Default)]
pub struct Repeater {
    /// The word being sent.
    pub word: Reg<U<8>>,
    /// How many times it goes.
    pub times: Reg<U<4>>,
}

#[lower]
impl Unit<Rx<Job>, Tx<U<8>>> for Repeater {
    async fn run(&mut self, jobs: Rx<Job>, out: Tx<U<8>>) {
        loop {
            let job = jobs.wait().await;
            self.word.set(job.word);
            self.times.set(job.times);
            // The count is read from the register an edge after it is
            // set, since a register set in a step holds its old value
            // until the edge.
            DefaultClock::rising().await;
            for _ in 0..self.times.get().raw() as usize {
                DefaultClock::rising().await;
                out.send(self.word.get());
            }
        }
    }
}
// end{unit}

fn main() {
    let (job_tx, jobs) = chan::<Job, DefaultClock>();
    let (out, out_rx) = chan::<U<8>, DefaultClock>();
    let mut repeater = Repeater::default();
    let times = repeater.times;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("jobs", &jobs);
        w.add("repeater", &repeater);
        w.add("out", &out);
        w.start();
    }
    let mut sim = Running::new(repeater.run(jobs, out));
    // Three jobs: three copies, one copy, four copies.
    let plan = [(0x11u8, 3u8), (0x22, 1), (0x33, 4)];
    let mut next = 0usize;
    let mut got: Vec<u8> = Vec::new();
    println!(" t job    times out");
    for _ in 0..16u32 {
        let job = if next < plan.len() && job_tx.ready().to_bool() {
            let (word, times) = plan[next];
            job_tx.send(Job {
                word: U::from(word),
                times: U::from(times),
            });
            next += 1;
            format!("{word:02x}x{times}")
        } else {
            "  -  ".to_string()
        };
        let out = match out_rx.recv() {
            Some(v) => {
                got.push(v.raw() as u8);
                format!("{:02x}", v.raw())
            }
            None => " -".to_string(),
        };
        sim.cycle();
        println!("{:2} {job:5} {:>5} {out}", now(), times.get().raw());
    }
    let mut want: Vec<u8> = Vec::new();
    for (word, times) in plan {
        want.extend(std::iter::repeat_n(word, times as usize));
    }
    assert_eq!(got, want, "each word as many times as asked");
    stop();
    let net = Repeater::lowered("repeater");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
