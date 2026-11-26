// SPDX-License-Identifier: Apache-2.0
//! A choice on a configuration constant. A unit generic over its
//! configuration chooses between two inputs on a constant flag, in each
//! of the ways a design writes one (issue 2069).
//!
//! A `mux` or an `if` that yields a value, on the flag itself, folds
//! when the unit is lowered if the operand it chooses brings its own
//! width: the netlist holds that operand and no mux, and the input not
//! chosen stays a port, read by nothing. The first configuration's `m`
//! drops a call of a lowered function this way, and keeps nothing of
//! it. A choice on a choice on the flag folds too, since the inner one
//! folds to a bit.
//!
//! The rest stay a mux whose condition is a constant: a selector
//! worked out of the flag with `!`, `&` or `==`, and an operand with no
//! width of its own, a bare number or the sum `(b + b)`, which takes its
//! width from the other. A `bool` constant is a one-bit literal
//! wherever it is written, by its type, so its negation chooses as the
//! run does (issue 2083); `q` reads it bare. An `if` statement on the
//! flag is folded, as before, and a check holds the flag. The netlists compute what the
//! run computes, and nvc accepts every line.
//!
//! `Both` holds the unit twice, once for each value of the flag, with
//! ports of its own for each. The build checks the netlist of `Both`
//! against the run under nvc and Verilator, so both lowerings are
//! compiled and replayed by both simulators.
use std::marker::PhantomData;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, mux, signal, Clock, DefaultClock, In, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{check, lower, Trace};

// begin{unit}
pub trait PickConfig: Default {
    const FIRST: bool;
}

#[derive(Default)]
pub struct First;
impl PickConfig for First {
    const FIRST: bool = true;
}

#[derive(Default)]
pub struct Second;
impl PickConfig for Second {
    const FIRST: bool = false;
}

/// A value doubled: a lowered function, for the operand not chosen.
#[lower]
fn twice(x: U<8>) -> U<8> {
    x + x
}

#[derive(Default, Trace)]
pub struct Pick<C: PickConfig> {
    /// Set by an `if` statement on the flag.
    pub r: Reg<U<8>>,
    _c: PhantomData<C>,
}

// `!C::FIRST == true` is one of the forms shown, written as a design
// might write it rather than as Clippy would simplify it.
#[allow(clippy::bool_comparison, clippy::nonminimal_bool)]
#[lower]
impl<C: PickConfig> Unit for Pick<C> {
    async fn run(
        &mut self,
        (a, b): (In<U<8>>, In<U<8>>),
        (m, i, k, o, s, q): (
            Out<U<8>>,
            Out<U<8>>,
            Out<U<12>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<8>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let first = Bit::from(C::FIRST);
            // The operand chosen where it brings its own width: `a` in
            // the first; the sum in the second has none, so it stays.
            m.set(mux(first, a.get(), twice(b.get())));
            let v = if C::FIRST { a.get() } else { b.get() };
            i.set(v);
            // A bare number has no width of its own: kept in the first,
            // where it is chosen; the second chooses the part.
            let top = mux(first, U::from(0u8), b.get().slice::<0, 4>());
            k.set(top.concat::<_, 12>(a.get()));
            // Selectors worked out of the flag stay a choice in both;
            // a choice on a choice folds, since the inner one is a bit.
            // Each instance reads `!first` where it counts: the first
            // in `same`, the second in `last`.
            let same =
                mux(!first, b.get(), mux(first == Bit::One, a.get(), b.get()));
            let inner = mux(mux(first, Bit::One, Bit::Zero), b.get(), a.get());
            let last = mux(!first, inner, b.get());
            o.set(mux(first & Bit::One, same, last));
            // The flag's negation written bare: behind a `let`, under
            // `&&` and `==`, and as a branch of a choice. Each is one
            // bit, so `q` is 0 in the first and `b` in the second; read
            // as a number, `!` would be 32 bits and true, and `q` would
            // be `a` in both (issue 2083).
            let p = !C::FIRST && a.get() != b.get();
            let e = !C::FIRST == true;
            let r = if a.get() != b.get() { e } else { false };
            q.set(mux(r, b.get(), mux(p, a.get(), U::from(0u8))));
            // An `if` statement on the flag, folded as before.
            s.set(self.r.get());
            if C::FIRST {
                self.r.set(a.get());
            } else {
                self.r.set(b.get());
            }
            check!(first == Bit::One || v == b.get(), "the second picks b");
        }
    }
}

/// Both values of the flag, side by side.
#[derive(Default, Trace)]
pub struct Both {
    pub first: Pick<First>,
    pub second: Pick<Second>,
}

#[lower]
impl Unit for Both {
    async fn run(
        &mut self,
        (fa, fb, sa, sb): (In<U<8>>, In<U<8>>, In<U<8>>, In<U<8>>),
        (fm, fi, fk, fo, fs, fq, sm, si, sk, so, ss, sq): (
            Out<U<8>>,
            Out<U<8>>,
            Out<U<12>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<12>>,
            Out<U<8>>,
            Out<U<8>>,
            Out<U<8>>,
        ),
    ) {
        join2(
            self.first.run((fa, fb), (fm, fi, fk, fo, fs, fq)),
            self.second.run((sa, sb), (sm, si, sk, so, ss, sq)),
        )
        .await;
    }
}
// end{unit}

fn main() {
    let (fa_out, fa) = signal::<U<8>, DefaultClock>();
    let (fb_out, fb) = signal::<U<8>, DefaultClock>();
    let (sa_out, sa) = signal::<U<8>, DefaultClock>();
    let (sb_out, sb) = signal::<U<8>, DefaultClock>();
    let mut both = Both::default();
    let (fm_out, fm) = signal::<U<8>, DefaultClock>();
    let (fi_out, fi) = signal::<U<8>, DefaultClock>();
    let (fk_out, fk) = signal::<U<12>, DefaultClock>();
    let (fo_out, fo) = signal::<U<8>, DefaultClock>();
    let (fs_out, fs) = signal::<U<8>, DefaultClock>();
    let (fq_out, fq) = signal::<U<8>, DefaultClock>();
    let (sm_out, sm) = signal::<U<8>, DefaultClock>();
    let (si_out, si) = signal::<U<8>, DefaultClock>();
    let (sk_out, sk) = signal::<U<12>, DefaultClock>();
    let (so_out, so) = signal::<U<8>, DefaultClock>();
    let (ss_out, ss) = signal::<U<8>, DefaultClock>();
    let (sq_out, sq) = signal::<U<8>, DefaultClock>();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("fa", &fa);
        w.add("fb", &fb);
        w.add("sa", &sa);
        w.add("sb", &sb);
        w.add("fm", &fm);
        w.add("fi", &fi);
        w.add("fo", &fo);
        w.add("fs", &fs);
        w.add("fq", &fq);
        w.add("sm", &sm);
        w.add("si", &si);
        w.add("so", &so);
        w.add("ss", &ss);
        w.add("sq", &sq);
        w.add("fk", &fk);
        w.add("sk", &sk);
        w.add("both", &both);
        w.start();
    }
    let mut sim = Running::new(both.run(
        (fa, fb, sa, sb),
        (
            fm_out, fi_out, fk_out, fo_out, fs_out, fq_out, sm_out, si_out,
            sk_out, so_out, ss_out, sq_out,
        ),
    ));
    for c in 0..4u8 {
        let (x, y) = (U::from(10 + c), U::from(20 + c));
        fa_out.set(x);
        fb_out.set(y);
        sa_out.set(x);
        sb_out.set(y);
        sim.cycle();
        println!(
            "a {} b {}: first {} {} {:03x} {} {} {}; {} {} {:03x} {} {} {}",
            10 + c,
            20 + c,
            fm.get().raw(),
            fi.get().raw(),
            fk.get().raw(),
            fo.get().raw(),
            fs.get().raw(),
            fq.get().raw(),
            sm.get().raw(),
            si.get().raw(),
            sk.get().raw(),
            so.get().raw(),
            ss.get().raw(),
            sq.get().raw()
        );
    }
    stop();
    // The lines of each lowering that drive its outputs, and its check.
    let both = Both::lowered("both");
    let (lf, ls) = (
        Pick::<First>::lowered("pick_first"),
        Pick::<Second>::lowered("pick_second"),
    );
    for (n, h) in [("first", lf.vhdl()), ("second", ls.vhdl())] {
        println!("\n{n}, VHDL:");
        let shown = |l: &&str| l.contains(" <= ") || l.contains("assert");
        for l in h.lines().filter(shown) {
            println!("{}", l.trim());
        }
    }
    for (n, v) in [("first", lf.verilog()), ("second", ls.verilog())] {
        println!("\n{n}, Verilog:");
        for l in v.lines().filter(|l| l.contains("assign ")) {
            println!("{}", l.trim());
        }
    }
    txhdl::netlist::write_vhdl_from_env(&both);
}
