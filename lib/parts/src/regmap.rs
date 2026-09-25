// SPDX-License-Identifier: Apache-2.0
//! The register map declaration, exercised: `regmap!` is the macro in
//! `txhdl`, its data types are `txhdl::regmap`, and this module is the
//! test that the two agree, on a map with fields (issues 499 and 569).
//! The peripherals that declare maps are the users; `sd` is the first.

#[cfg(test)]
mod tests {
    use txhdl::regmap;
    use txhdl::regmap::Access;
    use txhdl::types::{Bit, U};

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
}
