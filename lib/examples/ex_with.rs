// SPDX-License-Identifier: Apache-2.0
//! `with!`: the drives of one struct, its name written once. Each
//! entry is `field: value`; `c ? field: value` is a drive under a
//! condition; `c ? { .. } else { .. }` a group under one, with the
//! entries after `else` under its failure; and a memory word,
//! `m.at(i): v`, is a field like any other. Entries apply in order,
//! the last drive of a field winning. Both arms of a group exist in
//! the hardware at once and the condition selects, so nothing
//! branches; the lowering makes each an `if` in the clocked block.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    signal, Clock, DefaultClock, In, Mem, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

/// A four-entry table with a write port and a running total: a write
/// lands in the table and the total, a clear empties both, and the
/// last written index is kept for the read port.
#[derive(Trace, Default)]
pub struct Table {
    pub words: Mem<U<8>, 4>,
    pub total: Reg<U<8>>,
    pub last: Reg<U<2>>,
    pub written: Reg<U<4>>,
}

#[lower]
impl Unit for Table {
    async fn run(
        &mut self,
        (we, addr, data, clear): (In<Bit>, In<U<2>>, In<U<8>>, In<Bit>),
        (word, sum): (Out<U<8>>, Out<U<8>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let (we, addr, data) = (we.get(), addr.get(), data.get());
            with!(self <= {
                clear.get() ? {
                    total: 0,
                    written: 0,
                } else {
                    we ? {
                        words.at(addr): data,
                        total: self.total + data,
                        written: self.written + 1,
                    },
                },
                we ? last: addr,
            });
            word.set(self.words.read(self.last.get()));
            sum.set(self.total);
        }
    }
}

fn main() {
    let (we_out, we) = signal::<Bit, DefaultClock>();
    let (addr_out, addr) = signal::<U<2>, DefaultClock>();
    let (data_out, data) = signal::<U<8>, DefaultClock>();
    let (clear_out, clear) = signal::<Bit, DefaultClock>();
    let (word_out, word) = signal::<U<8>, DefaultClock>();
    let (sum_out, sum) = signal::<U<8>, DefaultClock>();
    let mut table = Table::default();
    let written = table.written;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("we", &we);
        w.add("addr", &addr);
        w.add("data", &data);
        w.add("clear", &clear);
        w.add("table", &table);
        w.add("word", &word);
        w.add("sum", &sum);
        w.start();
    }
    let mut sim =
        Running::new(table.run((we, addr, data, clear), (word_out, sum_out)));
    // Four writes, a read cycle, a clear, two more writes.
    let script: [(bool, u8, u8, bool); 9] = [
        (true, 0, 10, false),
        (true, 1, 20, false),
        (true, 2, 30, false),
        (true, 3, 40, false),
        (false, 0, 0, false),
        (false, 0, 0, true),
        (true, 1, 5, false),
        (true, 1, 7, false),
        (false, 0, 0, false),
    ];
    for (w, a, d, c) in script {
        we_out.set(w);
        addr_out.set(U::from(a));
        data_out.set(U::from(d));
        clear_out.set(c);
        sim.cycle();
        println!(
            "we={} addr={} data={:2} clear={} word={:3} sum={:3} written={}",
            w as u8,
            a,
            d,
            c as u8,
            word.get().raw(),
            sum.get().raw(),
            written.get().raw()
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Table::lowered("tally"));
    print!("\n{}", Table::verilog("tally"));
}
