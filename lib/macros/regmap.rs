// SPDX-License-Identifier: Apache-2.0
//! `regmap!`: a register map declared once, and everything a
//! peripheral needs from it (issues 499 and 569).
//!
//! ```ignore
//! regmap! { knobs (knobs_read, knobs_we), 2: [
//!     (0, id, ro, "who this is"),
//!     (1, ctrl, rw, "the control word", [
//!         (run, 0, 1, rw, 0, "runs the counter"),
//!         (rate, 4, 4, rw, 1, "cycles a count, less one"),
//!     ]),
//!     (2, count, ro, "cycles since the run bit rose"),
//! ] }
//! ```
//!
//! The first name is the map's, and the two in brackets name the read
//! mux and the write enables, with a third, the read enables, when a
//! register's access is `rc`; the number is how many address bits
//! select a word, above the two byte bits. A register is its word
//! index, its name, its access and a sentence, and may carry its
//! fields, each a name, its low bit, its width, its access, its reset
//! value and a sentence. The macro writes:
//!
//! * a module named after the map: a constant a register, the byte
//!   offset a program addresses by (`knobs::ctrl`); a constant a
//!   field, a `Field` with its shift, its width and its reset
//!   (`knobs::ctrl_run`), which gets and sets it in a word; and `MAP`,
//!   the table the tools read;
//! * `knobs_read(sel, id, ctrl, count)`: the word a read at `sel`
//!   answers, the registers in declaration order, zero for a word not
//!   named;
//! * `knobs_we(wgo, sel)`: a bit a register, set when a write is going
//!   and its word is this one, in declaration order from bit 0;
//! * with a third name, `knobs_re(rgo, sel)`: the same for a read, so a
//!   register a read takes, `rc`, knows when to take it;
//! * a function a field, `knobs_ctrl_run(w)`, the field's value out of
//!   a written word, a `Bit` for one bit and a `U<width>` otherwise;
//! * a function a register with fields, `knobs_ctrl_pack(run, rate)`,
//!   the fields packed into a word for the read mux, with zeros where
//!   nothing is named.
//!
//! Every one of those functions is ordinary Rust for the run, and the
//! lowering reads the same declaration in the file, through
//! [`helpers`], to inline the same functions into the netlist.
use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

use crate::{split_commas, Helper};

/// A field of a register, as declared.
pub(crate) struct FieldDecl {
    pub name: String,
    pub lo: u32,
    pub width: u32,
    pub access: String,
    pub reset: String,
    pub doc: String,
}

/// A register, as declared.
pub(crate) struct RegDecl {
    pub idx: u32,
    pub name: String,
    pub access: String,
    pub doc: String,
    pub fields: Vec<FieldDecl>,
}

/// The declaration.
pub(crate) struct Decl {
    pub name: String,
    pub read: String,
    pub we: String,
    /// The read enables' name, if the map has a register a read acts
    /// on (`rc`).
    pub re: Option<String>,
    pub sel_bits: u32,
    pub regs: Vec<RegDecl>,
}

fn ident(t: Option<&Vec<TokenTree>>, what: &str) -> Result<String, String> {
    match t.map(|v| v.as_slice()) {
        Some([TokenTree::Ident(i)]) => Ok(i.to_string()),
        _ => Err(format!("regmap!: expected {what}")),
    }
}

fn number(t: Option<&Vec<TokenTree>>, what: &str) -> Result<u32, String> {
    match t.map(|v| v.as_slice()) {
        Some([TokenTree::Literal(l)]) => {
            let s = l.to_string().replace('_', "");
            let v = if let Some(h) = s.strip_prefix("0x") {
                u32::from_str_radix(h, 16).ok()
            } else {
                s.parse().ok()
            };
            v.ok_or_else(|| format!("regmap!: {what} is not a number: {s}"))
        }
        _ => Err(format!("regmap!: expected {what}")),
    }
}

fn text(t: Option<&Vec<TokenTree>>, what: &str) -> Result<String, String> {
    match t.map(|v| v.as_slice()) {
        Some([TokenTree::Literal(l)]) => Ok(l.to_string()),
        _ => Err(format!("regmap!: expected {what}")),
    }
}

fn access(t: Option<&Vec<TokenTree>>) -> Result<String, String> {
    let a = ident(t, "an access, `rw`, `ro`, `wo`, `w1c` or `rc`")?;
    match a.as_str() {
        "rw" => Ok("Rw".into()),
        "ro" => Ok("Ro".into()),
        "wo" => Ok("Wo".into()),
        "w1c" => Ok("W1c".into()),
        "rc" => Ok("Rc".into()),
        _ => Err(format!("regmap!: `{a}` is not an access")),
    }
}

/// The declaration's tokens, the ones inside `regmap! { .. }`, read.
pub(crate) fn parse(ts: &[TokenTree]) -> Result<Decl, String> {
    // `name ( read , we ) , w : [ .. ]`
    let name = match ts.first() {
        Some(TokenTree::Ident(i)) => i.to_string(),
        _ => return Err("regmap!: expected the map's name".into()),
    };
    let names = match ts.get(1) {
        Some(TokenTree::Group(g))
            if g.delimiter() == Delimiter::Parenthesis =>
        {
            split_commas(g)
        }
        _ => return Err("regmap!: expected `(read, we)` after the name".into()),
    };
    let read = ident(names.first(), "the read function's name")?;
    let we = ident(names.get(1), "the write enables' name")?;
    let re = match names.get(2) {
        Some(_) => Some(ident(names.get(2), "the read enables' name")?),
        None => None,
    };
    let sel_bits = number(
        Some(&vec![ts
            .get(3)
            .cloned()
            .ok_or("regmap!: expected the select width")?]),
        "the select width",
    )?;
    let Some(TokenTree::Group(entries)) = ts.iter().find(|t| {
        matches!(t, TokenTree::Group(e) if e.delimiter() == Delimiter::Bracket)
    }) else {
        return Err("regmap!: expected `[ .. ]` of registers".into());
    };
    let mut regs = Vec::new();
    for e in split_commas(entries) {
        let Some(TokenTree::Group(e)) = e.first() else {
            return Err("regmap!: a register is `( .. )`".into());
        };
        let p = split_commas(e);
        let idx = number(p.first(), "a register's word index")?;
        let rname = ident(p.get(1), "a register's name")?;
        let racc = access(p.get(2))?;
        let doc = text(p.get(3), "a register's sentence")?;
        let mut fields = Vec::new();
        if let Some(fl) = p.get(4) {
            let Some(TokenTree::Group(fl)) = fl.first() else {
                return Err(format!(
                    "regmap!: `{rname}`'s fields are `[ .. ]`"
                ));
            };
            for f in split_commas(fl) {
                let Some(TokenTree::Group(f)) = f.first() else {
                    return Err("regmap!: a field is `( .. )`".into());
                };
                let q = split_commas(f);
                let fname = ident(q.first(), "a field's name")?;
                let lo = number(q.get(1), "a field's low bit")?;
                let width = number(q.get(2), "a field's width")?;
                let facc = access(q.get(3))?;
                let reset = text(q.get(4), "a field's reset value")?;
                let fdoc = text(q.get(5), "a field's sentence")?;
                if width == 0 || lo + width > 32 {
                    return Err(format!(
                        "regmap!: field `{fname}` of `{rname}` does not fit a word"
                    ));
                }
                fields.push(FieldDecl {
                    name: fname,
                    lo,
                    width,
                    access: facc,
                    reset,
                    doc: fdoc,
                });
            }
        }
        regs.push(RegDecl {
            idx,
            name: rname,
            access: racc,
            doc,
            fields,
        });
    }
    if regs.is_empty() {
        return Err("regmap!: a map needs a register".into());
    }
    Ok(Decl {
        name,
        read,
        we,
        re,
        sel_bits,
        regs,
    })
}

/// A field's type in a function: a `Bit` for one bit, a `U<n>` above.
fn ftype(f: &FieldDecl) -> String {
    if f.width == 1 {
        "::txhdl::types::Bit".into()
    } else {
        format!("::txhdl::types::U<{}>", f.width)
    }
}

/// The Rust the declaration stands for.
pub(crate) fn expand(d: &Decl) -> String {
    let mut s = String::new();
    let w = d.sel_bits;
    // The module: offsets, fields, the table.
    s.push_str(&format!(
        "/// The `{0}` register map: its offsets, its fields and its table.\n\
         #[allow(non_upper_case_globals)]\npub mod {0} {{\n",
        d.name
    ));
    for r in &d.regs {
        s.push_str(&format!(
            "    /// The byte offset of `{0}`: {1}.\n    pub const {0}: u32 = {2};\n",
            r.name,
            r.doc.trim_matches('"'),
            r.idx * 4
        ));
        for f in &r.fields {
            s.push_str(&format!(
                "    /// `{0}`'s field `{1}`: {2}.\n    pub const {0}_{1}: ::txhdl::regmap::Field = \
                 ::txhdl::regmap::Field {{ shift: {3}, width: {4}, reset: {5} }};\n",
                r.name,
                f.name,
                f.doc.trim_matches('"'),
                f.lo,
                f.width,
                f.reset
            ));
        }
    }
    s.push_str("    /// The map, for the tools.\n    pub static MAP: ::txhdl::regmap::RegMap = ::txhdl::regmap::RegMap {\n");
    s.push_str(&format!(
        "        name: \"{}\",\n        sel_bits: {w},\n        regs: &[\n",
        d.name
    ));
    for r in &d.regs {
        s.push_str(&format!(
            "            ::txhdl::regmap::Reg {{ name: \"{}\", index: {}, access: \
             ::txhdl::regmap::Access::{}, doc: {}, fields: &[\n",
            r.name, r.idx, r.access, r.doc
        ));
        for f in &r.fields {
            s.push_str(&format!(
                "                ::txhdl::regmap::FieldInfo {{ name: \"{}\", field: \
                 ::txhdl::regmap::Field {{ shift: {}, width: {}, reset: {} }}, \
                 access: ::txhdl::regmap::Access::{}, doc: {} }},\n",
                f.name, f.lo, f.width, f.reset, f.access, f.doc
            ));
        }
        s.push_str("            ] },\n");
    }
    s.push_str("        ],\n    };\n}\n\n");
    // The read mux.
    s.push_str(&format!(
        "/// The word a read at `sel` answers: the registers in declaration\n\
         /// order, and zero for a word not named.\n\
         #[allow(clippy::too_many_arguments)]\npub fn {}(sel: ::txhdl::types::U<{w}>",
        d.read
    ));
    for r in &d.regs {
        s.push_str(&format!(", {}: ::txhdl::types::U<32>", r.name));
    }
    s.push_str(
        ") -> ::txhdl::types::U<32> {\n    ::txhdl::select!(sel.raw() => {\n",
    );
    for r in &d.regs {
        s.push_str(&format!("        {} => {},\n", r.idx, r.name));
    }
    s.push_str(
        "        _ => ::txhdl::types::U::<32>::from(0u8),\n    })\n}\n\n",
    );
    // The write enables.
    let n = d.regs.len();
    s.push_str(&format!(
        "/// A bit a register, set when a write is going and its word is this\n\
         /// one, in declaration order from bit 0.\n\
         pub fn {}(wgo: ::txhdl::types::Bit, sel: ::txhdl::types::U<{w}>) -> ::txhdl::types::U<{n}> {{\n\
         \x20   let mut bits = 0u128;\n",
        d.we
    ));
    for (k, r) in d.regs.iter().enumerate() {
        s.push_str(&format!(
            "    if (wgo & (sel == {})).to_bool() {{\n        bits |= 1u128 << {k};\n    }}\n",
            r.idx
        ));
    }
    s.push_str("    ::txhdl::types::U::from(bits)\n}\n\n");
    // The read enables, the same shape, when the map names them: a
    // register a read acts on, `rc`, needs to know the read is going.
    if let Some(re) = &d.re {
        s.push_str(&format!(
            "/// A bit a register, set when a read is going and its word is this\n\
             /// one, in declaration order from bit 0.\n\
             pub fn {re}(rgo: ::txhdl::types::Bit, sel: ::txhdl::types::U<{w}>) -> ::txhdl::types::U<{n}> {{\n\
             \x20   let mut bits = 0u128;\n"
        ));
        for (k, r) in d.regs.iter().enumerate() {
            s.push_str(&format!(
                "    if (rgo & (sel == {})).to_bool() {{\n        bits |= 1u128 << {k};\n    }}\n",
                r.idx
            ));
        }
        s.push_str("    ::txhdl::types::U::from(bits)\n}\n\n");
    }
    // A function a field, and one a register with fields.
    for r in &d.regs {
        for f in &r.fields {
            let body = if f.width == 1 {
                format!("w.bit({})", f.lo)
            } else {
                format!("w.slice::<{}, {}>()", f.lo, f.width)
            };
            s.push_str(&format!(
                "/// `{1}`'s field `{2}` out of a written word.\n\
                 pub fn {0}_{1}_{2}(w: ::txhdl::types::U<32>) -> {3} {{\n    {4}\n}}\n\n",
                d.name,
                r.name,
                f.name,
                ftype(f),
                body
            ));
        }
        if !r.fields.is_empty() {
            s.push_str(&format!(
                "/// `{1}` packed from its fields, zeros where nothing is named.\n\
                 #[allow(clippy::too_many_arguments)]\n\
                 pub fn {0}_{1}_pack(",
                d.name, r.name
            ));
            let args: Vec<String> = r
                .fields
                .iter()
                .map(|f| format!("{}: {}", f.name, ftype(f)))
                .collect();
            s.push_str(&args.join(", "));
            s.push_str(") -> ::txhdl::types::U<32> {\n    ");
            let terms: Vec<String> = r
                .fields
                .iter()
                .map(|f| format!("({}.zext::<32>() << {}u32)", f.name, f.lo))
                .collect();
            s.push_str(&terms.join(" | "));
            s.push_str("\n}\n\n");
        }
    }
    s
}

/// The helpers the lowering inlines for a declaration: the same
/// functions [`expand`] writes, as the lowering's own bodies.
pub(crate) fn helpers(d: &Decl) -> Vec<Helper> {
    let mut out = Vec::new();
    let arms: Vec<String> = d
        .regs
        .iter()
        .map(|r| format!("{} => {}", r.idx, r.name))
        .collect();
    let mut params = vec!["sel".to_string()];
    params.extend(d.regs.iter().map(|r| r.name.clone()));
    out.push(Helper {
        name: d.read.clone(),
        params,
        consts: Vec::new(),
        lets: Vec::new(),
        value: format!(
            "select!(sel.raw() => {{ {}, _ => U::<32>::from(0u8) }})",
            arms.join(", ")
        ),
        refused: Vec::new(),
    });
    let lets: Vec<(String, String)> = d
        .regs
        .iter()
        .enumerate()
        .map(|(k, r)| (format!("we_{k}"), format!("wgo & (sel == {})", r.idx)))
        .collect();
    let n = d.regs.len();
    let mut value = format!("we_{}.zext::<1>()", n - 1);
    for k in (0..n - 1).rev() {
        value = format!("{value}.concat::<1, {}>(we_{k}.zext::<1>())", n - k);
    }
    out.push(Helper {
        name: d.we.clone(),
        params: vec!["wgo".to_string(), "sel".to_string()],
        consts: Vec::new(),
        lets,
        value: value.clone(),
        refused: Vec::new(),
    });
    // The read enables are the write enables' shape over `rgo`.
    if let Some(re) = &d.re {
        let lets: Vec<(String, String)> = d
            .regs
            .iter()
            .enumerate()
            .map(|(k, r)| {
                (format!("we_{k}"), format!("rgo & (sel == {})", r.idx))
            })
            .collect();
        out.push(Helper {
            name: re.clone(),
            params: vec!["rgo".to_string(), "sel".to_string()],
            consts: Vec::new(),
            lets,
            value,
            refused: Vec::new(),
        });
    }
    for r in &d.regs {
        for f in &r.fields {
            let value = if f.width == 1 {
                format!("w.bit({})", f.lo)
            } else {
                format!("w.slice::<{}, {}>()", f.lo, f.width)
            };
            out.push(Helper {
                name: format!("{}_{}_{}", d.name, r.name, f.name),
                params: vec!["w".to_string()],
                consts: Vec::new(),
                lets: Vec::new(),
                value,
                refused: Vec::new(),
            });
        }
        if !r.fields.is_empty() {
            let terms: Vec<String> = r
                .fields
                .iter()
                .map(|f| format!("({}.zext::<32>() << {}u32)", f.name, f.lo))
                .collect();
            out.push(Helper {
                name: format!("{}_{}_pack", d.name, r.name),
                params: r.fields.iter().map(|f| f.name.clone()).collect(),
                consts: Vec::new(),
                lets: Vec::new(),
                value: terms.join(" | "),
                refused: Vec::new(),
            });
        }
    }
    out
}

/// Every `regmap!` declaration among `ts`, the tokens of a file, as
/// the helpers it stands for.
pub(crate) fn helpers_in(ts: &[TokenTree]) -> Vec<Helper> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < ts.len() {
        let is_decl = matches!(&ts[i], TokenTree::Ident(id) if id.to_string() == "regmap")
            && matches!(&ts[i + 1], TokenTree::Punct(p) if p.as_char() == '!')
            && matches!(&ts[i + 2], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace);
        if !is_decl {
            i += 1;
            continue;
        }
        let TokenTree::Group(g) = &ts[i + 2] else {
            unreachable!()
        };
        i += 3;
        let d: Vec<TokenTree> = g.stream().into_iter().collect();
        if let Ok(decl) = parse(&d) {
            out.extend(helpers(&decl));
        }
    }
    out
}

/// The macro itself: the declaration's tokens to the Rust they stand
/// for, or a `compile_error!` saying what is wrong with them.
pub(crate) fn regmap(input: TokenStream) -> TokenStream {
    let ts: Vec<TokenTree> = input.into_iter().collect();
    match parse(&ts) {
        Ok(d) => {
            // Each function's lowering beside it, so a helper that
            // calls one, which is lowered on its own and cannot inline
            // from the file, reaches it by `f::lowered` (issue 697).
            let mut out = expand(&d);
            for h in helpers(&d) {
                if let Some(c) =
                    crate::companion_of(&h, "pub", &[], &Vec::new())
                {
                    out.push_str(&c);
                }
            }
            out
        }
        .parse()
        .unwrap_or_else(|e| {
            crate::err(proc_macro::Span::call_site(), &format!("regmap!: {e}"))
        }),
        Err(m) => crate::err(proc_macro::Span::call_site(), &m),
    }
}

#[allow(dead_code)]
fn _unused(_: Group) {}
