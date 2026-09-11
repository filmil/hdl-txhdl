// SPDX-License-Identifier: Apache-2.0
//! A structural netlist, from the same walk the waveform uses. Every
//! unit is a module, every register a `reg`, and every wire or channel
//! whose ends are fields of different units is a net between them,
//! with a port on each and a wire in their common parent. The walk
//! sees fields, so a unit whose ports are fields yields ports, and a
//! unit that takes its ports as parameters of `run` yields only its
//! registers: that is the gap between a structural netlist and the
//! lowering, which the proc-macro route closes by reading the
//! `Unit` impl. Behaviour is not here at all; the bodies are empty.
use crate::comp::trace::{collect, Kind, Probe, Traceable};
use crate::comp::{Clock, In, Mem, Out, Reg, Rx, Tx};
use crate::types::Value;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::marker::PhantomData;

/// The Verilog skeleton of `top`.
pub fn verilog(name: &str, top: &impl Traceable) -> String {
    let probes = collect(name, top);
    let ends = |cell: usize| -> Vec<&Probe> {
        probes
            .iter()
            .filter(|p| p.kind != Kind::Reg && p.cell == cell)
            .collect()
    };
    // Every scope that holds something, plus every ancestor.
    let mut modules: BTreeMap<String, Vec<&Probe>> = BTreeMap::new();
    for p in &probes {
        modules
            .entry(scope_of(&p.path).to_string())
            .or_default()
            .push(p);
        let mut s = scope_of(&p.path);
        while let Some((parent, _)) = s.rsplit_once('.') {
            modules.entry(parent.to_string()).or_default();
            s = parent;
        }
    }
    let mut out = String::new();
    for (path, items) in &modules {
        let mut ports = Vec::new();
        let mut regs = Vec::new();
        for p in items {
            if p.kind == Kind::Reg {
                let r = format!("  reg {}{};", range(p.width), leaf(&p.path));
                regs.push(r);
            } else if ends(p.cell).iter().any(|q| scope_of(&q.path) != path) {
                let dir = match p.kind {
                    Kind::Out | Kind::Tx => "output",
                    _ => "input",
                };
                let w = range(p.width);
                ports.push(format!("{dir} {w}{}", leaf(&p.path)));
            }
        }
        writeln!(out, "module {}({});", ident(path), ports.join(", ")).unwrap();
        for r in regs {
            writeln!(out, "{r}").unwrap();
        }
        // Nets whose ends lie in this module's children: declared here.
        let mut seen = Vec::new();
        for p in &probes {
            if p.kind == Kind::Reg || seen.contains(&p.cell) {
                continue;
            }
            let e = ends(p.cell);
            let below = |q: &&Probe| {
                scope_of(&q.path)
                    .strip_prefix(path.as_str())
                    .map_or(false, |r| r.starts_with('.'))
            };
            if e.len() < 2 || !e.iter().all(below) {
                continue;
            }
            let first = child_of(path, scope_of(&e[0].path));
            let spans_children =
                e.iter().any(|q| child_of(path, scope_of(&q.path)) != first);
            if spans_children {
                seen.push(p.cell);
                let w = range(p.width);
                writeln!(out, "  wire {w}{};", net_name(&e)).unwrap();
            }
        }
        // Instances: the direct children, ports connected to the nets.
        for (child, citems) in &modules {
            let Some(rest) = child.strip_prefix(&format!("{path}.")) else {
                continue;
            };
            if rest.contains('.') {
                continue;
            }
            let mut conns = Vec::new();
            for p in citems {
                if p.kind == Kind::Reg {
                    continue;
                }
                let e = ends(p.cell);
                if e.iter().any(|q| scope_of(&q.path) != child) {
                    conns.push(format!(".{}({})", leaf(&p.path), net_name(&e)));
                }
            }
            let c = conns.join(", ");
            writeln!(out, "  {} {rest}({c});", ident(child)).unwrap();
        }
        writeln!(out, "endmodule\n").unwrap();
    }
    out
}

/// A net is named after its driving end, or its first end.
fn net_name(ends: &[&Probe]) -> String {
    let d = ends
        .iter()
        .find(|p| matches!(p.kind, Kind::Out | Kind::Tx))
        .unwrap_or(&ends[0]);
    leaf(&d.path).replace('.', "_")
}
/// The child of `parent` on the way to `scope`.
fn child_of<'a>(parent: &str, scope: &'a str) -> &'a str {
    let rest = &scope[parent.len() + 1..];
    rest.split('.').next().unwrap()
}
fn leaf(path: &str) -> &str {
    path.rsplit('.').next().unwrap()
}
fn scope_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(a, _)| a).unwrap_or("")
}
fn ident(path: &str) -> String {
    path.replace('.', "_")
}
fn range(w: usize) -> String {
    if w > 1 {
        format!("[{}:0] ", w - 1)
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------
// Lowering a unit: what `#[lower]` needs at run time

/// What a field of a unit is to a netlist: state or an end of a wire,
/// with a width, or nothing. Every type a unit may hold implements it,
/// and `#[derive(Trace)]` gives a unit its `Fields` from them.
pub trait Port {
    const KIND: Option<Kind> = None;
    const WIDTH: usize = 0;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Reg<T, C> {
    const KIND: Option<Kind> = Some(Kind::Reg);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Out<T, C> {
    const KIND: Option<Kind> = Some(Kind::Out);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for In<T, C> {
    const KIND: Option<Kind> = Some(Kind::In);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + crate::types::Transaction + 'static, C: Clock> Port
    for Tx<T, C>
{
    const KIND: Option<Kind> = Some(Kind::Tx);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + crate::types::Transaction + 'static, C: Clock> Port
    for Rx<T, C>
{
    const KIND: Option<Kind> = Some(Kind::Rx);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Copy, const N: usize, C: Clock> Port for Mem<T, N, C> {}
impl<T> Port for PhantomData<T> {}
macro_rules! plain {
    ($($t:ty),*) => { $( impl Port for $t {} )* };
}
plain!(u8, u16, u32, u64, u128, usize, bool, &'static str);
plain!(crate::types::Bit, crate::types::Logic);
impl<const N: usize> Port for crate::types::U<N> {}

/// A unit's fields by name, kind and width. Derived with `Trace`.
pub trait Fields {
    fn fields() -> Vec<(&'static str, Option<Kind>, usize)>;
}

/// A value as an expression: its width and bits.
pub fn lit<V: Value>(v: V) -> Expr {
    Expr::Bits(V::WIDTH, v.vcd())
}

/// An expression of a lowered body, as `#[lower]` builds it: what both
/// emitters render.
#[derive(Clone, Debug)]
pub enum Expr {
    /// A register, a port, or a channel's `_data`, `_valid`, `_ready`.
    Name(String),
    /// An integer, from a literal or a configuration constant.
    Num(u128),
    /// A sized value, from an enum variant or a `Bit`.
    Bits(usize, String),
    /// `+ - & | && || == != < > <= >=`, spelled as Verilog spells them.
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
}

impl Expr {
    pub fn name(s: &str) -> Expr {
        Expr::Name(s.to_string())
    }
    pub fn bin(op: &'static str, a: Expr, b: Expr) -> Expr {
        Expr::Bin(op, Box::new(a), Box::new(b))
    }
    /// Whether this is a truth value rather than a number or bits.
    fn is_bool(&self) -> bool {
        match self {
            Expr::Bin(op, _, _) => {
                matches!(
                    *op,
                    "&&" | "||" | "==" | "!=" | "<" | ">" | "<=" | ">="
                )
            }
            Expr::Not(a) => a.is_bool(),
            _ => false,
        }
    }
}

/// One statement of a lowered `run` body, as `#[lower]` emits it.
pub enum Stmt {
    /// `target.set(expr)` or `target <= expr`: a register takes it at
    /// the edge, a wire is assigned it.
    Drive(String, Expr),
    /// `when!(cond => { drives } else { drives })`.
    When(Expr, Vec<(String, Expr)>, Vec<(String, Expr)>),
    /// `case!(value => { pattern => { drives }, .. })`: arms in order,
    /// each a condition on the value, the first that holds wins.
    Case(Vec<(Expr, Vec<(String, Expr)>)>),
    /// The wait the loop makes: every register drive after it happens
    /// only at an edge at which the condition holds.
    Guard(Expr),
}

/// A unit as `#[lower]` read it: what its `run` needs from its ports,
/// its registers from its fields, and the statements of its loop. The
/// two emitters render it.
pub struct Lowered {
    pub name: String,
    pub clock: &'static str,
    pub fields: Vec<(&'static str, Option<Kind>, usize)>,
    pub ports: Vec<(String, Kind, usize)>,
    pub body: Vec<Stmt>,
}

impl Lowered {
    fn is_reg(&self, t: &str) -> bool {
        self.fields
            .iter()
            .any(|(n, k, _)| *n == t && *k == Some(Kind::Reg))
    }
    /// The width of a register or port, or of a channel's part.
    fn width(&self, n: &str) -> usize {
        for (f, k, w) in &self.fields {
            if *f == n && k.is_some() {
                return *w;
            }
        }
        for (p, k, w) in &self.ports {
            match k {
                Kind::Tx | Kind::Rx => {
                    if n == format!("{p}_data") {
                        return *w;
                    }
                    if n == format!("{p}_valid") || n == format!("{p}_ready") {
                        return 1;
                    }
                }
                _ => {
                    if p == n {
                        return *w;
                    }
                }
            }
        }
        1
    }
    /// The ports as a sidecar for a testbench generator: one per line,
    /// `name direction width`, the clock first.
    pub fn ports_file(&self) -> String {
        let mut out = format!("{} in 1\n", self.clock);
        for (n, k, w) in &self.ports {
            match k {
                Kind::Out => out.push_str(&format!("{n} out {w}\n")),
                Kind::In => out.push_str(&format!("{n} in {w}\n")),
                Kind::Tx => out.push_str(&format!(
                    "{n}_data out {w}\n{n}_valid out 1\n{n}_ready in 1\n"
                )),
                Kind::Rx => out.push_str(&format!(
                    "{n}_data in {w}\n{n}_valid in 1\n{n}_ready out 1\n"
                )),
                Kind::Reg => {}
            }
        }
        for (n, k, w) in &self.fields {
            if *k == Some(Kind::Reg) {
                out.push_str(&format!("{n} reg {w}\n"));
            }
        }
        out
    }

    /// The Verilog.
    pub fn verilog(&self) -> String {
        let (name, clock) = (&self.name, self.clock);
        let mut out = String::new();
        let mut plist = vec![format!("input {clock}")];
        for (n, k, w) in &self.ports {
            match k {
                Kind::Out => plist.push(format!("output {}{n}", range(*w))),
                Kind::In => plist.push(format!("input {}{n}", range(*w))),
                Kind::Tx => plist.push(format!(
                    "output {}{n}_data, output {n}_valid, input {n}_ready",
                    range(*w)
                )),
                Kind::Rx => plist.push(format!(
                    "input {}{n}_data, input {n}_valid, output {n}_ready",
                    range(*w)
                )),
                Kind::Reg => {}
            }
        }
        writeln!(out, "module {name}(\n  {}\n);", plist.join(",\n  ")).unwrap();
        for (n, k, w) in &self.fields {
            if *k == Some(Kind::Reg) {
                writeln!(out, "  reg {}{n};", range(*w)).unwrap();
            }
        }
        let mut seq: Vec<String> = Vec::new();
        let mut comb: Vec<String> = Vec::new();
        let mut guard = false;
        let drive = |seq: &mut Vec<String>,
                     comb: &mut Vec<String>,
                     ind: &str,
                     t: &str,
                     e: &Expr| {
            if self.is_reg(t) {
                seq.push(format!("{ind}{t} <= {};", vexpr(e)));
            } else {
                comb.push(format!("  assign {t} = {};", vexpr(e)));
            }
        };
        for st in &self.body {
            match st {
                Stmt::Guard(c) => {
                    guard = true;
                    seq.push(format!("    if ({}) begin", vexpr(c)));
                }
                Stmt::Drive(t, e) => {
                    let ind = if guard { "      " } else { "    " };
                    drive(&mut seq, &mut comb, ind, t, e)
                }
                Stmt::When(c, then, otherwise) => {
                    seq.push(format!("    if ({}) begin", vexpr(c)));
                    for (t, e) in then {
                        drive(&mut seq, &mut comb, "      ", t, e);
                    }
                    if !otherwise.is_empty() {
                        seq.push("    end else begin".into());
                        for (t, e) in otherwise {
                            drive(&mut seq, &mut comb, "      ", t, e);
                        }
                    }
                    seq.push("    end".into());
                }
                Stmt::Case(arms) => {
                    for (i, (c, drives)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "end else if" };
                        seq.push(format!("    {kw} ({}) begin", vexpr(c)));
                        for (t, e) in drives {
                            drive(&mut seq, &mut comb, "      ", t, e);
                        }
                    }
                    if !arms.is_empty() {
                        seq.push("    end".into());
                    }
                }
            }
        }
        if guard {
            seq.push("    end".into());
        }
        if seq.iter().any(|l| l.contains("<=")) {
            writeln!(out, "  always @(posedge {clock}) begin").unwrap();
            for l in &seq {
                writeln!(out, "{l}").unwrap();
            }
            writeln!(out, "  end").unwrap();
        }
        for l in &comb {
            writeln!(out, "{l}").unwrap();
        }
        writeln!(out, "endmodule").unwrap();
        out
    }

    /// The VHDL, 2008: an entity, one process on the rising edge for
    /// the registers, a concurrent assignment per wire.
    pub fn vhdl(&self) -> String {
        let (name, clock) = (&self.name, self.clock);
        let ty = |w: usize| {
            if w == 1 {
                "std_logic".to_string()
            } else {
                format!("unsigned({} downto 0)", w - 1)
            }
        };
        let mut plist = vec![format!("{clock} : in std_logic")];
        for (n, k, w) in &self.ports {
            match k {
                Kind::Out => plist.push(format!("{n} : out {}", ty(*w))),
                Kind::In => plist.push(format!("{n} : in {}", ty(*w))),
                Kind::Tx => {
                    plist.push(format!("{n}_data : out {}", ty(*w)));
                    plist.push(format!("{n}_valid : out std_logic"));
                    plist.push(format!("{n}_ready : in std_logic"));
                }
                Kind::Rx => {
                    plist.push(format!("{n}_data : in {}", ty(*w)));
                    plist.push(format!("{n}_valid : in std_logic"));
                    plist.push(format!("{n}_ready : out std_logic"));
                }
                Kind::Reg => {}
            }
        }
        let mut out = String::new();
        out.push_str(
            "library ieee;\nuse ieee.std_logic_1164.all;\n\
             use ieee.numeric_std.all;\n\n",
        );
        writeln!(
            out,
            "entity {name} is\n  port (\n    {}\n  );\nend entity;\n",
            plist.join(";\n    ")
        )
        .unwrap();
        writeln!(out, "architecture rtl of {name} is").unwrap();
        for (n, k, w) in &self.fields {
            if *k == Some(Kind::Reg) {
                let init = if *w == 1 {
                    "'0'".to_string()
                } else {
                    "(others => '0')".to_string()
                };
                writeln!(out, "  signal {n} : {} := {init};", ty(*w)).unwrap();
            }
        }
        writeln!(out, "begin").unwrap();
        let mut seq: Vec<String> = Vec::new();
        let mut comb: Vec<String> = Vec::new();
        let mut guard = false;
        // A value for a target of width `w`: an integer becomes an
        // unsigned of that width, since VHDL will not assign one bare.
        let sized = |e: &Expr, w: usize| -> String {
            match e {
                Expr::Num(k) if w > 1 => format!("to_unsigned({k}, {w})"),
                e => hval(e, w, self),
            }
        };
        let assign = |t: &str, e: &Expr| -> String {
            let w = self.width(t);
            match e {
                Expr::Cond(c, a, b) => format!(
                    "{t} <= {} when {} else {};",
                    sized(a, w),
                    hbool(c, self),
                    sized(b, w)
                ),
                e if e.is_bool() => {
                    format!("{t} <= '1' when {} else '0';", hbool(e, self))
                }
                e => format!("{t} <= {};", sized(e, w)),
            }
        };
        let drive = |seq: &mut Vec<String>,
                     comb: &mut Vec<String>,
                     ind: &str,
                     t: &str,
                     e: &Expr| {
            if self.is_reg(t) {
                seq.push(format!("{ind}{}", assign(t, e)));
            } else {
                comb.push(format!("  {}", assign(t, e)));
            }
        };
        for st in &self.body {
            match st {
                Stmt::Guard(c) => {
                    guard = true;
                    seq.push(format!("      if {} then", hbool(c, self)));
                }
                Stmt::Drive(t, e) => {
                    let ind = if guard { "        " } else { "      " };
                    drive(&mut seq, &mut comb, ind, t, e)
                }
                Stmt::When(c, then, otherwise) => {
                    seq.push(format!("      if {} then", hbool(c, self)));
                    for (t, e) in then {
                        drive(&mut seq, &mut comb, "        ", t, e);
                    }
                    if !otherwise.is_empty() {
                        seq.push("      else".into());
                        for (t, e) in otherwise {
                            drive(&mut seq, &mut comb, "        ", t, e);
                        }
                    }
                    seq.push("      end if;".into());
                }
                Stmt::Case(arms) => {
                    for (i, (c, drives)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "elsif" };
                        seq.push(format!("      {kw} {} then", hbool(c, self)));
                        for (t, e) in drives {
                            drive(&mut seq, &mut comb, "        ", t, e);
                        }
                    }
                    if !arms.is_empty() {
                        seq.push("      end if;".into());
                    }
                }
            }
        }
        if guard {
            seq.push("      end if;".into());
        }
        if seq.iter().any(|l| l.contains("<=")) {
            writeln!(
                out,
                "  process ({clock})\n  begin\n    if rising_edge({clock}) then"
            )
            .unwrap();
            for l in &seq {
                writeln!(out, "{l}").unwrap();
            }
            writeln!(out, "    end if;\n  end process;").unwrap();
        }
        for l in &comb {
            writeln!(out, "{l}").unwrap();
        }
        writeln!(out, "end architecture;").unwrap();
        out
    }
}

/// Write a lowered unit's VHDL and its ports sidecar where `TXHDL_VHDL`
/// points, if it does. What an example calls so the build can simulate
/// what the example lowered.
pub fn write_vhdl_from_env(l: &Lowered) {
    if let Ok(p) = std::env::var("TXHDL_VHDL") {
        std::fs::write(&p, l.vhdl()).expect("TXHDL_VHDL file");
        std::fs::write(format!("{p}.ports"), l.ports_file())
            .expect("ports file");
    }
}

/// An expression in Verilog.
fn vexpr(e: &Expr) -> String {
    match e {
        Expr::Name(n) => n.clone(),
        Expr::Num(k) => k.to_string(),
        Expr::Bits(w, b) => format!("{w}'b{b}"),
        Expr::Bin(op, a, b) => format!("({} {op} {})", vexpr(a), vexpr(b)),
        Expr::Not(a) if a.is_bool() => format!("(!{})", vexpr(a)),
        Expr::Not(a) => format!("(~{})", vexpr(a)),
        Expr::Cond(c, a, b) => {
            format!("({} ? {} : {})", vexpr(c), vexpr(a), vexpr(b))
        }
        Expr::Index(a, i) => format!("{}[{}]", vexpr(a), vexpr(i)),
    }
}

/// An expression in VHDL, as a truth value.
fn hbool(e: &Expr, l: &Lowered) -> String {
    match e {
        Expr::Bin(op @ ("&&" | "||"), a, b) => {
            let w = if *op == "&&" { "and" } else { "or" };
            format!("({} {w} {})", hbool(a, l), hbool(b, l))
        }
        Expr::Bin(op, a, b) => {
            let vop = match *op {
                "==" => "=",
                "!=" => "/=",
                o => o,
            };
            // A one-bit name against a number is a comparison with a bit.
            let w = match (&**a, &**b) {
                (Expr::Name(n), _) | (_, Expr::Name(n)) => l.width(n),
                _ => 0,
            };
            format!("({} {vop} {})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Not(a) => format!("(not {})", hbool(a, l)),
        other => format!("({} = '1')", hval(other, 1, l)),
    }
}

/// An expression in VHDL, as a value of width `w`.
fn hval(e: &Expr, w: usize, l: &Lowered) -> String {
    match e {
        Expr::Name(n) => n.clone(),
        Expr::Num(k) if w == 1 => format!("'{}'", if *k == 0 { 0 } else { 1 }),
        Expr::Num(k) => k.to_string(),
        Expr::Bits(bw, b) if *bw == 1 => format!("'{b}'"),
        Expr::Bits(_, b) => format!("\"{b}\""),
        Expr::Bin(op @ ("+" | "-"), a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Bin(op @ ("&" | "|"), a, b) => {
            let vop = if *op == "&" { "and" } else { "or" };
            format!("({} {vop} {})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Not(a) => format!("(not {})", hval(a, w, l)),
        Expr::Index(a, i) => format!("{}({})", hval(a, 0, l), vexpr(i)),
        e if e.is_bool() => format!("({})", hbool(e, l)),
        Expr::Cond(c, a, b) => {
            format!(
                "{} when {} else {}",
                hval(a, w, l),
                hbool(c, l),
                hval(b, w, l)
            )
        }
        Expr::Bin(op, a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
    }
}
