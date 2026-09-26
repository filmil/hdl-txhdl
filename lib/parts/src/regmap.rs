// SPDX-License-Identifier: Apache-2.0
//! The register map declaration, exercised: `regmap!` is the macro in
//! `txhdl`, its data types are `txhdl::regmap`, and this module is the
//! test that the two agree, on a map with fields (issues 499 and 569).
//! The peripherals that declare maps are the users; `sd` is the first.

#[cfg(test)]
mod tests {
    use txhdl::netlist::Expr;
    use txhdl::regmap::Access;
    use txhdl::types::{Bit, U};
    use txhdl::{lower, regmap};

    regmap! { knobs (knobs_read, knobs_we), 2: [
        (0, id, ro, "who this is"),
        (1, ctrl, rw, "the control word", [
            (run, 0, 1, rw, 0, "runs the counter"),
            (step, 4, 4, rw, 1, "what a count adds"),
        ]),
        (2, count, ro, "the count"),
    ] }

    #[test]
    fn the_declaration_writes_the_offsets_the_table_and_the_helpers() {
        assert_eq!(knobs::id, 0);
        assert_eq!(knobs::ctrl, 4);
        assert_eq!(knobs::count, 8);
        assert_eq!(knobs::MAP.regs.len(), 3);
        assert_eq!(knobs::MAP.regs[1].name, "ctrl");
        assert_eq!(knobs::MAP.regs[1].access, Access::Rw);
        assert_eq!(knobs::MAP.regs[1].fields.len(), 2);
        assert_eq!(knobs::MAP.span(), 16);
        let word = |v: u32| U::<32>::from(v);
        let read = |s: u32| {
            knobs_read(U::<2>::from(s), word(0x1d), word(1), word(77)).raw()
        };
        assert_eq!(read(0), 0x1d);
        assert_eq!(read(1), 1);
        assert_eq!(read(2), 77);
        assert_eq!(read(3), 0, "a word not named reads as zero");
        let we = |go: bool, s: u32| {
            knobs_we(Bit::from_bool(go), U::<2>::from(s)).raw()
        };
        assert_eq!(we(true, 1), 0b010);
        assert_eq!(we(true, 2), 0b100);
        assert_eq!(we(false, 1), 0);
        assert_eq!(we(true, 3), 0, "a word not named enables nothing");
    }

    #[test]
    fn the_fields_come_out_of_a_word_and_go_back_in() {
        assert_eq!(knobs::ctrl_run.mask(), 1);
        assert_eq!(knobs::ctrl_step.mask(), 0xf0);
        assert_eq!(knobs::ctrl_step.reset, 1);
        let w = U::<32>::from(0x31u32);
        assert_eq!(knobs_ctrl_run(w), Bit::One);
        assert_eq!(knobs_ctrl_step(w).raw(), 3);
        let packed = knobs_ctrl_pack(Bit::One, U::<4>::from(3u8));
        assert_eq!(packed.raw(), 0x31, "the fields pack where they sit");
        assert_eq!(knobs::ctrl_step.set(0x31, 5), 0x51);
        assert_eq!(knobs::ctrl_step.get(0x51), 5);
        let h = knobs::MAP.c_header("knobs");
        assert!(h.contains("#define KNOBS_CTRL_STEP_SHIFT 4"), "{h}");
        assert!(h.contains("#define KNOBS_CTRL_STEP_MASK 0xf0"), "{h}");
        assert!(h.contains("#define KNOBS_CTRL_STEP_RESET 0x1"), "{h}");
        assert_eq!(
            knobs::MAP.tex_rows().len(),
            5,
            "three registers, two fields"
        );
    }

    /// A helper that calls the functions the declaration writes. It is
    /// lowered on its own, apart from any unit, so it reaches them
    /// through the lowerings `regmap!` writes beside them (issue 697).
    #[lower]
    fn run_go(wgo: Bit, wsel: U<2>, w: U<32>) -> Bit {
        let we = knobs_we(wgo, wsel);
        we.bit(1) & knobs_ctrl_run(w)
    }

    #[test]
    fn a_helper_calls_the_functions_the_declaration_writes() {
        let go = |g: bool, s: u32, w: u32| {
            run_go(Bit::from_bool(g), U::<2>::from(s), U::<32>::from(w))
        };
        assert_eq!(go(true, 1, 1), Bit::One, "a write to ctrl with run");
        assert_eq!(go(true, 1, 0), Bit::Zero, "without run");
        assert_eq!(go(true, 2, 1), Bit::Zero, "a write to another word");
        assert_eq!(go(false, 1, 1), Bit::Zero, "no write");
        let n = |s: &str| Expr::Name(s.to_string());
        let e = format!("{:?}", run_go::lowered(n("g"), n("s"), n("w")));
        for name in ["\"g\"", "\"s\"", "\"w\""] {
            assert!(e.contains(name), "{name} in {e}");
        }
        let p = format!("{:?}", knobs_ctrl_pack::lowered(n("r"), n("t")));
        assert!(p.contains("\"r\"") && p.contains("\"t\""), "{p}");
    }

    regmap! { fifo (fifo_read, fifo_we, fifo_re), 1: [
        (0, status, ro, "whether a word waits"),
        (1, head, rc, "the oldest word; a read takes it"),
    ] }

    #[test]
    fn a_read_the_register_acts_on_is_enabled_and_the_access_says_so() {
        assert_eq!(fifo::MAP.regs[1].access, Access::Rc);
        assert_eq!(Access::Rc.as_str(), "read; the read takes it");
        let re = |go: bool, s: u32| {
            fifo_re(Bit::from_bool(go), U::<1>::from(s)).raw()
        };
        assert_eq!(re(true, 1), 0b10);
        assert_eq!(re(true, 0), 0b01);
        assert_eq!(re(false, 1), 0);
        let _ = (fifo_read, fifo_we);
    }
}
