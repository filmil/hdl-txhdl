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
/// The VHDL type a foreign module's port is declared with: the plain
/// logic types, since a module written by hand, or in Verilog, knows
/// nothing of `unsigned`.
fn logic(w: usize) -> String {
    if w > 1 {
        format!("std_logic_vector({} downto 0)", w - 1)
    } else {
        "std_logic".to_string()
    }
}

// ---------------------------------------------------------------------
// Lowering a unit: what `#[lower]` needs at run time

/// What a field of a unit is to a netlist: state or an end of a wire,
/// with a width, or nothing. Every type a unit may hold implements it,
/// and `#[derive(Trace)]` gives a unit its `Fields` from them.
pub trait Port {
    /// What this field is to a netlist, or `None` for a field that
    /// is neither state nor an end of a wire.
    const KIND: Option<Kind> = None;
    /// How wide it is, in bits.
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
    /// The names of the unit's fields, in order, for the checks
    /// `#[lower]` writes against the names of its wires and ports.
    const NAMES: &'static [&'static str];
    /// Every field of the unit, as its name, what it is, how wide it
    /// is, and how many words it holds if it is a memory.
    fn fields() -> Vec<(&'static str, Option<Kind>, usize, usize)>;
}

/// Whether `name` is one of `names`, in a constant: the check
/// `#[lower]` writes for a wire or a port that would take the name of
/// a field, which the netlist would declare twice.
#[doc(hidden)]
pub const fn has_name(names: &[&str], name: &str) -> bool {
    let name = name.as_bytes();
    let mut i = 0;
    while i < names.len() {
        let n = names[i].as_bytes();
        if n.len() == name.len() {
            let mut k = 0;
            while k < n.len() && n[k] == name[k] {
                k += 1;
            }
            if k == n.len() {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// A value as an expression: its width and bits.
pub fn lit<V: Value>(v: V) -> Expr {
    Expr::Bits(V::WIDTH, v.vcd())
}

/// A field of a compound value on a wire: the bits it occupies, as
/// `#[derive(Value)]` laid them out, the first field highest; a
/// one-bit field is a bit.
pub fn field<V: Value>(e: Expr, name: &str) -> Expr {
    let mut hi = V::WIDTH;
    for (n, w) in V::layout() {
        hi -= w;
        if n == name {
            return if w == 1 {
                Expr::Index(Box::new(e), Box::new(Expr::Num(hi as u128)))
            } else {
                Expr::Slice(Box::new(e), hi, w)
            };
        }
    }
    panic!("`{name}` is not a field of the value")
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
    /// Bitwise negation.
    Not(Box<Expr>),
    /// A choice: the condition, what it is when true, when false.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A bit of a value, or a word of a memory.
    Index(Box<Expr>, Box<Expr>),
    /// `slice::<LO, LEN>`: bits `LO + LEN - 1` down to `LO`.
    Slice(Box<Expr>, usize, usize),
    /// `concat`: the first above the second.
    Cat(Box<Expr>, Box<Expr>),
    /// `sext::<M>` and `zext::<M>`: to `M` bits, by the top bit or by
    /// zeros.
    /// Sign extension to the stated width.
    Sext(Box<Expr>, usize),
    /// Zero extension, or truncation, to the stated width.
    Zext(Box<Expr>, usize),
}

impl Expr {
    /// A reference to a signal by name: a register, a port, a wire.
    pub fn name(s: &str) -> Expr {
        Expr::Name(s.to_string())
    }
    /// A binary operator; two numbers fold to their result, so a
    /// constant written as arithmetic, `PERIOD - 1`, lowers as the
    /// number it is.
    pub fn bin(op: &'static str, a: Expr, b: Expr) -> Expr {
        if let (Expr::Num(x), Expr::Num(y)) = (&a, &b) {
            let (x, y) = (*x, *y);
            let v = match op {
                "+" => Some(x.wrapping_add(y)),
                "-" => Some(x.wrapping_sub(y)),
                "&" => Some(x & y),
                "|" => Some(x | y),
                "^" => Some(x ^ y),
                "<<" => Some(x.checked_shl(y as u32).unwrap_or(0)),
                ">>" => Some(x.checked_shr(y as u32).unwrap_or(0)),
                "/" if y != 0 => Some(x / y),
                _ => None,
            };
            if let Some(v) = v {
                return Expr::Num(v);
            }
        }
        Expr::Bin(op, Box::new(a), Box::new(b))
    }
    /// A condition that is a constant: `true` or `false`, or none.
    fn constant(&self) -> Option<bool> {
        match self {
            Expr::Num(k) => Some(*k != 0),
            Expr::Bits(_, b) => Some(b.contains('1')),
            _ => None,
        }
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
    /// A register, or an output port, driven whole.
    Name(String),
    /// One word of a memory: the memory's name and the address.
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
    /// `if c { .. } else if d { .. } else { .. }`: arms in order, each
    /// a condition and the statements under it, then the statements
    /// under no condition; the first condition that holds wins, and a
    /// statement under it may be another `if`.
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    /// The wait the loop makes: every register drive after it happens
    /// only at an edge at which the condition holds.
    Guard(Expr),
}

/// One process of a unit: a loop of one wait, on the rising or the
/// falling edge of its clock, and the statements after the wait. A
/// clocked block in the netlist.
pub struct Process {
    /// The clock this process waits on.
    pub clock: &'static str,
    /// Whether it waits for the falling edge rather than the rising.
    pub falling: bool,
    /// What it does at that edge.
    pub body: Vec<Stmt>,
}

/// A unit as `#[lower]` read it: what its `run` needs from its ports,
/// its registers from its fields, and its processes, one per loop of
/// `run`. The two emitters render it.
pub struct Lowered {
    /// What the module or entity is called.
    pub name: String,
    /// The unit's fields, as [`Fields::fields`] gives them: the
    /// registers and memories the netlist declares.
    pub fields: Vec<(&'static str, Option<Kind>, usize, usize)>,
    /// The ports, from `run`'s signature: name, what it is, width.
    pub ports: Vec<(String, Kind, usize)>,
    /// The `let` names of the loops that are computed, each a wire
    /// driven by its expression; a read of a port or register is an
    /// alias and not here.
    pub wires: Vec<(String, Expr)>,
    /// One per loop of `run`: a clocked block in the netlist.
    pub procs: Vec<Process>,
    /// A memory's first words, as `Mem::with` gave them: a program.
    pub init: Vec<(String, Vec<u128>)>,
    /// A port's trace scope when it is not the port's own name: a
    /// channel two units share under one name in the run has a port
    /// name of its own on each side.
    pub aliases: Vec<(String, String)>,
    /// The channels and wires a unit of units made in its `run` to
    /// join its children: name, `Tx` for a channel or `Out` for a
    /// wire, the payload's width and the clock. A channel is a buffer
    /// of two between its ends, as the runtime's is, an instance of
    /// `txhdl_chan` with the sender's three nets on one side and the
    /// receiver's on the other; a wire is one net.
    pub nets: Vec<(String, Kind, usize, &'static str)>,
    /// The children of a unit of units, each a module of its own
    /// instantiated once here.
    pub instances: Vec<Instance>,
    /// Set when the unit is a module the netlist does not write: its
    /// ports are the module's own, and a parent instantiates it by the
    /// module's name, with the parameters given, and emits no body for
    /// it. See [`foreign`].
    pub foreign: Option<Foreign>,
}

/// A module the netlist instantiates and does not write: a controller
/// from elsewhere, a vendor primitive, anything that comes as its own
/// source. The name is the module's, or the entity's, and the
/// parameters are its parameters, or its generics, by name.
pub struct Foreign {
    /// The module's own name, which the instance names.
    pub module: String,
    /// Its parameters, each a name and an integer.
    pub params: Vec<(String, i128)>,
    /// Its clock pins, each with the clock that drives it: joined to
    /// the parent's clock of that name, as a lowered child's clock is.
    pub clocks: Vec<(String, &'static str)>,
}

/// The lowering of a unit that is a foreign module: what its `Lower`
/// gives, written by hand, since there is no `run` for `#[lower]` to
/// read. `ports` are the module's ports by their own names, in the
/// order the unit's `run` takes them, each an `In`, an `Out` or a
/// `Pad`. `clocks` are its clock pins, each named with the clock that
/// drives it, such as `DefaultClock::NAME`; they are not among `ports`,
/// since the unit's `run` does not take a clock. A unit of units that
/// holds such a unit joins its ports as it joins any child's, and the
/// netlist has an instance of `module` with `params` where the child
/// would be.
pub fn foreign(
    name: &str,
    module: &str,
    ports: &[(&str, Kind, usize)],
    params: &[(&str, i128)],
    clocks: &[(&str, &'static str)],
) -> Lowered {
    for (p, k, _) in ports {
        assert!(
            matches!(k, Kind::In | Kind::Out | Kind::Pad),
            "port `{p}` of foreign `{module}` is not an In, an Out or a Pad"
        );
    }
    Lowered {
        name: name.to_string(),
        fields: Vec::new(),
        ports: ports
            .iter()
            .map(|(p, k, w)| (p.to_string(), *k, *w))
            .collect(),
        wires: Vec::new(),
        procs: Vec::new(),
        init: Vec::new(),
        aliases: Vec::new(),
        nets: Vec::new(),
        instances: Vec::new(),
        foreign: Some(Foreign {
            module: module.to_string(),
            params: params.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
            clocks: clocks.iter().map(|(p, c)| (p.to_string(), *c)).collect(),
        }),
    }
}

/// A child of a unit of units: the field it lives in, its own
/// lowering, and what each of its ports is joined to in the parent,
/// a net or a port of the parent.
pub struct Instance {
    /// The field the child lives in, which names the instance.
    pub name: String,
    /// The child's own lowering, rendered as a module of its own.
    pub unit: Lowered,
    /// What each of the child's ports is joined to: the child's port
    /// name, and the parent's net or port.
    pub conns: Vec<(String, String)>,
}

/// What `#[lower]` gives every unit, so a unit of units reaches a
/// child's lowering through the child's type; the inherent `lowered`
/// keeps its name, and this one differs so the two never shadow.
pub trait Lower {
    /// This unit's lowering, under the module name given.
    fn lowered_as(name: &str) -> Lowered;
}

/// A child's lowering under a module name, from a value of its type:
/// what a parent's generated `lowered` calls on each field, since it
/// knows the field and not the type.
pub fn child_lowered<U: Lower>(_: &U, name: &str) -> Lowered {
    U::lowered_as(name)
}

/// A child joined to the parent: what the parent passed, flattened
/// in the order `run` names it, each with the child's port it joins.
/// An empty port name joins the child's port at that position; a
/// port named, a field of a struct the parent passed, joins the port
/// of that name. The counts must agree, or the parent passed a tuple
/// of the wrong shape, and no port is joined twice.
pub fn instance(unit: Lowered, name: &str, args: &[(&str, &str)]) -> Instance {
    assert_eq!(
        unit.ports.len(),
        args.len(),
        "`{name}` has {} ports and is joined to {} of the parent's",
        unit.ports.len(),
        args.len()
    );
    let mut conns: Vec<(String, String)> = Vec::new();
    for ((p, _, _), (by, a)) in unit.ports.iter().zip(args) {
        let port = if by.is_empty() {
            p.clone()
        } else {
            assert!(
                unit.ports.iter().any(|(n, _, _)| n == by),
                "`{name}` has no port `{by}`"
            );
            by.to_string()
        };
        assert!(
            !conns.iter().any(|(c, _)| *c == port),
            "port `{port}` of `{name}` is joined twice"
        );
        conns.push((port, a.to_string()));
    }
    Instance {
        name: name.to_string(),
        unit,
        conns,
    }
}

impl Lowered {
    /// The clocks the processes wait for, and the children's, each
    /// once, in order.
    fn clocks(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        if let Some(f) = &self.foreign {
            for (_, c) in &f.clocks {
                if !out.contains(c) {
                    out.push(c);
                }
            }
        }
        for p in &self.procs {
            if !out.contains(&p.clock) {
                out.push(p.clock);
            }
        }
        for i in &self.instances {
            for c in i.unit.clocks() {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }
    /// The nets of a port or a net, by kind: a channel's three, a
    /// wire's one.
    fn strands(k: &Kind) -> &'static [&'static str] {
        match k {
            Kind::Tx | Kind::Rx => &["_data", "_valid", "_ready"],
            _ => &[""],
        }
    }
    /// A child's ports joined to the parent's nets, strand by strand:
    /// (child net, parent net) pairs, in port order. A channel net has
    /// a sender's side and a receiver's side, and the child's port
    /// kind says which it is on; a parent port is joined by name.
    fn joins(&self, inst: &Instance) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (p, a) in &inst.conns {
            let (_, k, _) = inst
                .unit
                .ports
                .iter()
                .find(|(n, _, _)| n == p)
                .expect("a joined port is a port of the child");
            let chan_net = self
                .nets
                .iter()
                .any(|(n, nk, _, _)| n == a && matches!(nk, Kind::Tx));
            let side = match (chan_net, k) {
                (true, Kind::Tx) => "_tx",
                (true, Kind::Rx) => "_rx",
                _ => "",
            };
            for s in Self::strands(k) {
                out.push((format!("{p}{s}"), format!("{a}{side}{s}")));
            }
        }
        out
    }
    /// Whether this unit or any child joins children by a channel,
    /// so the netlist needs the channel module once.
    fn has_chan_nets(&self) -> bool {
        self.nets.iter().any(|(_, k, _, _)| matches!(k, Kind::Tx))
            || self.instances.iter().any(|i| i.unit.has_chan_nets())
    }
    /// Whether a register is driven by a falling-edge process.
    fn falling_reg(&self, name: &str) -> bool {
        fn drives(d: &[(Target, Expr)], name: &str) -> bool {
            d.iter()
                .any(|(t, _)| matches!(t, Target::Name(n) if n == name))
        }
        fn in_stmt(st: &Stmt, name: &str) -> bool {
            match st {
                Stmt::Drive(Target::Name(n), _) => n == name,
                Stmt::Drive(_, _) | Stmt::Guard(_) => false,
                Stmt::When(_, a, b) => drives(a, name) || drives(b, name),
                Stmt::Case(arms) => arms.iter().any(|(_, d)| drives(d, name)),
                Stmt::If(arms, els) => {
                    arms.iter().any(|(_, b)| b.iter().any(|s| in_stmt(s, name)))
                        || els.iter().any(|s| in_stmt(s, name))
                }
            }
        }
        self.procs
            .iter()
            .any(|p| p.falling && p.body.iter().any(|st| in_stmt(st, name)))
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
        fn target(
            x: &Target,
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Target {
            match x {
                Target::Word(m, a) => Target::Word(m.clone(), go(a, l, t)),
                n => n.clone(),
            }
        }
        fn drives(
            d: &[(Target, Expr)],
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Vec<(Target, Expr)> {
            d.iter()
                .map(|(x, e)| (target(x, l, t), go(e, l, t)))
                .collect::<Vec<_>>()
        }
        let wires = self
            .wires
            .iter()
            .map(|(n, e)| (n.clone(), go(e, self, &mut temps)))
            .collect();
        fn stmt(
            st: &Stmt,
            l: &Lowered,
            temps: &mut Vec<(String, usize, Expr)>,
        ) -> Stmt {
            match st {
                Stmt::Drive(x, e) => {
                    Stmt::Drive(target(x, l, temps), go(e, l, temps))
                }
                Stmt::When(c, a, b) => Stmt::When(
                    go(c, l, temps),
                    drives(a, l, temps),
                    drives(b, l, temps),
                ),
                Stmt::Case(arms) => Stmt::Case(
                    arms.iter()
                        .map(|(c, d)| (go(c, l, temps), drives(d, l, temps)))
                        .collect(),
                ),
                Stmt::If(arms, els) => Stmt::If(
                    arms.iter()
                        .map(|(c, b)| {
                            (
                                go(c, l, temps),
                                b.iter().map(|s| stmt(s, l, temps)).collect(),
                            )
                        })
                        .collect(),
                    els.iter().map(|s| stmt(s, l, temps)).collect(),
                ),
                Stmt::Guard(c) => Stmt::Guard(go(c, l, temps)),
            }
        }
        // An `if` on a constant is folded: a condition that is one
        // keeps its arm and drops the test, one that is zero drops
        // the arm, so a condition of the build costs nothing in the
        // netlist, as it costs nothing in the run.
        fn fold(st: Stmt, out: &mut Vec<Stmt>) {
            let Stmt::If(arms, els) = st else {
                out.push(st);
                return;
            };
            let folded = |body: Vec<Stmt>| {
                body.into_iter().fold(Vec::new(), |mut v, s| {
                    fold(s, &mut v);
                    v
                })
            };
            let mut kept: Vec<(Expr, Vec<Stmt>)> = Vec::new();
            let mut els = folded(els);
            for (c, body) in arms {
                match c.constant() {
                    Some(false) => continue,
                    Some(true) => {
                        els = folded(body);
                        break;
                    }
                    None => kept.push((c, folded(body))),
                }
            }
            if kept.is_empty() {
                out.extend(els);
            } else {
                out.push(Stmt::If(kept, els));
            }
        }
        let mut procs = Vec::new();
        for p in &self.procs {
            let body = p.body.iter().map(|st| stmt(st, self, &mut temps)).fold(
                Vec::new(),
                |mut v, s| {
                    fold(s, &mut v);
                    v
                },
            );
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
                Kind::Pad => out.push_str(&format!("{n} inout {w}{s}\n")),
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
        let mut out = self.verilog_in();
        if self.has_chan_nets() {
            out.push('\n');
            out.push_str(CHAN_VERILOG);
        }
        out
    }
    /// This unit's module and its children's, without the channel
    /// module, which the outermost unit adds once.
    fn verilog_in(&self) -> String {
        // A foreign module comes as its own source.
        if self.foreign.is_some() {
            return String::new();
        }
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
                Kind::Pad => plist.push(format!("inout {}{n}", range(*w))),
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
        // A unit of units: the nets between the children, a channel
        // being a buffer between its two sides, then each child as an
        // instance of its own module, joined port by port.
        for (n, k, w, c) in &self.nets {
            match k {
                Kind::Tx | Kind::Rx => writeln!(
                    out,
                    "  wire {r}{n}_tx_data;\n  wire {n}_tx_valid;\n  \
                     wire {n}_tx_ready;\n  \
                     wire {r}{n}_rx_data;\n  wire {n}_rx_valid;\n  \
                     wire {n}_rx_ready;\n  \
                     txhdl_chan #(.W({w})) {n}_chan(\n    .clk({c}),\n    \
                     .tx_data({n}_tx_data), .tx_valid({n}_tx_valid), \
                     .tx_ready({n}_tx_ready),\n    \
                     .rx_data({n}_rx_data), .rx_valid({n}_rx_valid), \
                     .rx_ready({n}_rx_ready)\n  );",
                    r = range(*w)
                )
                .unwrap(),
                _ => writeln!(out, "  wire {}{n};", range(*w)).unwrap(),
            }
        }
        for inst in &self.instances {
            // A lowered child's clock ports are named for their clocks;
            // a foreign child's have names of their own.
            let mut conns: Vec<String> = match &inst.unit.foreign {
                Some(f) => {
                    f.clocks.iter().map(|(p, c)| format!(".{p}({c})")).collect()
                }
                None => inst
                    .unit
                    .clocks()
                    .iter()
                    .map(|c| format!(".{c}({c})"))
                    .collect(),
            };
            for (a, b) in self.joins(inst) {
                conns.push(format!(".{a}({b})"));
            }
            // A foreign child is its module, with its parameters.
            let (module, params) = match &inst.unit.foreign {
                Some(f) if !f.params.is_empty() => {
                    let ps: Vec<String> = f
                        .params
                        .iter()
                        .map(|(p, v)| format!(".{p}({v})"))
                        .collect();
                    (&f.module, format!("#(\n    {}\n  ) ", ps.join(",\n    ")))
                }
                Some(f) => (&f.module, String::new()),
                None => (&inst.unit.name, String::new()),
            };
            writeln!(
                out,
                "  {module} {params}{}(\n    {}\n  );",
                inst.name,
                conns.join(",\n    ")
            )
            .unwrap();
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
        // A statement of a clocked block, at an indentation; a
        // conditional holds statements of its own, one level in.
        type Drive<'a> = &'a dyn Fn(
            &mut Vec<String>,
            &mut Vec<String>,
            &str,
            &Target,
            &Expr,
        );
        fn stmt(
            seq: &mut Vec<String>,
            comb: &mut Vec<String>,
            ind: &str,
            st: &Stmt,
            drive: Drive,
            l: &Lowered,
        ) {
            let inner = format!("{ind}  ");
            match st {
                Stmt::Guard(_) => {}
                Stmt::Drive(t, e) => drive(seq, comb, ind, t, e),
                Stmt::When(c, then, otherwise) => {
                    seq.push(format!("{ind}if ({}) begin", vexpr(c, l)));
                    for (t, e) in then {
                        drive(seq, comb, &inner, t, e);
                    }
                    if !otherwise.is_empty() {
                        seq.push(format!("{ind}end else begin"));
                        for (t, e) in otherwise {
                            drive(seq, comb, &inner, t, e);
                        }
                    }
                    seq.push(format!("{ind}end"));
                }
                Stmt::Case(arms) => {
                    for (i, (c, drives)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "end else if" };
                        seq.push(format!("{ind}{kw} ({}) begin", vexpr(c, l)));
                        for (t, e) in drives {
                            drive(seq, comb, &inner, t, e);
                        }
                    }
                    if !arms.is_empty() {
                        seq.push(format!("{ind}end"));
                    }
                }
                Stmt::If(arms, els) => {
                    for (i, (c, body)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "end else if" };
                        seq.push(format!("{ind}{kw} ({}) begin", vexpr(c, l)));
                        for s in body {
                            stmt(seq, comb, &inner, s, drive, l);
                        }
                    }
                    if !els.is_empty() {
                        seq.push(format!("{ind}end else begin"));
                        for s in els {
                            stmt(seq, comb, &inner, s, drive, l);
                        }
                    }
                    seq.push(format!("{ind}end"));
                }
            }
        }
        // A clocked block per process, on its clock and its edge.
        for p in &procs {
            let mut seq: Vec<String> = Vec::new();
            let mut guard = false;
            for st in &p.body {
                if let Stmt::Guard(c) = st {
                    guard = true;
                    seq.push(format!("    if ({}) begin", vexpr(c, l)));
                    continue;
                }
                let ind = if guard { "      " } else { "    " };
                stmt(&mut seq, &mut comb, ind, st, &drive, l);
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
        // The children's modules follow the parent's, each whole.
        for inst in &self.instances {
            out.push('\n');
            out.push_str(&inst.unit.verilog_in());
        }
        out
    }

    /// The VHDL, 2008: an entity, one process on the rising edge for
    /// the registers, a concurrent assignment per wire.
    pub fn vhdl(&self) -> String {
        let mut out = String::new();
        if self.has_chan_nets() {
            out.push_str(CHAN_VHDL);
            out.push('\n');
        }
        out.push_str(&self.vhdl_in());
        out
    }
    /// This unit's entity and its children's, without the channel
    /// entity, which the outermost unit puts first once.
    fn vhdl_in(&self) -> String {
        // A foreign entity comes as its own source.
        if self.foreign.is_some() {
            return String::new();
        }
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
                Kind::Pad => plist.push(format!("{n} : inout {}", logic(*w))),
                Kind::Reg | Kind::Mem | Kind::Wire => {}
            }
        }
        let mut out = String::new();
        // The children's entities come first, since an entity is
        // analysed before it is instantiated.
        for inst in &self.instances {
            let child = inst.unit.vhdl_in();
            if !child.is_empty() {
                out.push_str(&child);
                out.push('\n');
            }
        }
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
        for (n, k, w, _) in &self.nets {
            match k {
                Kind::Tx | Kind::Rx => writeln!(
                    out,
                    "  signal {n}_tx_data, {n}_rx_data : {};\n  \
                     signal {n}_tx_valid, {n}_tx_ready, {n}_rx_valid, \
                     {n}_rx_ready : std_logic;",
                    ty(*w)
                )
                .unwrap(),
                _ => writeln!(out, "  signal {n} : {};", ty(*w)).unwrap(),
            }
        }
        // A foreign child is a component, declared once per module: a
        // Verilog module is reached from VHDL no other way, and an
        // entity from elsewhere may not be in `work`.
        let mut declared: Vec<&str> = Vec::new();
        for inst in &self.instances {
            let Some(f) = &inst.unit.foreign else {
                continue;
            };
            if declared.contains(&f.module.as_str()) {
                continue;
            }
            declared.push(&f.module);
            let generics: Vec<String> = f
                .params
                .iter()
                .map(|(p, _)| format!("{p} : integer"))
                .collect();
            let ports: Vec<String> = f
                .clocks
                .iter()
                .map(|(p, _)| format!("{p} : in std_logic"))
                .chain(inst.unit.ports.iter().map(|(p, k, w)| {
                    let dir = match k {
                        Kind::Out => "out",
                        Kind::Pad => "inout",
                        _ => "in",
                    };
                    format!("{p} : {dir} {}", logic(*w))
                }))
                .collect();
            writeln!(out, "  component {}", f.module).unwrap();
            if !generics.is_empty() {
                writeln!(out, "    generic ({});", generics.join("; "))
                    .unwrap();
            }
            writeln!(
                out,
                "    port (\n      {}\n    );\n  end component;",
                ports.join(";\n      ")
            )
            .unwrap();
        }
        writeln!(out, "begin").unwrap();
        for (n, k, w, c) in &self.nets {
            if matches!(k, Kind::Tx | Kind::Rx) {
                writeln!(
                    out,
                    "  {n}_chan : entity work.txhdl_chan \
                     generic map (W => {w}) port map (\n    clk => {c},\n    \
                     tx_data => {n}_tx_data, tx_valid => {n}_tx_valid, \
                     tx_ready => {n}_tx_ready,\n    \
                     rx_data => {n}_rx_data, rx_valid => {n}_rx_valid, \
                     rx_ready => {n}_rx_ready\n  );"
                )
                .unwrap();
            }
        }
        // A unit of units: each child an instance of its entity,
        // joined port by port to the nets and the parent's ports.
        for inst in &self.instances {
            let mut conns: Vec<String> = match &inst.unit.foreign {
                Some(f) => f
                    .clocks
                    .iter()
                    .map(|(p, c)| format!("{p} => {c}"))
                    .collect(),
                None => inst
                    .unit
                    .clocks()
                    .iter()
                    .map(|c| format!("{c} => {c}"))
                    .collect(),
            };
            if let Some(f) = &inst.unit.foreign {
                // The component's vectors are `std_logic_vector` and the
                // netlist's are `unsigned`, so a vector crosses with a
                // conversion: on the actual for an input, on the formal
                // for an output. A pad is `std_logic_vector` on both.
                let joins = self.joins(inst);
                for (a, b) in &joins {
                    let (_, k, w) = inst
                        .unit
                        .ports
                        .iter()
                        .find(|(n, _, _)| n == a)
                        .expect("a joined port is a port of the child");
                    conns.push(match k {
                        Kind::In if *w > 1 => {
                            format!("{a} => std_logic_vector({b})")
                        }
                        Kind::Out if *w > 1 => format!("unsigned({a}) => {b}"),
                        _ => format!("{a} => {b}"),
                    });
                }
                let generic = if f.params.is_empty() {
                    String::new()
                } else {
                    let gs: Vec<String> = f
                        .params
                        .iter()
                        .map(|(p, v)| format!("{p} => {v}"))
                        .collect();
                    format!(" generic map ({})", gs.join(", "))
                };
                writeln!(
                    out,
                    "  {} : {}{generic} port map (\n    {}\n  );",
                    inst.name,
                    f.module,
                    conns.join(",\n    ")
                )
                .unwrap();
                continue;
            }
            for (a, b) in self.joins(inst) {
                conns.push(format!("{a} => {b}"));
            }
            writeln!(
                out,
                "  {} : entity work.{} port map (\n    {}\n  );",
                inst.name,
                inst.unit.name,
                conns.join(",\n    ")
            )
            .unwrap();
        }
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
        // A statement of a process, at an indentation; a conditional
        // holds statements of its own, one level in.
        type Drive<'a> = &'a dyn Fn(
            &mut Vec<String>,
            &mut Vec<String>,
            &str,
            &Target,
            &Expr,
        );
        fn stmt(
            seq: &mut Vec<String>,
            comb: &mut Vec<String>,
            ind: &str,
            st: &Stmt,
            drive: Drive,
            l: &Lowered,
        ) {
            let inner = format!("{ind}  ");
            match st {
                Stmt::Guard(_) => {}
                Stmt::Drive(t, e) => drive(seq, comb, ind, t, e),
                Stmt::When(c, then, otherwise) => {
                    seq.push(format!("{ind}if {} then", hbool(c, l)));
                    for (t, e) in then {
                        drive(seq, comb, &inner, t, e);
                    }
                    if !otherwise.is_empty() {
                        seq.push(format!("{ind}else"));
                        for (t, e) in otherwise {
                            drive(seq, comb, &inner, t, e);
                        }
                    }
                    seq.push(format!("{ind}end if;"));
                }
                Stmt::Case(arms) => {
                    for (i, (c, drives)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "elsif" };
                        seq.push(format!("{ind}{kw} {} then", hbool(c, l)));
                        for (t, e) in drives {
                            drive(seq, comb, &inner, t, e);
                        }
                    }
                    if !arms.is_empty() {
                        seq.push(format!("{ind}end if;"));
                    }
                }
                Stmt::If(arms, els) => {
                    for (i, (c, body)) in arms.iter().enumerate() {
                        let kw = if i == 0 { "if" } else { "elsif" };
                        seq.push(format!("{ind}{kw} {} then", hbool(c, l)));
                        for s in body {
                            stmt(seq, comb, &inner, s, drive, l);
                        }
                    }
                    if !els.is_empty() {
                        seq.push(format!("{ind}else"));
                        for s in els {
                            stmt(seq, comb, &inner, s, drive, l);
                        }
                    }
                    seq.push(format!("{ind}end if;"));
                }
            }
        }
        // A process per process, on its clock and its edge.
        for p in &procs {
            let mut seq: Vec<String> = Vec::new();
            let mut guard = false;
            for st in &p.body {
                if let Stmt::Guard(c) = st {
                    guard = true;
                    seq.push(format!("      if {} then", hbool(c, self)));
                    continue;
                }
                let ind = if guard { "        " } else { "      " };
                stmt(&mut seq, &mut comb, ind, st, &drive, self);
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

/// The channel between two lowered units, as the runtime has it: a
/// buffer of two, `head` and `tail`, the receiver's `valid` and
/// `data` the head as the edge left it, the sender's `ready` the
/// tail's room; a take moves the tail up and an offer fills the
/// first free place, the take first. In Verilog, a module.
const CHAN_VERILOG: &str = "`timescale 1ns/1ps
module txhdl_chan #(parameter W = 1)(
  input clk,
  input [W-1:0] tx_data, input tx_valid, output tx_ready,
  output [W-1:0] rx_data, output rx_valid, input rx_ready
);
  reg [W-1:0] head = 0;
  reg [W-1:0] tail = 0;
  reg head_v = 0;
  reg tail_v = 0;
  wire pop = rx_ready & head_v;
  wire hv1 = pop ? tail_v : head_v;
  wire [W-1:0] h1 = pop ? tail : head;
  wire tv1 = pop ? 1'b0 : tail_v;
  always @(posedge clk) begin
    head_v <= hv1 | tx_valid;
    head <= (tx_valid & ~hv1) ? tx_data : h1;
    tail_v <= tv1 | (tx_valid & hv1);
    tail <= (tx_valid & hv1) ? tx_data : tail;
  end
  assign rx_data = head;
  assign rx_valid = head_v;
  assign tx_ready = ~tail_v;
endmodule
";

/// The same channel in VHDL, an entity with the width as a generic.
const CHAN_VHDL: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity txhdl_chan is
  generic (W : natural);
  port (
    clk : in std_logic;
    tx_data : in unsigned(W - 1 downto 0);
    tx_valid : in std_logic;
    tx_ready : out std_logic;
    rx_data : out unsigned(W - 1 downto 0);
    rx_valid : out std_logic;
    rx_ready : in std_logic
  );
end entity;

architecture rtl of txhdl_chan is
  signal head, tail : unsigned(W - 1 downto 0) := (others => '0');
  signal head_v, tail_v : std_logic := '0';
begin
  process (clk)
    variable h, t : unsigned(W - 1 downto 0);
    variable hv, tv : std_logic;
  begin
    if rising_edge(clk) then
      h := head; t := tail; hv := head_v; tv := tail_v;
      if rx_ready = '1' and head_v = '1' then
        h := tail; hv := tail_v; tv := '0';
      end if;
      if tx_valid = '1' then
        if hv = '0' then h := tx_data; hv := '1';
        else t := tx_data; tv := '1'; end if;
      end if;
      head <= h; tail <= t; head_v <= hv; tail_v <= tv;
    end if;
  end process;
  rx_data <= head;
  rx_valid <= head_v;
  tx_ready <= not tail_v;
end architecture;
";

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
            // A number against a vector is sized to it: `a & 63` is
            // `a and to_unsigned(63, 8)`, since numeric_std has no
            // logic between an unsigned and an integer.
            let side = |x: &Expr| match x {
                Expr::Num(k) if w > 1 => format!("to_unsigned({k}, {w})"),
                x => hval(x, w, l),
            };
            format!("({} {vop} {})", side(a), side(b))
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
            _ => format!("{}({})", hval(a, 0, l), hint(i, l)),
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
