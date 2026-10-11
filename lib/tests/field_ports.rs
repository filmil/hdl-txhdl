// SPDX-License-Identifier: Apache-2.0
//! A field of a struct literal passed to a child joins the one port the
//! child declared for it, `side_path`, and not every port whose name
//! ends in the field's (issue 1637). Both shapes the issue names lower:
//! a field `ready` beside a nested field `int.rs1_ready` on one side,
//! and a field `retire.valid` on each side.
use txhdl::comp::{Clock, DefaultClock, In, Out, Unit};
use txhdl::types::Bit;
use txhdl::{lower, Ports, Trace};

#[derive(Ports)]
pub struct EntryOut {
    pub rs1_ready: Out<Bit>,
}

#[derive(Ports)]
pub struct RetireIn {
    pub valid: In<Bit>,
}

#[derive(Ports)]
pub struct RetireOut {
    pub valid: Out<Bit>,
}

pub struct GlueIn {
    pub go: In<Bit>,
    pub retire: RetireIn,
}

pub struct GlueOut {
    pub int: EntryOut,
    pub ready: Out<Bit>,
    pub retire: RetireOut,
}

#[derive(Trace, Default)]
pub struct Glue {}

#[lower]
impl Unit for Glue {
    async fn run(&mut self, inp: GlueIn, out: GlueOut) {
        loop {
            DefaultClock::rising().await;
            let go = inp.go.get();
            out.ready.set(go);
            out.int.rs1_ready.set(!go);
            out.retire.valid.set(inp.retire.valid.get());
        }
    }
}

#[derive(Trace, Default)]
pub struct Top {
    pub glue: Glue,
}

#[lower]
impl Unit for Top {
    async fn run(
        &mut self,
        (go, back): (In<Bit>, In<Bit>),
        (ready, rs1, retired): (Out<Bit>, Out<Bit>, Out<Bit>),
    ) {
        self.glue
            .run(
                GlueIn {
                    go,
                    retire: RetireIn { valid: back },
                },
                GlueOut {
                    int: EntryOut { rs1_ready: rs1 },
                    ready,
                    retire: RetireOut { valid: retired },
                },
            )
            .await;
    }
}

/// Each of the child's ports is joined to the parent's port the literal
/// gave its field, though `ready` ends `out_int_rs1_ready` and
/// `retire_valid` ends both `inp_retire_valid` and `out_retire_valid`.
#[test]
fn a_field_joins_the_port_of_its_own_side_and_path() {
    let v = Top::lowered("top").verilog();
    for join in [
        ".inp_go(",
        ".inp_retire_valid(",
        ".out_int_rs1_ready(",
        ".out_ready(",
        ".out_retire_valid(",
    ] {
        assert!(v.contains(join), "no `{join}` in:\n{v}");
    }
    let joined = |port: &str| {
        let at = v.find(&format!(".{port}(")).unwrap() + port.len() + 2;
        v[at..].split(')').next().unwrap().trim().to_string()
    };
    for (port, net) in [
        ("inp_go", "go"),
        ("inp_retire_valid", "back"),
        ("out_int_rs1_ready", "rs1"),
        ("out_ready", "ready"),
        ("out_retire_valid", "retired"),
    ] {
        assert_eq!(joined(port), net, "`{port}` joined in:\n{v}");
    }
}
