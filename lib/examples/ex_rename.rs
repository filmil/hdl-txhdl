// SPDX-License-Identifier: Apache-2.0
//! A field named for the netlist rather than by it. VHDL reserves
//! `next` and `buffer`, Verilog reserves `signed`, and a unit whose
//! field takes one of those names would write a netlist that does not
//! analyse. The lowering escapes such a name, to `next_rw` (issue
//! 497), but a queue that wants better names in its netlist keeps
//! `next` and `buffer` in Rust and says what the netlist should call
//! them, with `#[rename("...")]` (issue 222).
//!
//! The register that holds the next word out is `next` in Rust and
//! `nxt` in the netlist; the word behind it is `buffer` and `held`.
//! Everything that reads them is written in Rust, under the Rust
//! names, and the netlist has the other pair throughout: its
//! declarations, its drives and its trace.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, now, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// A queue of two words: one is about to leave, one is behind it.
#[derive(Trace, Default)]
pub struct Queue {
    /// The word about to leave. `next` is a reserved word of VHDL, so
    /// the netlist calls it `nxt`.
    #[rename("nxt")]
    pub next: Reg<U<8>>,
    /// The word behind it. `buffer` is reserved by VHDL too, and
    /// `buf` by Verilog, so the netlist calls it `held`.
    #[rename("held")]
    pub buffer: Reg<U<8>>,
    /// How many of the two words are filled.
    pub count: Reg<U<2>>,
}

#[lower]
impl Unit for Queue {
    async fn run(
        &mut self,
        (push, word, pop): (In<Bit>, In<U<8>>, In<Bit>),
        (head, full): (Out<U<8>>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let taking = push.get() & (self.count.get() < 2);
            let giving = pop.get() & (self.count.get() > 0);
            // A push with room fills the far end; a pop moves the word
            // behind up. The two can happen in one cycle.
            let empty = self.count.get() == 0;
            when!(taking & !giving => self {
                next: mux(empty, word.get(), self.next.get()),
                buffer: mux(empty, self.buffer.get(), word.get()),
                count: self.count.get() + 1
            });
            when!(giving & !taking => self {
                next: self.buffer.get(),
                count: self.count.get() - 1
            });
            when!(giving & taking => self {
                next: self.buffer.get(),
                buffer: word.get()
            });
            head.set(self.next.get());
            full.set(Bit::from(self.count.get() == 2));
        }
    }
}
// end{unit}

fn main() {
    let (push_out, push) = signal::<Bit, DefaultClock>();
    let (word_out, word) = signal::<U<8>, DefaultClock>();
    let (pop_out, pop) = signal::<Bit, DefaultClock>();
    let (head_out, head) = signal::<U<8>, DefaultClock>();
    let (full_out, full) = signal::<Bit, DefaultClock>();
    let mut queue = Queue::default();
    let count = queue.count;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("push", &push);
        w.add("word", &word);
        w.add("pop", &pop);
        w.add("queue", &queue);
        w.add("head", &head);
        w.add("full", &full);
        w.start();
    }
    let mut sim =
        Running::new(queue.run((push, word, pop), (head_out, full_out)));
    // Two pushes, then a pop a cycle, then pushes and pops together.
    let script: [(bool, u8, bool); 8] = [
        (true, 0x11, false),
        (true, 0x22, false),
        (true, 0x33, false),
        (false, 0, true),
        (false, 0, true),
        (true, 0x44, false),
        (true, 0x55, true),
        (false, 0, true),
    ];
    for (p, v, q) in script {
        push_out.set(p);
        word_out.set(U::<8>::from(v));
        pop_out.set(q);
        sim.cycle();
        println!(
            "t={:>2} count {} head {:02x} full {}",
            now(),
            count.get().raw(),
            head.get().raw(),
            full.get().to_bool() as u8
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Queue::lowered("queue"));
    print!("\n{}", Queue::verilog("queue"));
}
