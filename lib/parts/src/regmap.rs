// SPDX-License-Identifier: Apache-2.0
//! A register map declared once, and everything a peripheral needs
//! from it: the offsets, the read mux, the write enables, and a table
//! the tools read to write the datasheet's register table, a C header
//! and a device tree node (issue 499).
//!
//! Every AXI-Lite peripheral used to write the same machinery by hand:
//! a `select!` over the address's word bits for the read, a `wsel == k`
//! guard for every register a write reaches, and the map stated again
//! in its comment, its datasheet, its driver and its device tree. The
//! declaration below is the one place the map is said:
//!
//! ```ignore
//! regmap! { knobs (knobs_read, knobs_we), 2: [
//!     (0, id, ro, "who this is"),
//!     (1, ctrl, rw, "bit 0 runs the counter"),
//!     (2, count, ro, "cycles since the run bit rose"),
//! ] }
//! ```
//!
//! The first name is the map's, and the two in brackets name the read
//! mux and the write enables; the number is how many address bits
//! select a word, taken above the two byte bits; each register is its
//! word index, its name, its access and a sentence. From that the
//! macro writes:
//!
//! * a module `knobs` with a constant per register, `knobs::id`,
//!   `knobs::ctrl` and so on, the byte offset a program uses;
//! * `knobs_read(sel, id, ctrl, count) -> U<32>`: the word a read at
//!   `sel` answers, the registers given in declaration order, and zero
//!   for a word that is not named;
//! * `knobs_we(wgo, sel) -> U<N>`: a bit a register, set when a write
//!   is going and its word is this one, in declaration order from bit
//!   0, so `with!` guards on `we.bit(k)`;
//! * `knobs::MAP`, the table, which `//tools/regmap` turns into a C
//!   header or a device tree node and `//tools/datasheet` into the
//!   sheet's register table.
//!
//! The two functions are ordinary Rust for the run, and the lowering
//! knows the declaration too: it reads `regmap!` in the file as it
//! reads a `#[lower] fn`, and inlines the same read mux and enables
//! into the netlist, so the map a program sees and the map the
//! hardware decodes are one text.
use std::fmt::Write;

/// What a register allows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    /// Read and written.
    Rw,
    /// Read only: a write is answered and does nothing.
    Ro,
    /// Write only: a read answers zero.
    Wo,
    /// Read, and a written one clears the bit.
    W1c,
}

impl Access {
    /// The access as a sheet or a header says it.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Rw => "read, write",
            Access::Ro => "read only",
            Access::Wo => "write only",
            Access::W1c => "read, write one to clear",
        }
    }
}

/// One register of a map.
#[derive(Clone, Copy, Debug)]
pub struct Reg {
    /// Its name, as the declaration has it.
    pub name: &'static str,
    /// Its word index: the byte offset is four times this.
    pub index: u32,
    /// What it allows.
    pub access: Access,
    /// A sentence on what it is.
    pub doc: &'static str,
}

impl Reg {
    /// The byte offset from the peripheral's base.
    pub fn offset(&self) -> u32 {
        self.index * 4
    }
}

/// A map: its name, how many address bits select a word, and its
/// registers in declaration order.
#[derive(Clone, Copy, Debug)]
pub struct RegMap {
    /// The map's name, lower case, as the declaration has it.
    pub name: &'static str,
    /// Address bits above the two byte bits that select a word.
    pub sel_bits: u32,
    /// The registers, in declaration order.
    pub regs: &'static [Reg],
}

impl RegMap {
    /// Bytes the map spans: every word its select bits can name.
    pub fn span(&self) -> u32 {
        4 << self.sel_bits
    }

    /// The map as a C header for the peripheral called `name`: a
    /// define per register with its offset, the sentence beside it,
    /// and the span.
    pub fn c_header(&self, name: &str) -> String {
        let up = name.to_uppercase();
        let mut s = String::new();
        let _ = writeln!(
            s,
            "/* The {name} register map, written by //tools/regmap from"
        );
        let _ = writeln!(s, " * its declaration; edit that and not this. */");
        let _ = writeln!(s, "#ifndef {up}_REGS_H");
        let _ = writeln!(s, "#define {up}_REGS_H");
        let _ = writeln!(s);
        let _ = writeln!(s, "#define {up}_SPAN 0x{:x}", self.span());
        for r in self.regs {
            let _ = writeln!(
                s,
                "#define {up}_{} 0x{:02x} /* {}: {} */",
                r.name.to_uppercase(),
                r.offset(),
                r.access.as_str(),
                r.doc
            );
        }
        let _ = writeln!(s);
        let _ = writeln!(s, "#endif");
        s
    }

    /// The map as a device tree node for the peripheral called `name`
    /// at `base`, with `compatible` as the driver's binding names it.
    pub fn dts_node(&self, name: &str, base: u32, compatible: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "{name}0: {name}@{base:x} {{");
        let _ = writeln!(s, "\tcompatible = \"{compatible}\";");
        let _ = writeln!(s, "\treg = <0x{:08x} 0x{:x}>;", base, self.span());
        let _ = writeln!(s, "\tstatus = \"okay\";");
        let _ = writeln!(s, "}};");
        s
    }

    /// The map as the rows of the datasheet's register table: the
    /// offset, the name, the access and the sentence, each a row.
    pub fn tex_rows(&self) -> Vec<String> {
        self.regs
            .iter()
            .map(|r| {
                format!(
                    "\\code{{0x{:02x}}} & \\code{{{}}} & {} & {} \\\\",
                    r.offset(),
                    r.name.replace('_', "\\_"),
                    r.access.as_str(),
                    r.doc.replace('_', "\\_").replace('&', "\\&")
                )
            })
            .collect()
    }
}

/// The access an entry names, as the macro reads it.
#[doc(hidden)]
#[macro_export]
macro_rules! regmap_access {
    (rw) => {
        $crate::regmap::Access::Rw
    };
    (ro) => {
        $crate::regmap::Access::Ro
    };
    (wo) => {
        $crate::regmap::Access::Wo
    };
    (w1c) => {
        $crate::regmap::Access::W1c
    };
}

/// How many names are given.
#[doc(hidden)]
#[macro_export]
macro_rules! regmap_count {
    () => { 0usize };
    ($x:ident $( $rest:ident )*) => {
        1usize + $crate::regmap_count!($( $rest )*)
    };
}

/// A register map declared once: a module of constants and the table,
/// the read mux and the write enables. See the module's documentation.
#[macro_export]
macro_rules! regmap {
    ($name:ident ($read:ident, $we:ident), $w:literal : [
        $( ($idx:literal, $reg:ident, $acc:ident, $doc:literal) ),+ $(,)?
    ]) => {
        #[doc = concat!(
            "The `",
            stringify!($name),
            "` register map: its offsets and its table."
        )]
        #[allow(non_upper_case_globals)]
        pub mod $name {
            $(
                #[doc = concat!(
                    "The byte offset of `", stringify!($reg), "`: ", $doc, "."
                )]
                #[allow(non_upper_case_globals)]
                pub const $reg: u32 = $idx * 4;
            )+

            /// The map, for the tools.
            pub static MAP: $crate::regmap::RegMap = $crate::regmap::RegMap {
                name: stringify!($name),
                sel_bits: $w,
                regs: &[ $( $crate::regmap::Reg {
                    name: stringify!($reg),
                    index: $idx,
                    access: $crate::regmap_access!($acc),
                    doc: $doc,
                } ),+ ],
            };
        }

        /// The word a read at `sel` answers: the registers in
        /// declaration order, and zero for a word not named.
        #[allow(clippy::too_many_arguments)]
        pub fn $read(
            sel: ::txhdl::types::U<$w>,
            $( $reg: ::txhdl::types::U<32> ),+
        ) -> ::txhdl::types::U<32> {
            ::txhdl::select!(sel.raw() => {
                $( $idx => $reg, )+
                _ => ::txhdl::types::U::<32>::from(0u8),
            })
        }

        /// A bit a register, set when a write is going and its word
        /// is this one, in declaration order from bit 0.
        pub fn $we(
            wgo: ::txhdl::types::Bit,
            sel: ::txhdl::types::U<$w>,
        ) -> ::txhdl::types::U<{ $crate::regmap_count!($( $reg )+) }> {
            let mut bits = 0u128;
            let mut k = 0u32;
            $(
                if (wgo & (sel == $idx)).to_bool() {
                    bits |= 1u128 << k;
                }
                k += 1;
            )+
            let _ = k;
            ::txhdl::types::U::from(bits)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    regmap! { knobs (knobs_read, knobs_we), 2: [
        (0, id, ro, "who this is"),
        (1, ctrl, rw, "bit 0 runs the counter"),
        (2, count, ro, "cycles since the run bit rose"),
    ] }

    #[test]
    fn the_declaration_writes_the_offsets_the_table_and_the_helpers() {
        use txhdl::types::{Bit, U};
        assert_eq!(knobs::id, 0);
        assert_eq!(knobs::ctrl, 4);
        assert_eq!(knobs::count, 8);
        assert_eq!(knobs::MAP.regs.len(), 3);
        assert_eq!(knobs::MAP.regs[1].name, "ctrl");
        assert_eq!(knobs::MAP.regs[1].access, Access::Rw);
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
        let h = knobs::MAP.c_header("knobs");
        assert!(
            h.contains(
                "#define KNOBS_CTRL 0x04 /* read, write: bit 0 runs the \
                 counter */"
            ),
            "{h}"
        );
        assert!(h.contains("#define KNOBS_SPAN 0x10"), "{h}");
        let d =
            knobs::MAP.dts_node("knobs", 0x3600, "hdlfactory,vreteno-knobs");
        assert!(d.contains("knobs0: knobs@3600 {"), "{d}");
        assert!(d.contains("reg = <0x00003600 0x10>;"), "{d}");
        assert_eq!(knobs::MAP.tex_rows().len(), 3);
    }
}
