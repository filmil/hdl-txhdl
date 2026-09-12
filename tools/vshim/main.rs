// SPDX-License-Identifier: Apache-2.0
//! A Verilog module as a unit: from the module's port list, the C shim
//! over its Verilated model, and the Rust that makes a `Unit` of it,
//! for `verilog_unit()` in `lib/foreign.bzl`.
//!
//! The ports are read from the module header, ANSI style: `input`
//! or `output`, an optional `wire`, `reg` or `logic`, an optional
//! `signed`, an optional `[hi:lo]`, and the name. One port is the
//! clock, which the shim pulses; a port a bit wide is a `Bit`, a wider
//! one a `U<W>`; three ports `x_data`, `x_valid` and `x_ready` are a
//! channel `x`, into the module when the data comes in and out of it
//! when the data goes out.
//!
//! A VHDL entity goes the same way with nvc as the engine, run as a
//! child process: `--vhdl-tb` writes a testbench around the entity
//! that reads a line per step from its standard input, a command and
//! the inputs as hex, and writes the outputs back as one line, and
//! `--vhdl-unit` writes the unit, which drives that testbench through
//! `Cosim`.
//!
//! Usage: vshim MODULE.v TOP CLOCK NAME SHIM.cc UNIT.rs
//!        vshim --vhdl-tb MODULE.vhd ENTITY CLOCK NAME TB.vhd
//!        vshim --vhdl-unit MODULE.vhd ENTITY CLOCK NAME SCRIPT UNIT.rs
//! where SCRIPT is the path, under the main repository's runfiles, of
//! the script that runs nvc on the testbench.

struct Port {
    name: String,
    input: bool,
    width: usize,
    /// The VHDL type, as declared, for the testbench's signals.
    vtype: String,
}

/// What runs the module: Verilator in process, or nvc as a child.
enum Engine {
    Verilator,
    Nvc(String),
}

enum Item {
    Wire(Port),
    /// A channel: its stem, the data width, and whether it comes into
    /// the module.
    Chan(String, usize, bool),
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a[1] == "--vhdl-tb" || a[1] == "--vhdl-unit" {
        let (src, top, clock, name) = (&a[2], &a[3], &a[4], &a[5]);
        let text = std::fs::read_to_string(src).expect("read the entity");
        let ports: Vec<Port> = parse_vhdl_ports(&text, top)
            .into_iter()
            .filter(|p| p.name != *clock)
            .collect();
        if a[1] == "--vhdl-tb" {
            std::fs::write(&a[6], testbench(&ports, top, clock, name))
                .expect("testbench");
        } else {
            let items = group(&ports);
            let engine = Engine::Nvc(a[6].clone());
            std::fs::write(&a[7], unit(&ports, &items, top, name, &engine))
                .expect("unit");
        }
        return;
    }
    let (src, top, clock, name) = (&a[1], &a[2], &a[3], &a[4]);
    let (shim_out, rs_out) = (&a[5], &a[6]);
    let text = std::fs::read_to_string(src).expect("read the module");
    let ports = parse_ports(&text, top);
    let ports: Vec<Port> =
        ports.into_iter().filter(|p| p.name != *clock).collect();
    let items = group(&ports);
    std::fs::write(shim_out, shim(&ports, top, clock, name)).expect("shim");
    let unit = unit(&ports, &items, top, name, &Engine::Verilator);
    std::fs::write(rs_out, unit).expect("unit");
}

/// The entity's ports, in order: `name : in|out type`, the width from
/// `(hi downto lo)`, a bit otherwise.
fn parse_vhdl_ports(text: &str, entity: &str) -> Vec<Port> {
    let lower = text.to_ascii_lowercase();
    let at = lower
        .find(&format!("entity {} is", entity.to_ascii_lowercase()))
        .unwrap_or_else(|| panic!("no `entity {entity}` in the file"));
    let rest = &text[at..];
    let open = rest.to_ascii_lowercase().find("port").expect("the ports");
    let open = open + rest[open..].find('(').expect("the ports' (");
    // The list's own closing paren: the types have parens of their own.
    let mut depth = 0;
    let mut close = open;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    let list = &rest[open + 1..close];
    let mut out = Vec::new();
    for entry in list.split(';') {
        let Some((name, decl)) = entry.split_once(':') else {
            continue;
        };
        let decl = decl.trim();
        let (dir, vtype) = decl.split_once(char::is_whitespace).expect("dir");
        let vtype = vtype.trim().to_string();
        let width = vtype
            .find('(')
            .map(|p| {
                let inner = vtype[p + 1..].trim_end_matches(')');
                let mut it = inner.split_whitespace();
                let hi: usize = it.next().expect("hi").parse().expect("hi");
                let _downto = it.next();
                let lo: usize = it.next().expect("lo").parse().expect("lo");
                hi - lo + 1
            })
            .unwrap_or(1);
        out.push(Port {
            name: name.trim().to_string(),
            input: dir.eq_ignore_ascii_case("in"),
            width,
            vtype,
        });
    }
    out
}

/// The testbench around the entity: one line in per step, the command
/// and the inputs as hex, one line out, the outputs as hex.
fn testbench(ports: &[Port], entity: &str, clock: &str, name: &str) -> String {
    let mut o = String::new();
    o.push_str(&format!(
        "-- Generated by //tools/vshim around entity {entity}: the co-run\n\
         -- testbench, driven a step at a time over its standard streams.\n\
         library ieee;\nuse ieee.std_logic_1164.all;\n\
         use ieee.numeric_std.all;\nuse std.textio.all;\n\n\
         entity {name}_cosim is\nend entity;\n\n\
         architecture sim of {name}_cosim is\n  \
         signal {clock} : std_logic := '0';\n"
    ));
    for p in ports {
        let init = if p.width == 1 {
            "'0'"
        } else {
            "(others => '0')"
        };
        o.push_str(&format!("  signal {} : {} := {init};\n", p.name, p.vtype));
    }
    let maps: Vec<String> = std::iter::once(format!("{clock} => {clock}"))
        .chain(ports.iter().map(|p| format!("{0} => {0}", p.name)))
        .collect();
    o.push_str(&format!(
        "begin\n  uut : entity work.{entity} port map ({});\n\n  \
         drive : process\n    variable l, r : line;\n    \
         variable c : character;\n",
        maps.join(", ")
    ));
    for p in ports.iter().filter(|p| p.input) {
        o.push_str(&format!("    variable v_{} : {};\n", p.name, p.vtype));
    }
    o.push_str(
        "  begin\n    loop\n      readline(input, l);\n      read(l, c);\n \
               \
         if c = 'Q' then\n        exit;\n      end if;\n      \
         if c = 'S' then\n",
    );
    for p in ports.iter().filter(|p| p.input) {
        let rd = if p.width == 1 { "read" } else { "hread" };
        o.push_str(&format!(
            "        {rd}(l, v_{0});\n        {0} <= v_{0};\n",
            p.name
        ));
    }
    o.push_str(&format!(
        "        wait for 1 ps;\n      else\n        {clock} <= '1';\n        \
          wait for 1 ns;\n        {clock} <= '0';\n        wait for 1 ns;\n \
               \
         end if;\n"
    ));
    for p in ports.iter().filter(|p| !p.input) {
        let wr = if p.width == 1 { "write" } else { "hwrite" };
        o.push_str(&format!(
            "      {wr}(r, {});\n      write(r, ' ');\n",
            p.name
        ));
    }
    o.push_str(
        "      writeline(output, r);\n      flush(output);\n    end loop;\n    \
         std.env.finish;\n  end process;\nend architecture;\n",
    );
    o
}

/// The header's ports, in order.
fn parse_ports(text: &str, top: &str) -> Vec<Port> {
    let at = text
        .find(&format!("module {top}"))
        .unwrap_or_else(|| panic!("no `module {top}` in the file"));
    let rest = &text[at..];
    let open = rest.find('(').expect("the port list");
    let close = rest.find(");").expect("the port list's end");
    let list = &rest[open + 1..close];
    let mut out = Vec::new();
    for entry in list.split(',') {
        let toks: Vec<&str> = entry.split_whitespace().collect();
        let Some(dir) =
            toks.iter().position(|t| *t == "input" || *t == "output")
        else {
            continue;
        };
        let input = toks[dir] == "input";
        let name = toks.last().expect("a port name").to_string();
        let width = toks
            .iter()
            .find(|t| t.starts_with('['))
            .map(|t| {
                let t = t.trim_matches(|c| c == '[' || c == ']');
                let (hi, lo) = t.split_once(':').expect("[hi:lo]");
                let (hi, lo): (usize, usize) =
                    (hi.parse().expect("hi"), lo.parse().expect("lo"));
                hi - lo + 1
            })
            .unwrap_or(1);
        out.push(Port {
            name,
            input,
            width,
            vtype: String::new(),
        });
    }
    out
}

/// The ports as the unit sees them: channels where three ports make
/// one, wires otherwise.
fn group(ports: &[Port]) -> Vec<Item> {
    let find = |n: &str| ports.iter().find(|p| p.name == n);
    let mut items = Vec::new();
    let mut taken: Vec<String> = Vec::new();
    for p in ports {
        if taken.contains(&p.name) {
            continue;
        }
        if let Some(stem) = p.name.strip_suffix("_data") {
            let (v, r) = (format!("{stem}_valid"), format!("{stem}_ready"));
            if let (Some(valid), Some(ready)) = (find(&v), find(&r)) {
                if valid.input == p.input && ready.input != p.input {
                    taken.extend([p.name.clone(), v, r]);
                    items.push(Item::Chan(stem.to_string(), p.width, p.input));
                    continue;
                }
            }
        }
        items.push(Item::Wire(Port {
            name: p.name.clone(),
            input: p.input,
            width: p.width,
            vtype: p.vtype.clone(),
        }));
    }
    items
}

/// The Verilator type of a port, by width.
fn ctype(width: usize) -> &'static str {
    match width {
        0..=8 => "CData",
        9..=16 => "SData",
        17..=32 => "IData",
        33..=64 => "QData",
        _ => "wide",
    }
}

fn shim(ports: &[Port], top: &str, clock: &str, name: &str) -> String {
    let mut o = String::new();
    o.push_str(&format!(
        "// Generated by //tools/vshim from module {top}: the C shim.\n\
         #include \"V{top}.h\"\n#include <cstdint>\n\n\
         extern \"C\" {{\n\
         void *{name}_new() {{ return new V{top}; }}\n\
         void {name}_free(void *m) {{ auto t = (V{top} *)m; t->final(); \
         delete t; }}\n\
         void {name}_eval(void *m) {{ ((V{top} *)m)->eval(); }}\n\
         void {name}_edge(void *m) {{ auto t = (V{top} *)m; t->{clock} = 1; \
         t->eval(); t->{clock} = 0; t->eval(); }}\n"
    ));
    o.push_str(&format!(
        "void {name}_set(void *m, uint32_t i, const uint32_t *w, uint32_t n) \
         {{\n  auto t = (V{top} *)m;\n  (void)n;\n  switch (i) {{\n"
    ));
    for (i, p) in ports.iter().enumerate() {
        if !p.input {
            continue;
        }
        let w = p.width;
        if ctype(w) == "wide" {
            let words = w.div_ceil(32);
            o.push_str(&format!(
                "  case {i}: for (uint32_t k = 0; k < {words} && k < n; k++) \
                 t->{}[k] = w[k]; break;\n",
                p.name
            ));
        } else {
            let mask = if w >= 64 {
                "".to_string()
            } else {
                format!(" & ((1ull << {w}) - 1)")
            };
            o.push_str(&format!(
                "  case {i}: t->{} = ({})(((uint64_t)w[0] | ((uint64_t)w[1] \
                 << 32)){mask}); break;\n",
                p.name,
                ctype(w)
            ));
        }
    }
    o.push_str("  default: break;\n  }\n}\n");
    o.push_str(&format!(
        "void {name}_get(void *m, uint32_t i, uint32_t *w, uint32_t n) {{\n  \
         auto t = (V{top} *)m;\n  for (uint32_t k = 0; k < n; k++) w[k] = 0;\n \
           \
         switch (i) {{\n"
    ));
    for (i, p) in ports.iter().enumerate() {
        if p.input {
            continue;
        }
        let w = p.width;
        if ctype(w) == "wide" {
            let words = w.div_ceil(32);
            o.push_str(&format!(
                "  case {i}: for (uint32_t k = 0; k < {words} && k < n; k++) \
                 w[k] = t->{}[k]; break;\n",
                p.name
            ));
        } else {
            o.push_str(&format!(
                "  case {i}: {{ uint64_t v = (uint64_t)t->{}; w[0] = \
                 (uint32_t)v; if (n > 1) w[1] = (uint32_t)(v >> 32); \
                 break; }}\n",
                p.name
            ));
        }
    }
    o.push_str("  default: break;\n  }\n}\n}\n");
    o
}

fn camel(s: &str) -> String {
    let mut o = String::new();
    let mut up = true;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            o.extend(c.to_uppercase());
            up = false;
        } else {
            o.push(c);
        }
    }
    o
}

fn ty(width: usize) -> String {
    if width == 1 {
        "Bit".into()
    } else {
        format!("U<{width}>")
    }
}

fn tuple(xs: &[String]) -> String {
    match xs.len() {
        0 => "()".into(),
        1 => xs[0].clone(),
        _ => format!("({})", xs.join(", ")),
    }
}

fn unit(
    ports: &[Port],
    items: &[Item],
    top: &str,
    name: &str,
    engine: &Engine,
) -> String {
    let idx = |n: &str| ports.iter().position(|p| p.name == n).unwrap();
    let strukt = camel(top);
    // What holds the module: the shim's functions over a Verilated
    // model, or the nvc child behind its script, told each port's
    // width so it can write and read the lines.
    let (model_ty, extern_block, construct) = match engine {
        Engine::Verilator => (
            "Model",
            format!(
                "type H = *mut core::ffi::c_void;\n\n\
                 extern \"C\" {{\n    \
                 fn {name}_new() -> H;\n    \
                 fn {name}_free(m: H);\n    \
                 fn {name}_eval(m: H);\n    \
                 fn {name}_edge(m: H);\n    \
                 fn {name}_set(m: H, i: u32, w: *const u32, n: u32);\n    \
                 fn {name}_get(m: H, i: u32, w: *mut u32, n: u32);\n\
                 }}\n\n"
            ),
            format!(
                "Model::new(Shim {{\n                \
                  new: {name}_new,\n                free: {name}_free,\n \
                                 \
                  eval: {name}_eval,\n                edge: {name}_edge,\n \
                                 \
                  set: {name}_set,\n                get: {name}_get,\n \
                             \
                  }})"
            ),
        ),
        Engine::Nvc(script) => {
            let ins: Vec<String> = ports
                .iter()
                .enumerate()
                .filter(|(_, p)| p.input)
                .map(|(i, p)| format!("({i}, {})", p.width))
                .collect();
            let outs: Vec<String> = ports
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.input)
                .map(|(i, p)| format!("({i}, {})", p.width))
                .collect();
            (
                "Cosim",
                String::new(),
                format!(
                    "Cosim::new(\n                \
                     \"{script}\",\n                \
                     &[{}],\n                &[{}],\n            )",
                    ins.join(", "),
                    outs.join(", ")
                ),
            )
        }
    };
    let runfile_fn = String::new();
    let (mut in_ty, mut in_nm, mut out_ty, mut out_nm) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for it in items {
        match it {
            Item::Wire(p) if p.input => {
                in_ty.push(format!("In<{}>", ty(p.width)));
                in_nm.push(p.name.clone());
            }
            Item::Wire(p) => {
                out_ty.push(format!("Out<{}>", ty(p.width)));
                out_nm.push(p.name.clone());
            }
            Item::Chan(stem, w, true) => {
                in_ty.push(format!("Rx<{}>", ty(*w)));
                in_nm.push(stem.clone());
            }
            Item::Chan(stem, w, false) => {
                out_ty.push(format!("Tx<{}>", ty(*w)));
                out_nm.push(stem.clone());
            }
        }
    }
    let param = |nm: &[String], t: &[String]| -> String {
        if nm.is_empty() {
            "_none: ()".into()
        } else {
            format!("{}: {}", tuple(nm), tuple(t))
        }
    };
    // The step: inputs in, a settle, the transfers the module agrees
    // to, the edge, and the outputs out.
    let mut before = String::new();
    let mut after = String::new();
    let mut outs = String::new();
    let raw_in = |n: &str, w: usize| -> String {
        if w == 1 {
            format!("{n}.get().to_bool() as u128")
        } else {
            format!("{n}.get().raw()")
        }
    };
    let from_raw = |e: &str, w: usize| -> String {
        if w == 1 {
            format!("Bit::from_bool({e} != 0)")
        } else {
            format!("U::<{w}>::new({e})")
        }
    };
    for it in items {
        match it {
            Item::Wire(p) if p.input => before.push_str(&format!(
                "            m.set({}, {});\n",
                idx(&p.name),
                raw_in(&p.name, p.width)
            )),
            Item::Wire(p) => outs.push_str(&format!(
                "            {}.set({});\n",
                p.name,
                from_raw(&format!("m.get({})", idx(&p.name)), p.width)
            )),
            Item::Chan(s, w, true) => {
                let head = if *w == 1 {
                    format!("{s}.head().to_bool() as u128")
                } else {
                    format!("{s}.head().raw()")
                };
                before.push_str(&format!(
                    "            let {s}_offered = {s}.peek().is_some();\n \
                                \
                     m.set({}, {s}_offered as u128);\n            \
                     m.set({}, {head});\n",
                    idx(&format!("{s}_valid")),
                    idx(&format!("{s}_data"))
                ));
                after.push_str(&format!(
                    "            if {s}_offered && m.get({}) != 0 {{\n \
                                    \
                     {s}.recv();\n            }}\n",
                    idx(&format!("{s}_ready"))
                ));
            }
            Item::Chan(s, w, false) => {
                before.push_str(&format!(
                    "            let {s}_room = {s}.ready().to_bool();\n \
                                \
                     m.set({}, {s}_room as u128);\n",
                    idx(&format!("{s}_ready"))
                ));
                after.push_str(&format!(
                    "            if {s}_room && m.get({}) != 0 {{\n \
                                    \
                     {s}.send({});\n            }}\n",
                    idx(&format!("{s}_valid")),
                    from_raw(
                        &format!("m.get({})", idx(&format!("{s}_data"))),
                        *w
                    )
                ));
            }
        }
    }
    format!(
        "// Generated by //tools/vshim from {top}: the unit.\n\
         //! `{top}`, a foreign module, as a unit.\n\
         #![allow(unused_imports, unused_variables, clippy::all)]\n\
         use txhdl::comp::{{Clock, DefaultClock, In, Out, Rx, Tx, Unit}};\n\
         use txhdl::foreign::{{Cosim, Model, Shim}};\n\
         use txhdl::types::{{Bit, U}};\n\n\
         {extern_block}{runfile_fn}\
         /// The module, its clock pulsed once per rising edge of the\n\
         /// default clock.\n\
         pub struct {strukt} {{\n    pub model: {model_ty},\n}}\n\n\
         impl Default for {strukt} {{\n    fn default() -> Self {{\n        \
         Self {{\n            model: {construct},\n        }}\n    }}\n}}\n\n\
         impl Unit<{}, {}> for {strukt} {{\n    \
         async fn run(&mut self, {}, {}) {{\n        \
         loop {{\n            DefaultClock::rising().await;\n            \
         let m = &mut self.model;\n{before}            m.eval();\n\
         {after}            \
         m.edge();\n{outs}        }}\n    }}\n}}\n",
        tuple(&in_ty),
        tuple(&out_ty),
        param(&in_nm, &in_ty),
        param(&out_nm, &out_ty),
    )
}
