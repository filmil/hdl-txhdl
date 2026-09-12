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
use crate::comp::{Clock, In, Mem, Out, Reg, Rx, Tx, Wire};
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
    /// Words, for a memory; zero for everything else.
    const DEPTH: usize = 0;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Reg<T, C> {
    const KIND: Option<Kind> = Some(Kind::Reg);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Out<T, C> {
    const KIND: Option<Kind> = Some(Kind::Out);
    const WIDTH: usize = T::WIDTH;
}
impl<T: Value + Copy + 'static, C: Clock> Port for Wire<T, C> {
    const KIND: Option<Kind> = Some(Kind::Wire);
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
impl<T: Value + Copy + 'static, const N: usize, C: Clock> Port
    for Mem<T, N, C>
{
    const KIND: Option<Kind> = Some(Kind::Mem);
    const WIDTH: usize = T::WIDTH;
    const DEPTH: usize = N;
}
impl<T> Port for PhantomData<T> {}
macro_rules! plain {
    ($($t:ty),*) => { $( impl Port for $t {} )* };
}
plain!(u8, u16, u32, u64, u128, usize, bool, &'static str);
plain!(crate::types::Bit, crate::types::Logic);
impl<const N: usize> Port for crate::types::U<N> {}

/// A unit's fields by name, kind, width and depth. Derived with
/// `Trace`.
pub trait Fields {
    fn fields() -> Vec<(&'static str, Option<Kind>, usize, usize)>;
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
    /// `+ - * & | && || == != < > <= >=`, spelled as Verilog spells
    /// them.
    Bin(&'static str, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A bit of a value, or a word of a memory.
    Index(Box<Expr>, Box<Expr>),
    /// `slice::<LO, LEN>`: bits `LO + LEN - 1` down to `LO`.
    Slice(Box<Expr>, usize, usize),
    /// `concat`: the first above the second.
    Cat(Box<Expr>, Box<Expr>),
    /// `sext::<M>` and `zext::<M>`: to `M` bits, by the top bit or by
    /// zeros.
    Sext(Box<Expr>, usize),
    Zext(Box<Expr>, usize),
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
                    "&&" | "||" | "==" | "!=" | "<" | ">" | "<=" | ">=" | "<s"
                )
            }
            Expr::Not(a) => a.is_bool(),
            _ => false,
        }
    }
}

/// Where a drive lands: a register or a wire by name, or a word of a
/// memory, `m.at(addr)`.
#[derive(Clone, Debug)]
pub enum Target {
    Name(String),
    Word(String, Expr),
}

/// One statement of a lowered `run` body, as `#[lower]` emits it.
pub enum Stmt {
    /// `target.set(expr)` or `target <= expr`: a register takes it at
    /// the edge, a wire is assigned it, a word of a memory is written.
    Drive(Target, Expr),
    /// `when!(cond => { drives } else { drives })`.
    When(Expr, Vec<(Target, Expr)>, Vec<(Target, Expr)>),
    /// `case!(value => { pattern => { drives }, .. })`: arms in order,
    /// each a condition on the value, the first that holds wins.
    Case(Vec<(Expr, Vec<(Target, Expr)>)>),
    /// The wait the loop makes: every register drive after it happens
    /// only at an edge at which the condition holds.
    Guard(Expr),
}

/// One process of a unit: a loop of one wait, on the rising or the
/// falling edge of its clock, and the statements after the wait. A
/// clocked block in the netlist.
pub struct Process {
    pub clock: &'static str,
    pub falling: bool,
    pub body: Vec<Stmt>,
}

/// A unit as `#[lower]` read it: what its `run` needs from its ports,
/// its registers from its fields, and its processes, one per loop of
/// `run`. The two emitters render it.
pub struct Lowered {
    pub name: String,
    pub fields: Vec<(&'static str, Option<Kind>, usize, usize)>,
    pub ports: Vec<(String, Kind, usize)>,
    /// The `let` names of the loops that are computed, each a wire
    /// driven by its expression; a read of a port or register is an
    /// alias and not here.
    pub wires: Vec<(String, Expr)>,
    pub procs: Vec<Process>,
    /// A memory's first words, as `Mem::with` gave them: a program.
    pub init: Vec<(String, Vec<u128>)>,
    /// A port's trace scope when it is not the port's own name: a
    /// channel two units share under one name in the run has a port
    /// name of its own on each side.
    pub aliases: Vec<(String, String)>,
}

impl Lowered {
    /// The clocks the processes wait for, each once, in order.
    fn clocks(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for p in &self.procs {
            if !out.contains(&p.clock) {
                out.push(p.clock);
            }
        }
        out
    }
    /// Whether a register is driven by a falling-edge process.
    fn falling_reg(&self, name: &str) -> bool {
        fn drives(d: &[(Target, Expr)], name: &str) -> bool {
            d.iter()
                .any(|(t, _)| matches!(t, Target::Name(n) if n == name))
        }
        self.procs.iter().any(|p| {
            p.falling
                && p.body.iter().any(|st| match st {
                    Stmt::Drive(Target::Name(n), _) => n == name,
                    Stmt::Drive(_, _) | Stmt::Guard(_) => false,
                    Stmt::When(_, a, b) => drives(a, name) || drives(b, name),
                    Stmt::Case(arms) => {
                        arms.iter().any(|(_, d)| drives(d, name))
                    }
                })
        })
    }
    /// Give a memory its first words, as `Mem::with` gave them at run
    /// time; the netlist cannot see those, so the example says them
    /// again here.
    pub fn init(&mut self, mem: &str, words: &[u128]) {
        self.init.push((mem.to_string(), words.to_vec()));
    }
    /// Say under which trace scope a port is found, when the run named
    /// the wire or channel otherwise than the port.
    pub fn trace_as(&mut self, port: &str, scope: &str) {
        self.aliases.push((port.to_string(), scope.to_string()));
    }
    fn scope_col(&self, port: &str) -> String {
        match self.aliases.iter().find(|(p, _)| p == port) {
            Some((_, s)) => format!(" {s}"),
            None => String::new(),
        }
    }
    /// Verilog cannot part-select an expression, so every slice of
    /// one is hoisted into a wire of its own, `slN`, declared before
    /// the body that uses it.
    #[allow(clippy::type_complexity)]
    fn hoisted(
        &self,
    ) -> (
        Vec<Process>,
        Vec<(String, Expr)>,
        Vec<(String, usize, Expr)>,
    ) {
        let mut temps: Vec<(String, usize, Expr)> = Vec::new();
        fn go(
            e: &Expr,
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Expr {
            let b = |x: &Expr, t: &mut Vec<(String, usize, Expr)>| {
                Box::new(go(x, l, t))
            };
            match e {
                Expr::Slice(a, lo, len) => {
                    let a = go(a, l, t);
                    let plain = matches!(&a, Expr::Name(_))
                        || matches!(&a, Expr::Index(m, _)
                            if matches!(&**m, Expr::Name(n) if l.is_mem(n)));
                    if plain {
                        return Expr::Slice(Box::new(a), *lo, *len);
                    }
                    // The same expression sliced twice is one wire.
                    let same = format!("{a:?}");
                    let found =
                        t.iter().find(|(_, _, e)| format!("{e:?}") == same);
                    let name = match found {
                        Some((n, _, _)) => n.clone(),
                        None => {
                            let n = format!("sl{}", t.len());
                            t.push((n.clone(), l.ewidth(&a), a));
                            n
                        }
                    };
                    Expr::Slice(Box::new(Expr::Name(name)), *lo, *len)
                }
                Expr::Bin(op, a, c) => Expr::Bin(op, b(a, t), b(c, t)),
                Expr::Not(a) => Expr::Not(b(a, t)),
                Expr::Cond(c, a, d) => Expr::Cond(b(c, t), b(a, t), b(d, t)),
                Expr::Index(a, i) => Expr::Index(b(a, t), b(i, t)),
                Expr::Cat(a, c) => Expr::Cat(b(a, t), b(c, t)),
                Expr::Sext(a, m) => Expr::Sext(b(a, t), *m),
                Expr::Zext(a, m) => Expr::Zext(b(a, t), *m),
                e => e.clone(),
            }
        }
        let target = |x: &Target, t: &mut Vec<(String, usize, Expr)>| match x {
            Target::Word(m, a) => Target::Word(m.clone(), go(a, self, t)),
            n => n.clone(),
        };
        let drives = |d: &[(Target, Expr)],
                      t: &mut Vec<(String, usize, Expr)>| {
            d.iter()
                .map(|(x, e)| (target(x, t), go(e, self, t)))
                .collect::<Vec<_>>()
        };
        let wires = self
            .wires
            .iter()
            .map(|(n, e)| (n.clone(), go(e, self, &mut temps)))
            .collect();
        let mut procs = Vec::new();
        for p in &self.procs {
            let body = p
                .body
                .iter()
                .map(|st| match st {
                    Stmt::Drive(x, e) => Stmt::Drive(
                        target(x, &mut temps),
                        go(e, self, &mut temps),
                    ),
                    Stmt::When(c, a, b) => Stmt::When(
                        go(c, self, &mut temps),
                        drives(a, &mut temps),
                        drives(b, &mut temps),
                    ),
                    Stmt::Case(arms) => Stmt::Case(
                        arms.iter()
                            .map(|(c, d)| {
                                (go(c, self, &mut temps), drives(d, &mut temps))
                            })
                            .collect(),
                    ),
                    Stmt::Guard(c) => Stmt::Guard(go(c, self, &mut temps)),
                })
                .collect();
            procs.push(Process {
                clock: p.clock,
                falling: p.falling,
                body,
            });
        }
        (procs, wires, temps)
    }
    fn is_reg(&self, t: &str) -> bool {
        self.fields
            .iter()
            .any(|(n, k, _, _)| *n == t && *k == Some(Kind::Reg))
    }
    fn is_mem(&self, t: &str) -> bool {
        self.fields
            .iter()
            .any(|(n, k, _, _)| *n == t && *k == Some(Kind::Mem))
    }
    /// The width of an expression, as far as the netlist can tell:
    /// what an extension's replication needs.
    fn ewidth(&self, e: &Expr) -> usize {
        match e {
            Expr::Name(n) => self.width(n),
            Expr::Num(_) => 0,
            Expr::Bits(w, _) => *w,
            Expr::Bin(..) if e.is_bool() => 1,
            Expr::Bin(_, a, b) | Expr::Cond(_, a, b) => {
                self.ewidth(a).max(self.ewidth(b))
            }
            Expr::Not(a) => self.ewidth(a),
            Expr::Index(a, _) => match &**a {
                Expr::Name(m) if self.is_mem(m) => self.width(m),
                _ => 1,
            },
            Expr::Slice(_, _, len) => *len,
            Expr::Cat(a, b) => self.ewidth(a) + self.ewidth(b),
            Expr::Sext(_, m) | Expr::Zext(_, m) => *m,
        }
    }
    /// The width of a register, a memory's word, a port, a wire, or a
    /// channel's part.
    fn width(&self, n: &str) -> usize {
        if let Some((_, e)) = self.wires.iter().find(|(w, _)| w == n) {
            return self.ewidth(e);
        }
        for (f, k, w, _) in &self.fields {
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
    /// `name direction width`, the clock first. A channel's three
    /// wires carry their side, `rxin`, `rxout`, `txin` or `txout`,
    /// since the trace names them by side and a channel's inputs are
    /// registered where a wire's are not.
    pub fn ports_file(&self) -> String {
        let mut out = String::new();
        for c in self.clocks() {
            out.push_str(&format!("{c} in 1\n"));
        }
        for (n, k, w) in &self.ports {
            let s = self.scope_col(n);
            match k {
                Kind::Out => out.push_str(&format!("{n} out {w}{s}\n")),
                Kind::In => out.push_str(&format!("{n} in {w}{s}\n")),
                Kind::Tx => out.push_str(&format!(
                    "{n}_data txout {w}{s}\n{n}_valid txout 1{s}\n\
                     {n}_ready txin 1{s}\n"
                )),
                Kind::Rx => out.push_str(&format!(
                    "{n}_data rxin {w}{s}\n{n}_valid rxin 1{s}\n\
                     {n}_ready rxout 1{s}\n"
                )),
                Kind::Reg | Kind::Mem | Kind::Wire => {}
            }
        }
        for (n, k, w, d) in &self.fields {
            match k {
                Some(Kind::Reg) if self.falling_reg(n) => {
                    out.push_str(&format!("{n} regf {w}\n"))
                }
                Some(Kind::Reg) => out.push_str(&format!("{n} reg {w}\n")),
                Some(Kind::Mem) => out.push_str(&format!("{n} mem {w} {d}\n")),
                Some(Kind::Wire) => out.push_str(&format!("{n} wire {w}\n")),
                _ => {}
            }
        }
        out
    }

    /// The Verilog.
    pub fn verilog(&self) -> String {
        let name = &self.name;
        let l = self;
        let mut out = String::new();
        let mut plist: Vec<String> =
            self.clocks().iter().map(|c| format!("input {c}")).collect();
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
                Kind::Reg | Kind::Mem | Kind::Wire => {}
            }
        }
        writeln!(out, "`timescale 1ns/1ps").unwrap();
        writeln!(out, "module {name}(\n  {}\n);", plist.join(",\n  ")).unwrap();
        for (n, k, w, d) in &self.fields {
            match k {
                // Zero at the start, as the runtime's register is.
                Some(Kind::Reg) => {
                    writeln!(out, "  reg {}{n} = 0;", range(*w)).unwrap()
                }
                // A wire kept as a field: declared here, driven below.
                Some(Kind::Wire) => {
                    writeln!(out, "  wire {}{n};", range(*w)).unwrap()
                }
                // A memory, zero at the start as the runtime's is, then
                // its first words if the example gave them.
                Some(Kind::Mem) => {
                    writeln!(
                        out,
                        "  reg {}{n} [0:{}];\n  integer {n}_i;\n  \
                         initial for ({n}_i = 0; {n}_i < {d}; \
                         {n}_i = {n}_i + 1) \
                         {n}[{n}_i] = 0;",
                        range(*w),
                        d - 1
                    )
                    .unwrap();
                    for (m, words) in &self.init {
                        if m != n {
                            continue;
                        }
                        writeln!(out, "  initial begin").unwrap();
                        for (i, v) in words.iter().enumerate() {
                            if *v != 0 {
                                writeln!(out, "    {n}[{i}] = {w}'h{v:x};")
                                    .unwrap();
                            }
                        }
                        writeln!(out, "  end").unwrap();
                    }
                }
                _ => {}
            }
        }
        let (procs, wires, temps) = self.hoisted();
        for (n, e) in &wires {
            let w = self.ewidth(e);
            assert!(w > 0, "wire `{n}` has no width: size its literals");
            writeln!(out, "  wire {}{n};", range(w)).unwrap();
        }
        for (t, w, e) in &temps {
            writeln!(out, "  wire {}{t} = {};", range(*w), vexpr(e, l))
                .unwrap();
        }
        let mut comb: Vec<String> = Vec::new();
        let drive = |seq: &mut Vec<String>,
                     comb: &mut Vec<String>,
                     ind: &str,
                     t: &Target,
                     e: &Expr| match t {
            Target::Word(m, a) => seq.push(format!(
                "{ind}{m}[{}] <= {};",
                vexpr(a, l),
                vexpr(e, l)
            )),
            Target::Name(t) if self.is_reg(t) => {
                seq.push(format!("{ind}{t} <= {};", vexpr(e, l)))
            }
            Target::Name(t) => {
                comb.push(format!("  assign {t} = {};", vexpr(e, l)))
            }
        };
        for (n, e) in &wires {
            comb.push(format!("  assign {n} = {};", vexpr(e, l)));
        }
        // A clocked block per process, on its clock and its edge.
        for p in &procs {
            let mut seq: Vec<String> = Vec::new();
            let mut guard = false;
            for st in &p.body {
                match st {
                    Stmt::Guard(c) => {
                        guard = true;
                        seq.push(format!("    if ({}) begin", vexpr(c, l)));
                    }
                    Stmt::Drive(t, e) => {
                        let ind = if guard { "      " } else { "    " };
                        drive(&mut seq, &mut comb, ind, t, e)
                    }
                    Stmt::When(c, then, otherwise) => {
                        seq.push(format!("    if ({}) begin", vexpr(c, l)));
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
                            seq.push(format!(
                                "    {kw} ({}) begin",
                                vexpr(c, l)
                            ));
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
                let edge = if p.falling { "negedge" } else { "posedge" };
                writeln!(out, "  always @({edge} {}) begin", p.clock).unwrap();
                for l in &seq {
                    writeln!(out, "{l}").unwrap();
                }
                writeln!(out, "  end").unwrap();
            }
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
        let name = &self.name;
        let ty = |w: usize| {
            if w == 1 {
                "std_logic".to_string()
            } else {
                format!("unsigned({} downto 0)", w - 1)
            }
        };
        let mut plist: Vec<String> = self
            .clocks()
            .iter()
            .map(|c| format!("{c} : in std_logic"))
            .collect();
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
                Kind::Reg | Kind::Mem | Kind::Wire => {}
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
        // A conditional inside an expression, and a truth value as a
        // bit: functions, since VHDL-2008 has neither as an operator.
        out.push_str(
            "  function mux(c : boolean; a, b : unsigned) \
             return unsigned is\n  \
             begin if c then return a; else return b; end if; \
             end function;\n  \
             function mux(c : boolean; a, b : std_logic) \
             return std_logic is\n  \
             begin if c then return a; else return b; end if; \
             end function;\n  \
             function tobit(c : boolean) return std_logic is\n  \
             begin if c then return '1'; else return '0'; end if; \
             end function;\n",
        );
        for (n, k, w, d) in &self.fields {
            let init = if *w == 1 {
                "'0'".to_string()
            } else {
                "(others => '0')".to_string()
            };
            match k {
                Some(Kind::Reg) => {
                    writeln!(out, "  signal {n} : {} := {init};", ty(*w))
                        .unwrap()
                }
                Some(Kind::Wire) => {
                    writeln!(out, "  signal {n} : {};", ty(*w)).unwrap()
                }
                // A memory: an array type of its own, zero at the start,
                // then its first words if the example gave them.
                Some(Kind::Mem) => {
                    let mut words = String::new();
                    for (m, ws) in &self.init {
                        if m != n {
                            continue;
                        }
                        for (i, v) in ws.iter().enumerate() {
                            if *v != 0 {
                                let b = format!("{v:0w$b}", w = *w);
                                words.push_str(&format!("{i} => \"{b}\", "));
                            }
                        }
                    }
                    writeln!(
                        out,
                        "  type {n}_t is array (0 to {}) of {};\n  \
                         signal {n} : {n}_t := ({words}others => {init});",
                        d - 1,
                        ty(*w)
                    )
                    .unwrap()
                }
                _ => {}
            }
        }
        let (procs, wires, temps) = self.hoisted();
        for (n, e) in &wires {
            let w = self.ewidth(e);
            assert!(w > 0, "wire `{n}` has no width: size its literals");
            writeln!(out, "  signal {n} : {};", ty(w)).unwrap();
        }
        for (t, w, _) in &temps {
            writeln!(out, "  signal {t} : {};", ty(*w)).unwrap();
        }
        writeln!(out, "begin").unwrap();
        let mut comb: Vec<String> = Vec::new();
        // A value for a target of width `w`: an integer becomes an
        // unsigned of that width, since VHDL will not assign one bare.
        let sized = |e: &Expr, w: usize| -> String {
            match e {
                Expr::Num(k) if w > 1 => format!("to_unsigned({k}, {w})"),
                e => hval(e, w, self),
            }
        };
        let assign = |t: &str, w: usize, e: &Expr| -> String {
            match e {
                Expr::Cond(c, a, b) => format!(
                    "{t} <= {} when {} else {};",
                    sized(a, w),
                    hbool(c, self),
                    sized(b, w)
                ),
                e if e.is_bool() && w == 1 => {
                    format!("{t} <= '1' when {} else '0';", hbool(e, self))
                }
                e if e.is_bool() => format!(
                    "{t} <= to_unsigned(1, {w}) when {} \
                     else to_unsigned(0, {w});",
                    hbool(e, self)
                ),
                e => format!("{t} <= {};", sized(e, w)),
            }
        };
        let drive = |seq: &mut Vec<String>,
                     comb: &mut Vec<String>,
                     ind: &str,
                     t: &Target,
                     e: &Expr| match t {
            Target::Word(m, a) => {
                let word = format!("{m}(to_integer({}))", hval(a, 0, self));
                seq.push(format!("{ind}{}", assign(&word, self.width(m), e)));
            }
            Target::Name(t) if self.is_reg(t) => {
                seq.push(format!("{ind}{}", assign(t, self.width(t), e)))
            }
            Target::Name(t) => {
                comb.push(format!("  {}", assign(t, self.width(t), e)))
            }
        };
        for (t, w, e) in &temps {
            comb.push(format!("  {}", assign(t, *w, e)));
        }
        for (n, e) in &wires {
            comb.push(format!("  {}", assign(n, self.ewidth(e), e)));
        }
        // A process per process, on its clock and its edge.
        for p in &procs {
            let mut seq: Vec<String> = Vec::new();
            let mut guard = false;
            for st in &p.body {
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
                            seq.push(format!(
                                "      {kw} {} then",
                                hbool(c, self)
                            ));
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
                let c = p.clock;
                let edge = if p.falling { "falling" } else { "rising" };
                writeln!(
                    out,
                    "  process ({c})\n  begin\n    if {edge}_edge({c}) then"
                )
                .unwrap();
                for l in &seq {
                    writeln!(out, "{l}").unwrap();
                }
                writeln!(out, "    end if;\n  end process;").unwrap();
            }
        }
        for l in &comb {
            writeln!(out, "{l}").unwrap();
        }
        writeln!(out, "end architecture;").unwrap();
        out
    }
}

/// Write a lowered unit's VHDL and its ports sidecar where `TXHDL_VHDL`
/// points, and its Verilog where `TXHDL_VERILOG` points, if they do.
/// What an example calls so the build can simulate what the example
/// lowered, under nvc and under Verilator.
pub fn write_vhdl_from_env(l: &Lowered) {
    write_netlists_from_env(&[l]);
}

/// Several units of one run into the same files: the VHDL and the
/// Verilog one after another, and the ports file in sections, each
/// opened by a line `entity NAME`, which the testbench generator reads
/// for the entity it is asked for.
pub fn write_netlists_from_env(units: &[&Lowered]) {
    if let Ok(p) = std::env::var("TXHDL_VHDL") {
        let vhdl: Vec<String> = units.iter().map(|l| l.vhdl()).collect();
        std::fs::write(&p, vhdl.join("\n")).expect("TXHDL_VHDL file");
        let ports: Vec<String> = units
            .iter()
            .map(|l| format!("entity {}\n{}", l.name, l.ports_file()))
            .collect();
        std::fs::write(format!("{p}.ports"), ports.join(""))
            .expect("ports file");
    }
    if let Ok(p) = std::env::var("TXHDL_VERILOG") {
        let v: Vec<String> = units.iter().map(|l| l.verilog()).collect();
        std::fs::write(&p, v.join("\n")).expect("TXHDL_VERILOG file");
    }
}

/// The top bit of an expression, in Verilog: what a sign extension
/// replicates. A slice's is a bit of what it slices, since Verilog
/// does not index an expression.
fn vtop(e: &Expr, l: &Lowered) -> String {
    match e {
        Expr::Slice(a, lo, len) => format!("{}[{}]", vexpr(a, l), lo + len - 1),
        Expr::Cat(a, _) => vtop(a, l),
        _ => format!("{}[{}]", vexpr(e, l), l.ewidth(e).max(1) - 1),
    }
}

/// An expression in Verilog.
fn vexpr(e: &Expr, l: &Lowered) -> String {
    match e {
        Expr::Name(n) => n.clone(),
        Expr::Num(k) => k.to_string(),
        Expr::Bits(w, b) => format!("{w}'b{b}"),
        Expr::Bin("<s", a, b) => {
            format!("($signed({}) < $signed({}))", vexpr(a, l), vexpr(b, l))
        }
        // Self-determined inside $unsigned, since Verilog decides
        // signedness by the whole expression, and an unsigned operand
        // anywhere in it would make the shift logical.
        Expr::Bin(">>>", a, b) => {
            format!("$unsigned($signed({}) >>> {})", vexpr(a, l), vexpr(b, l))
        }
        Expr::Bin(op, a, b) => {
            format!("({} {op} {})", vexpr(a, l), vexpr(b, l))
        }
        Expr::Not(a) if a.is_bool() => format!("(!{})", vexpr(a, l)),
        Expr::Not(a) => format!("(~{})", vexpr(a, l)),
        // A number in a branch takes the other branch's width: unsized,
        // it would be 32 bits wide inside a concatenation.
        Expr::Cond(c, a, b) => {
            let w = l.ewidth(a).max(l.ewidth(b));
            let side = |x: &Expr| match x {
                Expr::Num(k) if w > 0 => format!("{w}'d{k}"),
                x => vexpr(x, l),
            };
            format!("({} ? {} : {})", vexpr(c, l), side(a), side(b))
        }
        Expr::Index(a, i) => format!("{}[{}]", vexpr(a, l), vexpr(i, l)),
        Expr::Slice(a, lo, len) => {
            format!("{}[{}:{}]", vexpr(a, l), lo + len - 1, lo)
        }
        Expr::Cat(a, b) => format!("{{{}, {}}}", vexpr(a, l), vexpr(b, l)),
        Expr::Sext(a, m) => match l.ewidth(a) {
            n if n >= *m || n == 0 => vexpr(a, l),
            n => {
                format!("{{{{{}{{{}}}}}, {}}}", m - n, vtop(a, l), vexpr(a, l))
            }
        },
        Expr::Zext(a, m) => match l.ewidth(a) {
            n if n >= *m || n == 0 => vexpr(a, l),
            n => format!("{{{{{}{{1'b0}}}}, {}}}", m - n, vexpr(a, l)),
        },
    }
}

/// An expression in VHDL, as a truth value.
fn hbool(e: &Expr, l: &Lowered) -> String {
    match e {
        // A one-bit literal is a truth value outright.
        Expr::Bits(1, b) => (if b == "1" { "true" } else { "false" }).into(),
        Expr::Bin(op @ ("&&" | "||"), a, b) => {
            let w = if *op == "&&" { "and" } else { "or" };
            format!("({} {w} {})", hbool(a, l), hbool(b, l))
        }
        // Bitwise operators and shifts yield bits, not truth values.
        Expr::Bin(
            "&" | "|" | "^" | "<<" | ">>" | ">>>" | "+" | "-" | "*",
            ..,
        ) => {
            format!("({} = '1')", hval(e, 1, l))
        }
        Expr::Bin("<s", a, b) => {
            format!("(signed({}) < signed({}))", hval(a, 0, l), hval(b, 0, l))
        }
        Expr::Bin(op, a, b) => {
            let vop = match *op {
                "==" => "=",
                "!=" => "/=",
                o => o,
            };
            // Both sides at the width either side has: a one-bit name
            // against a number is a comparison with a bit.
            let w = l.ewidth(a).max(l.ewidth(b));
            format!("({} {vop} {})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Not(a) => format!("(not {})", hbool(a, l)),
        other => format!("({} = '1')", hval(other, 1, l)),
    }
}

/// A shift count in VHDL: an integer, which a literal already is and
/// a value is converted to.
fn hint(e: &Expr, l: &Lowered) -> String {
    match e {
        Expr::Num(k) => k.to_string(),
        e => format!("to_integer({})", hval(e, 0, l)),
    }
}

/// An expression in VHDL, as a value of width `w`.
fn hval(e: &Expr, w: usize, l: &Lowered) -> String {
    match e {
        Expr::Name(n) => n.clone(),
        Expr::Num(k) if w == 1 => format!("'{}'", if *k == 0 { 0 } else { 1 }),
        Expr::Num(k) => k.to_string(),
        Expr::Bits(bw, b) if *bw == 1 => format!("'{b}'"),
        Expr::Bits(_, b) => format!("unsigned'(\"{b}\")"),
        Expr::Bin(op @ ("+" | "-"), a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
        // A product is twice as wide as its operands in VHDL; the
        // lowering states its width, so the product is cut to it.
        Expr::Bin("*", a, b) => {
            let w = if w == 0 { l.ewidth(e) } else { w };
            format!("resize(({} * {}), {w})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Bin(op @ ("&" | "|" | "^"), a, b) => {
            let vop = match *op {
                "&" => "and",
                "|" => "or",
                _ => "xor",
            };
            // At the operands' own width: a bit and a truth value are
            // both std_logic there.
            let w = if w == 0 {
                l.ewidth(e)
            } else {
                w.min(l.ewidth(e).max(1))
            };
            let w = if l.ewidth(e) == 1 { 1 } else { w };
            format!("({} {vop} {})", hval(a, w, l), hval(b, w, l))
        }
        Expr::Bin("<<", a, b) => {
            format!("shift_left({}, {})", hval(a, w, l), hint(b, l))
        }
        Expr::Bin(">>", a, b) => {
            format!("shift_right({}, {})", hval(a, w, l), hint(b, l))
        }
        Expr::Bin(">>>", a, b) => format!(
            "unsigned(shift_right(signed({}), {}))",
            hval(a, w, l),
            hint(b, l)
        ),
        Expr::Not(a) => format!("(not {})", hval(a, w, l)),
        Expr::Slice(a, lo, len) => {
            format!("{}({} downto {})", hval(a, 0, l), lo + len - 1, lo)
        }
        // Qualified, since every array type in scope has a `&` too.
        Expr::Cat(a, b) => {
            format!("unsigned'({} & {})", hval(a, 0, l), hval(b, 0, l))
        }
        Expr::Sext(a, m) => {
            format!("unsigned(resize(signed({}), {m}))", hval(a, 0, l))
        }
        // A bit is not an array in VHDL, so a bit is extended by
        // putting zeros before it, and a bit extended to a bit is itself.
        Expr::Zext(a, m) if l.ewidth(a) == 1 && *m == 1 => hval(a, 1, l),
        Expr::Zext(a, m) if l.ewidth(a) == 1 => {
            format!("unsigned'(to_unsigned(0, {}) & {})", m - 1, hval(a, 1, l))
        }
        Expr::Zext(a, m) => format!("resize({}, {m})", hval(a, 0, l)),
        // A word of a memory, or a bit of a value.
        Expr::Index(a, i) => match &**a {
            Expr::Name(m) if l.is_mem(m) => {
                format!("{m}(to_integer({}))", hval(i, 0, l))
            }
            _ => format!("{}({})", hval(a, 0, l), vexpr(i, l)),
        },
        // A truth value as a bit, or as a word of that width.
        e if e.is_bool() && w > 1 => format!(
            "mux({}, to_unsigned(1, {w}), to_unsigned(0, {w}))",
            hbool(e, l)
        ),
        e if e.is_bool() => format!("tobit({})", hbool(e, l)),
        // A conditional inside an expression is the mux function; a
        // number in a branch takes the other branch's width.
        Expr::Cond(c, a, b) => {
            let w = if w == 0 {
                l.ewidth(a).max(l.ewidth(b))
            } else {
                w
            };
            let side = |x: &Expr| match x {
                Expr::Num(k) if w > 1 => format!("to_unsigned({k}, {w})"),
                x => hval(x, w, l),
            };
            format!("mux({}, {}, {})", hbool(c, l), side(a), side(b))
        }
        Expr::Bin(op, a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
    }
}
