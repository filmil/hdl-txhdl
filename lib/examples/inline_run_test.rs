//! A call of a function under `#[lower]` whose argument `lowered`
//! works out as it runs, and whose body reads a value of its own
//! twice, so the inlining holds that value in a wire (issue 126). The
//! unit's own wires are written before any such argument exists, so
//! the call's wire is added where the call is, one a turn of a loop
//! (issue 1186). Before that, neither unit here compiled.

use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Rx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

#[lower]
fn twice(v: U<8>, s: Bit) -> U<8> {
    let w = (v ^ (v + U::<8>::from(1u8))) & (v | U::<8>::from(3u8));
    mux(s, w, w + U::<8>::from(1u8))
}

/// The argument is a `let mut`.
#[derive(Trace, Default)]
pub struct Acc {
    pub a: Reg<U<8>>,
    pub s: Reg<Bit>,
    pub o: Reg<U<8>>,
}

#[lower]
impl Unit for Acc {
    async fn run(&mut self, _i: (), out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.o.get());
            let mut acc = self.a.get();
            acc = acc + U::<8>::from(1u8);
            self.o.set(twice(acc, self.s.get()));
        }
    }
}

/// The argument is a port a loop's index names.
#[derive(Trace, Default)]
pub struct Pick {
    pub s: Reg<Bit>,
    pub o: Reg<U<8>>,
}

#[allow(clippy::needless_range_loop)]
#[lower]
impl Unit for Pick {
    async fn run(&mut self, ins: [Rx<U<8>>; 2], out: Out<U<8>>) {
        loop {
            DefaultClock::rising().await;
            out.set(self.o.get());
            for i in 0..2 {
                self.o.set(twice(ins[i].head(), self.s.get()));
            }
        }
    }
}

#[test]
fn let_mut_argument() {
    let v = Acc::verilog("acc");
    assert!(v.contains("assign acc_l0 = (a + 8'b00000001);"), "{v}");
    assert!(
        v.contains(
            "assign twice_w_1_l1 = ((acc_l0 ^ (acc_l0 + 8'b00000001)) \
             & (acc_l0 | 8'b00000011));"
        ),
        "{v}"
    );
    assert!(
        v.contains("o <= (s ? twice_w_1_l1 : (twice_w_1_l1 + 8'b00000001));"),
        "{v}"
    );
}

#[test]
fn loop_index_argument() {
    let v = Pick::verilog("pick");
    for k in 0..2 {
        let d = format!("ins_{k}_data");
        let w = format!("twice_w_1_l{k}");
        let assign = format!(
            "assign {w} = (({d} ^ ({d} + 8'b00000001)) & ({d} | 8'b00000011));"
        );
        assert!(v.contains(&assign), "{v}");
        let set = format!("o <= (s ? {w} : ({w} + 8'b00000001));");
        assert!(v.contains(&set), "{v}");
    }
}
