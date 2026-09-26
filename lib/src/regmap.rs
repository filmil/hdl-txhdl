// SPDX-License-Identifier: Apache-2.0
//! What a register map declared with `regmap!` stands for at run time:
//! the table the tools read, and the field constants a program
//! addresses by (issues 499 and 569).
//!
//! The declaration itself is the macro in `txhdl_macros`, and what it
//! writes into a peripheral is described there and in the parts. This
//! module is the data: a [`RegMap`] of [`Reg`]s, each with its
//! [`FieldInfo`]s, which `//tools/regmap` turns into a C header or a
//! device tree node and `//tools/datasheet` into a sheet's register
//! table; and [`Field`], the constant a program uses to get a field
//! out of a word and to set it in one.
use std::fmt::Write;

/// What a register, or a field of one, allows.
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
    /// Read, and the read itself takes the value: the next read gives
    /// the next one, as a receive FIFO's head does.
    Rc,
}

impl Access {
    /// The access as a sheet or a header says it.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Rw => "read, write",
            Access::Ro => "read only",
            Access::Wo => "write only",
            Access::W1c => "read, write one to clear",
            Access::Rc => "read; the read takes it",
        }
    }
}

/// A field of a register, as a program uses it: where it sits in the
/// word and how wide it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Field {
    /// The field's low bit.
    pub shift: u32,
    /// Its width in bits.
    pub width: u32,
    /// Its value after reset.
    pub reset: u32,
}

impl Field {
    /// The field's mask, in place in the word.
    pub const fn mask(self) -> u32 {
        let ones = if self.width >= 32 {
            u32::MAX
        } else {
            (1u32 << self.width) - 1
        };
        ones << self.shift
    }

    /// The field's value out of `word`.
    pub const fn get(self, word: u32) -> u32 {
        (word & self.mask()) >> self.shift
    }

    /// `word` with the field set to `value`, the rest kept.
    pub const fn set(self, word: u32, value: u32) -> u32 {
        (word & !self.mask()) | ((value << self.shift) & self.mask())
    }

    /// The word with only this field set to `value`: what a program
    /// writes to a register whose other fields it does not touch.
    pub const fn with(self, value: u32) -> u32 {
        self.set(0, value)
    }
}

/// One field of a register, in the table.
#[derive(Clone, Copy, Debug)]
pub struct FieldInfo {
    /// Its name, as the declaration has it.
    pub name: &'static str,
    /// Where it sits and how wide it is.
    pub field: Field,
    /// What it allows.
    pub access: Access,
    /// A sentence on what it is.
    pub doc: &'static str,
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
    /// Its fields, in declaration order; none for a whole register.
    pub fields: &'static [FieldInfo],
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
    /// define per register with its offset and, per field, its shift,
    /// its mask in place, its width and its reset value, the sentence
    /// beside each, and the span.
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
            let reg = format!("{up}_{}", r.name.to_uppercase());
            let _ = writeln!(
                s,
                "#define {reg} 0x{:02x} /* {}: {} */",
                r.offset(),
                r.access.as_str(),
                r.doc
            );
            for f in r.fields {
                let fname = format!("{reg}_{}", f.name.to_uppercase());
                let _ = writeln!(
                    s,
                    "#define {fname}_SHIFT {} /* {}: {} */",
                    f.field.shift,
                    f.access.as_str(),
                    f.doc
                );
                let _ =
                    writeln!(s, "#define {fname}_MASK 0x{:x}", f.field.mask());
                let _ = writeln!(s, "#define {fname}_WIDTH {}", f.field.width);
                let _ =
                    writeln!(s, "#define {fname}_RESET 0x{:x}", f.field.reset);
            }
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

    /// The map as the rows of the datasheet's register table: a row a
    /// register with its offset, its name, its access and its
    /// sentence, and under it a row a field with its bits.
    pub fn tex_rows(&self) -> Vec<String> {
        let esc = |t: &str| t.replace('_', "\\_").replace('&', "\\&");
        let mut rows = Vec::new();
        for r in self.regs {
            rows.push(format!(
                "\\code{{0x{:02x}}} & \\code{{{}}} & {} & {} \\\\",
                r.offset(),
                esc(r.name),
                r.access.as_str(),
                esc(r.doc)
            ));
            for f in r.fields {
                let bits = if f.field.width == 1 {
                    format!("bit {}", f.field.shift)
                } else {
                    format!(
                        "bits {} to {}",
                        f.field.shift + f.field.width - 1,
                        f.field.shift
                    )
                };
                rows.push(format!(
                    "\\quad {bits} & \\quad\\code{{{}}} & {} & {} \\\\",
                    esc(f.name),
                    f.access.as_str(),
                    esc(f.doc)
                ));
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIV: Field = Field {
        shift: 0,
        width: 8,
        reset: 0,
    };
    const WIDE: Field = Field {
        shift: 8,
        width: 1,
        reset: 0,
    };

    #[test]
    fn a_field_gets_sets_and_masks() {
        assert_eq!(DIV.mask(), 0xff);
        assert_eq!(WIDE.mask(), 0x100);
        assert_eq!(DIV.get(0x0000_017b), 0x7b);
        assert_eq!(WIDE.get(0x0000_017b), 1);
        assert_eq!(DIV.set(0x0000_0100, 5), 0x0000_0105);
        assert_eq!(WIDE.set(0x0000_0105, 0), 0x0000_0005);
        assert_eq!(WIDE.with(1), 0x100);
    }

    static FIELDS: [FieldInfo; 2] = [
        FieldInfo {
            name: "div",
            field: DIV,
            access: Access::Rw,
            doc: "cycles a half clock, less one",
        },
        FieldInfo {
            name: "wide",
            field: WIDE,
            access: Access::Rw,
            doc: "four lines",
        },
    ];
    static MAP: RegMap = RegMap {
        name: "sd",
        sel_bits: 4,
        regs: &[
            Reg {
                name: "ctrl",
                index: 0,
                access: Access::Rw,
                doc: "the control word",
                fields: &FIELDS,
            },
            Reg {
                name: "arg",
                index: 2,
                access: Access::Rw,
                doc: "the argument",
                fields: &[],
            },
        ],
    };

    #[test]
    fn the_header_the_node_and_the_rows_say_the_fields() {
        let h = MAP.c_header("sd");
        assert!(
            h.contains(
                "#define SD_CTRL 0x00 /* read, write: the control word */"
            ),
            "{h}"
        );
        assert!(h.contains("#define SD_CTRL_DIV_SHIFT 0 /* read, write: cycles a half clock, less one */"), "{h}");
        assert!(h.contains("#define SD_CTRL_DIV_MASK 0xff"), "{h}");
        assert!(h.contains("#define SD_CTRL_WIDE_MASK 0x100"), "{h}");
        assert!(h.contains("#define SD_CTRL_WIDE_WIDTH 1"), "{h}");
        assert!(h.contains("#define SD_ARG 0x08"), "{h}");
        assert!(h.contains("#define SD_SPAN 0x40"), "{h}");
        let d = MAP.dts_node("sd", 0x3600, "hdlfactory,vreteno-sd");
        assert!(d.contains("sd0: sd@3600 {"), "{d}");
        assert!(d.contains("reg = <0x00003600 0x40>;"), "{d}");
        let rows = MAP.tex_rows();
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert!(
            rows[1].starts_with("\\quad bits 7 to 0 & \\quad\\code{div}"),
            "{}",
            rows[1]
        );
        assert!(
            rows[2].starts_with("\\quad bit 8 & \\quad\\code{wide}"),
            "{}",
            rows[2]
        );
    }
}
