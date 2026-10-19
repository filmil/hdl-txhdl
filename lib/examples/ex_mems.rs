// SPDX-License-Identifier: Apache-2.0
//! An array of memories and a loop over it, in a lowered body (issue
//! 1845).
//!
//! `Ways<N>` is the core of a set-associative table: `N` memories of
//! eight words, one field `way: Mems<U<8>, 8, N>`, every one read at
//! the same row in the same cycle. The body goes over them in
//! `for w in 0..N`: each way's word at the row is compared with the
//! key, which gives a bit of `hits`, and a write goes to the one way
//! `pick` names. The netlist of `Ways<4>` has the memories `way_0` to
//! `way_3`, each with the field's `#[ram_style]`, and is checked
//! against the run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    mux, signal, Clock, DefaultClock, In, Mems, Out, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

// begin{unit}
/// `N` ways of eight words, looked up together.
#[derive(Trace, Default)]
pub struct Ways<const N: usize> {
    /// One memory per way, `way_0` to `way_{N-1}` in the netlist.
    #[ram_style("distributed")]
    pub way: Mems<U<8>, 8, N>,
}

#[lower]
impl<const N: usize> Unit for Ways<N> {
    async fn run(
        &mut self,
        (row, key, wen, pick): (In<U<3>>, In<U<8>>, In<Bit>, In<U<2>>),
        hits: Out<U<4>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let r = row.get();
            let k = key.get();
            let mut h = U::<4>::from(0u8);
            for w in 0..N {
                // Every way read at the row, in the same cycle.
                let hit = Bit::from(self.way[w].read(r) == k);
                h = h | mux(hit, U::<4>::from(1u8 << w), U::<4>::from(0u8));
                // The key written into the way picked.
                let mine = wen.get() & Bit::from(pick.get() == w);
                with!(self <= { mine ? way[w].at(r): k });
            }
            hits.set(h);
        }
    }
}
// end{unit}

fn main() {
    let (row_out, row) = signal::<U<3>, DefaultClock>();
    let (key_out, key) = signal::<U<8>, DefaultClock>();
    let (wen_out, wen) = signal::<Bit, DefaultClock>();
    let (pick_out, pick) = signal::<U<2>, DefaultClock>();
    let (hits_out, hits) = signal::<U<4>, DefaultClock>();
    let mut unit = Ways::<4>::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("row", &row);
        w.add("key", &key);
        w.add("wen", &wen);
        w.add("pick", &pick);
        w.add("hits", &hits);
        w.add("ways", &unit);
        w.start();
    }
    let mut sim = Running::new(unit.run((row, key, wen, pick), hits_out));
    // Write 0x5a into ways 0 and 2 at row 3, and 0x3c into way 1 at
    // row 3; then look for each at row 3, and for 0x5a at row 4.
    let steps: [(u8, u8, bool, u8); 7] = [
        (3, 0x5a, true, 0),
        (3, 0x5a, true, 2),
        (3, 0x3c, true, 1),
        (3, 0x5a, false, 0),
        (3, 0x3c, false, 0),
        (4, 0x5a, false, 0),
        (3, 0x77, false, 0),
    ];
    let mut seen: Vec<u128> = Vec::new();
    for (r, k, we, p) in steps {
        row_out.set(U::from(r));
        key_out.set(U::from(k));
        wen_out.set(Bit::from_bool(we));
        pick_out.set(U::from(p));
        sim.cycle();
        seen.push(hits.get().raw());
    }
    println!("hits: {seen:?}");
    // The look at row 3 for 0x5a finds ways 0 and 2, for 0x3c way 1;
    // row 4 and the key 0x77 find none.
    assert_eq!(&seen[3..], &[0b0101, 0b0010, 0, 0]);
    stop();
    let net = Ways::<4>::lowered("ways");
    let mems: Vec<&str> = net
        .fields
        .iter()
        .filter(|f| f.1 == Some(txhdl::comp::trace::Kind::Mem))
        .map(|f| f.0)
        .collect();
    println!("memories: {mems:?}");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
