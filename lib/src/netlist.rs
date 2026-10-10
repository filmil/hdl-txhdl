// SPDX-License-Identifier: Apache-2.0
//! A structural netlist, from the same walk the waveform uses. Every
//! unit is a module, every register a `reg`, and every wire or channel
//! whose ends are fields of different units is a net between them,
//! with a port on each and a wire in their common parent. The walk
//! sees fields, so a unit whose ports are fields yields ports, and a
//! unit that takes its ports as parameters of `run` yields only its
//! registers: that is the gap between a structural netlist and the
//! lowering, which the proc-macro route closes by reading the
//! `Unit` impl. The structural walk carries no behaviour, and its
//! bodies are empty; the behaviour is the rest of this module:
//! [`Lowered`], which `#[lower]` fills from the `Unit` impl with the
//! unit's ports, registers and processes, and the Verilog and VHDL
//! written from it.
use crate::comp::trace::{collect, Kind, Probe, Traceable};
use crate::comp::{Clock, DefaultClock, In, Mem, Out, Pad, Reg, Rx, Tx, Wire};
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
                    .is_some_and(|r| r.starts_with('.'))
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
    /// The netlist's fields this one is, under the name `name`: itself
    /// alone, or for an array of registers `name_0` onward (issue 594).
    fn entries(
        name: &'static str,
    ) -> Vec<(&'static str, Option<Kind>, usize, usize)> {
        vec![(name, Self::KIND, Self::WIDTH, Self::DEPTH)]
    }
}
impl<T: Value + Copy + 'static, C: Clock> Port for Reg<T, C> {
    const KIND: Option<Kind> = Some(Kind::Reg);
    const WIDTH: usize = T::WIDTH;
}
/// An array of child units is no field of the netlist: its children are
/// instances, which the parent's lowering makes (issue 635).
impl<T, const N: usize> Port for crate::comp::Units<T, N> {}
impl<T: Value + Copy + 'static, const N: usize, C: Clock> Port
    for crate::comp::Regs<T, N, C>
{
    fn entries(
        name: &'static str,
    ) -> Vec<(&'static str, Option<Kind>, usize, usize)> {
        // A field's name is `&'static str`, and these are made when the
        // netlist is, once per unit lowered, so they are leaked.
        (0..N)
            .map(|i| {
                let n: &'static str =
                    Box::leak(format!("{name}_{i}").into_boxed_str());
                (n, Some(Kind::Reg), T::WIDTH, 0)
            })
            .collect()
    }
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
impl<const N: usize, const L: usize> Port for crate::types::U<N, L> {}

/// A unit's fields by name, kind, width and depth. Derived with
/// `Trace`.
pub trait Fields {
    /// The names of the unit's fields, in order, for the checks
    /// `#[lower]` writes against the names of its wires and ports.
    /// A field carrying `#[rename("...")]` is here under the name it
    /// takes in the netlist, not the name Rust knows it by.
    const NAMES: &'static [&'static str];
    /// Every field the netlist names differently from Rust, as the
    /// Rust name and the netlist name. `#[rename("...")]` on a field
    /// puts it here, and the lowering rewrites its references through
    /// this table (issue 222). Empty when nothing is renamed.
    const RENAMES: &'static [(&'static str, &'static str)] = &[];
    /// The registers that sample a value made on another clock: the
    /// first two stages of a synchroniser. `#[async_reg]` on a field
    /// puts it here, under its netlist name, and the netlist marks
    /// it `ASYNC_REG`, so that Vivado places the stages together and
    /// reports them as a synchroniser (#884). Empty when there are
    /// none.
    const ASYNC_REGS: &'static [&'static str] = &[];
    /// The memories meant to be distributed RAM, not block RAM:
    /// `#[distributed]` on a field puts it here, and the check that a
    /// memory of more than 4096 bits has the ports a block RAM has
    /// leaves it alone (issue 1285). Empty when there are none.
    const DISTRIBUTED: &'static [&'static str] = &[];
    /// What each memory asks Vivado to make it, by name:
    /// `#[ram_style("block")]` on a field, or `"distributed"`,
    /// `"registers"` or `"ultra"`, and `#[distributed]` as
    /// `"distributed"` (issue 1371). The netlists say it as the
    /// `ram_style` attribute. Empty when there are none.
    const RAM_STYLES: &'static [(&'static str, &'static str)] = &[];
    /// Whether Vivado may build what drives each register from DSP
    /// slices, by name: `#[use_dsp("no")]` on a field, or `"yes"`,
    /// `"logic"` or `"simd"` (issue 1383). The netlists say it as the
    /// `use_dsp` attribute. Empty when there are none.
    const USE_DSP: &'static [(&'static str, &'static str)] = &[];
    /// Every field of the unit, as its name, what it is, how wide it
    /// is, and how many words it holds if it is a memory.
    fn fields() -> Vec<(&'static str, Option<Kind>, usize, usize)>;
}

/// The name a wire takes: `want`, unless a field of the unit already
/// has that name, in which case `alt`. `#[lower]` writes one of these
/// per computed `let`, so the choice is made for every type the unit
/// is lowered at, generic or not, and a `let` that shadows a register
/// lowers rather than being refused. The netlist says which wires
/// were renamed. This is issue 171.
#[doc(hidden)]
pub const fn wire_name(
    names: &[&'static str],
    want: &'static str,
    alt: &'static str,
) -> &'static str {
    if has_name(names, want) {
        alt
    } else {
        want
    }
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
                Expr::index(e, Expr::Num(hi as u128))
            } else {
                Expr::slice(e, hi, w)
            };
        }
    }
    panic!("`{name}` is not a field of the value")
}

/// A struct literal on a wire: its fields' values, named as the literal
/// names them, concatenated in the order the struct declares them, the
/// first field highest, as `#[derive(Value)]` lays them out (issue 1541).
/// A field the literal leaves out or one the struct does not have is a
/// mistake the Rust compiler has already refused in the run, so it is
/// said here rather than lowered.
pub fn literal(
    order: &crate::types::Fields,
    fields: Vec<(&str, Expr)>,
) -> Expr {
    assert_eq!(
        fields.len(),
        order.0.len(),
        "a struct literal names {} fields of {}",
        fields.len(),
        order.0.len()
    );
    let mut fields: Vec<(&str, Option<Expr>)> =
        fields.into_iter().map(|(n, e)| (n, Some(e))).collect();
    let mut acc: Option<Expr> = None;
    for want in order.0 {
        let e = fields
            .iter_mut()
            .find(|(n, _)| n == want)
            .and_then(|(_, e)| e.take())
            .unwrap_or_else(|| panic!("a struct literal leaves out {want}"));
        acc = Some(match acc {
            None => e,
            Some(a) => Expr::Cat(Box::new(a), Box::new(e)),
        });
    }
    acc.expect("a struct with no fields")
}

/// One end a unit may take as a port, as the netlist needs it: what
/// it is, how wide, on which clock, and how its value is laid out.
/// `#[derive(Ports)]` reads a struct's fields through this, so a
/// struct of ports declared anywhere, in any crate, is a side of a
/// unit (issue 483).
pub trait PortEnd {
    /// `In`, `Out`, `Tx`, `Rx` or `Pad`.
    const KIND: Kind;
    /// The width of the value, in bits.
    const WIDTH: usize;
    /// The name of the clock the end is on.
    const CLOCK: &'static str;
    /// The fields of the value, as [`Value::layout`] gives them.
    fn layout() -> Vec<(&'static str, usize)>;
}
macro_rules! port_end {
    ($t:ident, $k:ident, $($b:tt)*) => {
        impl<T: Value + $($b)*, C: Clock> PortEnd for $t<T, C> {
            const KIND: Kind = Kind::$k;
            const WIDTH: usize = T::WIDTH;
            const CLOCK: &'static str = C::NAME;
            fn layout() -> Vec<(&'static str, usize)> {
                T::layout()
            }
        }
        impl<T: Value + $($b)*, C: Clock> PortField for $t<T, C> {
            fn ports_of(name: &str) -> Vec<BundlePort> {
                vec![bundle_port::<Self>(name)]
            }
        }
    };
}
/// What a field of a struct of ports is: one port, or, when the field
/// is itself a struct of ports, every port of it, each named for the
/// field and its own name, `field_sub` (issue 498). `#[derive(Ports)]`
/// lists its fields through this, so a struct of ports may hold another
/// and the netlist sees them flattened.
pub trait PortField {
    /// The ports of a field of this type called `name`.
    fn ports_of(name: &str) -> Vec<BundlePort>;
}

port_end!(In, In, Copy + 'static);
port_end!(Out, Out, Copy + 'static);
port_end!(Pad, Pad, Copy + 'static);
port_end!(Tx, Tx, crate::types::Transaction + 'static);
port_end!(Rx, Rx, crate::types::Transaction + 'static);

thread_local! {
    /// The wires the last hoisting pass made, `slN`, with their widths:
    /// a name the lowered unit does not hold, read while that pass and
    /// its emitter run (issue 683).
    static HOISTED: std::cell::RefCell<Vec<(String, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Of those wires, the products under a register's `#[use_dsp]`,
    /// each with the register's word on DSP slices (issue 1343).
    static DSP_WIRES: std::cell::RefCell<Vec<(String, &'static str)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The wires the helpers called in the unit being lowered asked
    /// for, a frame per unit: `lowered` opens one, and a child lowered
    /// inside it opens its own, so the two do not mix (issue 504).
    static INLINE: std::cell::RefCell<Vec<(Vec<(String, Expr)>, usize)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// What a wire the hoisting pass made says of DSP slices: the word of
/// the register whose product it is (issue 1343).
fn dsp_wire(n: &str) -> Option<&'static str> {
    DSP_WIRES.with(|h| h.borrow().iter().find(|(m, _)| m == n).map(|(_, v)| *v))
}

/// Open a frame for the wires the helpers of one unit ask for. What a
/// generated `lowered` does first.
#[doc(hidden)]
pub fn inlined_begin() {
    INLINE.with(|f| f.borrow_mut().push((Vec::new(), 0)));
}

/// Close the frame [`inlined_begin`] opened and give its wires.
#[doc(hidden)]
pub fn inlined_end() -> Vec<(String, Expr)> {
    INLINE.with(|f| f.borrow_mut().pop().map(|(w, _)| w).unwrap_or_default())
}

/// A value bound in a helper that `#[lower]` reached through its
/// `lowered`, from another file: the expression itself when it is read
/// once or costs nothing to read again, and otherwise a wire of the unit
/// that holds it, so that a chain of calls grows by a wire a call and
/// not by a power of its reads, as a helper of the unit's own file does
/// (issues 126 and 504).
#[doc(hidden)]
pub fn inline_bind(hint: &str, e: Expr, reads: usize) -> Expr {
    let cheap = matches!(e, Expr::Name(_) | Expr::Num(_) | Expr::Bits(..));
    if reads < 2 || cheap {
        return e;
    }
    INLINE.with(|f| {
        let mut f = f.borrow_mut();
        match f.last_mut() {
            Some((wires, n)) => {
                *n += 1;
                let name = format!("{hint}_i{n}");
                wires.push((name.clone(), e));
                Expr::Name(name)
            }
            None => e,
        }
    })
}

/// A port of a struct of ports: its field's name and what
/// [`PortEnd`] says of the field's type.
pub struct BundlePort {
    /// The field's name, which is the port's name in the netlist: a
    /// struct's field, or an array's index (issue 500); a port of a
    /// nested struct of ports is `field_sub` (issue 498).
    pub name: String,
    /// What the port is.
    pub kind: Kind,
    /// Its width.
    pub width: usize,
    /// Its clock.
    pub clock: &'static str,
    /// The layout of its value, for a field of it read in a body.
    pub layout: Vec<(&'static str, usize)>,
}

/// The port a field of type `P` named `name` is.
pub fn bundle_port<P: PortEnd>(name: &str) -> BundlePort {
    BundlePort {
        name: name.to_string(),
        kind: P::KIND,
        width: P::WIDTH,
        clock: P::CLOCK,
        layout: P::layout(),
    }
}

/// A struct whose fields are ports, so that one name on a side of
/// `run` stands for all of them: `bus: LitePort<32, 32, 4>` and
/// `bus.ar` in the body. Derived with `#[derive(Ports)]`; the fields
/// are the ports in declaration order, each named `side_field`, so
/// `bus.ar` is the port `bus_ar` of the netlist.
pub trait Ports {
    /// The ports, in declaration order.
    fn ports() -> Vec<BundlePort>;
}

/// An array of ports is a struct of ports whose fields are its indices:
/// `ins: [Rx<T>; N]` has the ports `ins_0` to `ins_{N-1}`, and
/// `ins[i]` in a loop the lowering unrolls is `ins_{i}` (issue 500).
impl<P: PortEnd, const N: usize> Ports for [P; N] {
    fn ports() -> Vec<BundlePort> {
        (0..N).map(|i| bundle_port::<P>(&i.to_string())).collect()
    }
}

/// An array of ports is a port field as a struct of ports is, so a
/// struct of ports may hold one: `ins_0` to `ins_{N-1}` (issues 498 and
/// 500).
impl<P: PortEnd, const N: usize> PortField for [P; N] {
    fn ports_of(name: &str) -> Vec<BundlePort> {
        <Self as Ports>::ports()
            .into_iter()
            .map(|mut p| {
                p.name = format!("{name}_{}", p.name);
                p
            })
            .collect()
    }
}

/// The ports of `B` as a lowered unit lists its own, when the unit
/// takes it as the side `side`: each named `side_field`, so a unit can
/// take two sides of one type and their ports stay apart.
#[doc(hidden)]
pub fn bundle_ports<B: Ports>(
    side: &str,
) -> Vec<(String, Kind, usize, &'static str)> {
    B::ports()
        .into_iter()
        .map(|p| (format!("{side}_{}", p.name), p.kind, p.width, p.clock))
        .collect()
}

/// What a unit of units passes a child when it passes the whole of its
/// side `side`, whose struct `B` is declared in another file: the side's
/// ports in the order `B` declares them, each joined to the child's
/// port at that position (issue 498). `#[lower]` cannot see `B`'s
/// fields, so the join is made here, when `lowered` runs.
#[doc(hidden)]
pub fn bundle_args<B: Ports>(side: &str) -> Vec<(String, String)> {
    bundle_ports::<B>(side)
        .into_iter()
        .map(|(n, _, _, _)| (String::new(), n))
        .collect()
}

/// What a unit of units passes a child's field `port`, itself a struct
/// of ports, when it gives it the whole of `side`, a struct of ports of
/// the same type nested in its own: each port of `B`, the child's
/// `port_name` joined to the parent's `side_name` (issue 579).
#[doc(hidden)]
pub fn bundle_args_named<B: Ports>(
    port: &str,
    side: &str,
) -> Vec<(String, String)> {
    B::ports()
        .into_iter()
        .map(|p| (format!("{port}_{}", p.name), format!("{side}_{}", p.name)))
        .collect()
}

/// What a unit of units passes a child when it passes `side.path` of a
/// side whose struct `B` is declared in another file: the one port
/// named `path`, or, where `path` is a struct of ports nested in `B`,
/// each of its ports in order (issue 498).
#[doc(hidden)]
pub fn bundle_args_at<B: Ports>(
    side: &str,
    path: &str,
) -> Vec<(String, String)> {
    let ports = B::ports();
    let under = format!("{path}_");
    let hits: Vec<String> = if ports.iter().any(|p| p.name == path) {
        vec![path.to_string()]
    } else {
        ports
            .into_iter()
            .map(|p| p.name)
            .filter(|n| n.starts_with(&under))
            .collect()
    };
    assert!(
        !hits.is_empty(),
        "`{side}.{path}` is not a port of the side"
    );
    hits.into_iter()
        .map(|n| (String::new(), format!("{side}_{n}")))
        .collect()
}

/// The nets of a bundle `B` made whole in a unit of units, `let (host,
/// per) = link::<B>()`: one per port of `B`, each named `net_field`, a
/// channel's as a channel and a wire's as a wire (issue 498).
#[doc(hidden)]
pub fn link_nets<B: Ports>(
    net: &str,
) -> Vec<(String, Kind, usize, &'static str)> {
    B::ports()
        .into_iter()
        .map(|p| {
            let kind = match p.kind {
                Kind::Tx | Kind::Rx => Kind::Tx,
                _ => Kind::Out,
            };
            (format!("{net}_{}", p.name), kind, p.width, p.clock)
        })
        .collect()
}

/// What a unit of units passes a child when it passes one side of a
/// bundle made whole: each net in the order `B` declares its ports.
#[doc(hidden)]
pub fn link_args<B: Ports>(net: &str) -> Vec<(String, String)> {
    B::ports()
        .into_iter()
        .map(|p| (String::new(), format!("{net}_{}", p.name)))
        .collect()
}

/// [`instance`] with its joins made when `lowered` runs rather than
/// written out, which a side passed whole needs.
#[doc(hidden)]
pub fn instance_of(
    unit: Lowered,
    name: &str,
    args: Vec<(String, String)>,
) -> Instance {
    let args: Vec<(&str, &str)> =
        args.iter().map(|(p, a)| (p.as_str(), a.as_str())).collect();
    instance(unit, name, &args)
}

/// A field of the value on the port `port` of `B`, as [`field`] finds
/// one on a port whose type the lowering could read.
#[doc(hidden)]
pub fn bundle_field<B: Ports>(e: Expr, port: &str, name: &str) -> Expr {
    let Some(p) = B::ports().into_iter().find(|p| p.name == port) else {
        panic!("`{port}` is not a port of the struct of ports")
    };
    let mut hi = p.width;
    for (n, w) in p.layout {
        hi -= w;
        if n == name {
            return if w == 1 {
                Expr::index(e, Expr::Num(hi as u128))
            } else {
                Expr::slice(e, hi, w)
            };
        }
    }
    panic!("`{name}` is not a field of the value on `{port}`")
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
    /// Rust's `as` to an unsigned integer of the stated width: the
    /// value itself when it is no wider, its low bits when it is
    /// (issue 496). Built by [`Expr::cast`].
    Cast(Box<Expr>, usize),
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
                "*" => Some(x.wrapping_mul(y)),
                "%" if y != 0 => Some(x % y),
                _ => None,
            };
            if let Some(v) = v {
                return Expr::Num(v);
            }
        }
        Expr::Bin(op, Box::new(a), Box::new(b))
    }
    /// A bit of a value. A bit of a part of a value is a bit of the
    /// value at an offset: Verilog has no bit-select of a part-select,
    /// and the VHDL written for one did not simulate as the run did,
    /// which is issue 129.
    ///
    /// A bit of a literal at a known place is that bit, as a literal.
    /// A lowered function applied to a literal reads bits of it, and
    /// VHDL does not index a qualified expression: `crc7_step` on a
    /// zero wrote `unsigned'("0000000")(6)`, which nvc refused and
    /// Verilator took (issue 555). Folded here, neither emitter sees
    /// one.
    pub fn index(a: Expr, i: Expr) -> Expr {
        if let (Expr::Bits(w, b), Expr::Num(k)) = (&a, &i) {
            if (*k as usize) < *w {
                let at = *w - 1 - *k as usize;
                return Expr::Bits(1, b[at..at + 1].to_string());
            }
        }
        match (a, i) {
            (Expr::Slice(inner, lo, _), Expr::Num(k)) => {
                Expr::Index(inner, Box::new(Expr::Num(lo as u128 + k)))
            }
            (Expr::Slice(inner, lo, _), i) => Expr::Index(
                inner,
                Box::new(Expr::bin("+", i, Expr::Num(lo as u128))),
            ),
            (a, i) => Expr::Index(Box::new(a), Box::new(i)),
        }
    }
    /// A part of a value. A part of a part is one part of the value,
    /// for the same reason a bit of a part is one bit of it.
    /// A part of a literal is those bits, as a literal, for the reason
    /// a bit of one is (issue 555).
    pub fn slice(a: Expr, lo: usize, len: usize) -> Expr {
        if let Expr::Bits(w, b) = &a {
            if len > 0 && lo + len <= *w {
                let hi = *w - lo;
                return Expr::Bits(len, b[hi - len..hi].to_string());
            }
        }
        match a {
            Expr::Slice(inner, ilo, _) => Expr::Slice(inner, ilo + lo, len),
            a => Expr::Slice(Box::new(a), lo, len),
        }
    }
    /// `a as uN`: a number keeps its low `n` bits, and anything else
    /// is a cast, which the emitters render once its width is known
    /// (issue 496).
    pub fn cast(a: Expr, n: usize) -> Expr {
        match a {
            Expr::Num(k) if n < 128 => Expr::Num(k & ((1u128 << n) - 1)),
            Expr::Num(k) => Expr::Num(k),
            a => Expr::Cast(Box::new(a), n),
        }
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
#[derive(Clone)]
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
    /// `check!`, `assume!` or `cover!`: a condition stated at an edge,
    /// under the conditions the statement is under, with its message
    /// (issue 502).
    Check(Checked, Expr, String),
}

/// What a [`Stmt::Check`] states of its condition. See
/// [`formal`](crate::formal).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checked {
    /// It holds: `check!`.
    Assert,
    /// The unit takes it for granted of its inputs: `assume!`.
    Assume,
    /// It can happen: `cover!`.
    Cover,
}

/// One process of a unit: one loop of `run`, on the rising or the
/// falling edge of its clock. A loop of one wait is the statements
/// after it; a loop of several is a state machine with a state per
/// wait. A clocked block in the netlist, if it drives anything.
#[derive(Clone)]
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
#[derive(Clone)]
pub struct Lowered {
    /// What the module or entity is called.
    pub name: String,
    /// The unit's fields, as [`Fields::fields`] gives them: the
    /// registers and memories the netlist declares.
    pub fields: Vec<(&'static str, Option<Kind>, usize, usize)>,
    /// The ports, from `run`'s signature: name, what it is, width,
    /// and the clock its domain is on, which is the second type
    /// argument of `Out`, `In`, `Tx`, `Rx` or `Pad` and is the only
    /// place a domain is still written down (issue 131).
    pub ports: Vec<(String, Kind, usize, &'static str)>,
    /// The `let` names of the loops that are computed, each a wire
    /// driven by its expression; a read of a port or register is an
    /// alias and not here.
    pub wires: Vec<(String, Expr)>,
    /// Every computed `let` as it is written and as the netlist names
    /// it. The two differ where the `let`'s name is a port's, another
    /// wire's or a field's, since the netlist has one namespace for
    /// all of them; the emitters say so in a comment, so that a reader
    /// can map a wire back to the `let` it came from (issue 171).
    pub wire_names: Vec<(String, String)>,
    /// One per loop of `run`: a clocked block in the netlist, except
    /// for one that drives nothing, which gets no block (as
    /// `chanpair_last` in `//docs:chanwait.v`).
    pub procs: Vec<Process>,
    /// A memory's first words, as `Mem::with` gave them: a program.
    pub init: Vec<(String, Vec<u128>)>,
    /// A register's value before the first edge, as `Reg::new` gave it.
    ///
    /// A register not named here starts at zero, which is what
    /// `Reg::default` does. The generated `lowered` fills this from
    /// the unit's `Default`, its own registers read before any edge
    /// ([`starts`], issue 890), and [`Lowered::init_reg`] adds to it
    /// for a top the run built some other way (issue 359).
    pub init_regs: Vec<(String, u128)>,
    /// The registers the netlist marks `ASYNC_REG`, as
    /// [`Fields::ASYNC_REGS`] names them: a synchroniser's stages.
    pub async_regs: Vec<&'static str>,
    /// The memories [`Fields::DISTRIBUTED`] names: meant to be LUT RAM,
    /// so not held to a block RAM's ports (issue 1285).
    pub distributed: Vec<&'static str>,
    /// The memories [`Fields::RAM_STYLES`] names, with what each asks
    /// Vivado to make it (issue 1371).
    pub ram_styles: Vec<(&'static str, &'static str)>,
    /// The registers [`Fields::USE_DSP`] names, with what each says
    /// (issue 1383).
    pub use_dsp: Vec<(&'static str, &'static str)>,
    /// A port's trace scope when it is not the port's own name: a
    /// channel two units share under one name in the run has a port
    /// name of its own on each side.
    pub aliases: Vec<(String, String)>,
    /// The channels and wires a unit of units made in its `run` to
    /// join its children: name, `Tx` for a channel or `Out` for a
    /// wire, the payload's width and the clock. A channel is a buffer
    /// of two between its ends, as the runtime's is, an instance of
    /// the netlist's channel module, `<top>_txhdl_chan`, with the
    /// sender's three nets on one side and the receiver's on the other;
    /// a wire is one net.
    pub nets: Vec<(String, Kind, usize, &'static str)>,
    /// The channels among `nets` that are unregistered (issue 1293):
    /// each an instance of the netlist's second channel module,
    /// `<top>_txhdl_chan_u`, whose receiver sees the sender's `valid`
    /// and `data` in the same cycle while its buffer is empty.
    pub unregistered: Vec<String>,
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
#[derive(Clone)]
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
            // A foreign module's ports are on the clock its first
            // pin is driven by, since the module is written outside
            // and says nothing about domains itself.
            .map(|(p, k, w)| {
                (
                    p.to_string(),
                    *k,
                    *w,
                    clocks.first().map_or("clk", |(_, c)| *c),
                )
            })
            .collect(),
        wires: Vec::new(),
        wire_names: Vec::new(),
        procs: Vec::new(),
        init: Vec::new(),
        init_regs: Vec::new(),
        async_regs: Vec::new(),
        distributed: Vec::new(),
        ram_styles: Vec::new(),
        use_dsp: Vec::new(),
        aliases: Vec::new(),
        nets: Vec::new(),
        unregistered: Vec::new(),
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
#[derive(Clone)]
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
    for ((p, _, _, _), (by, a)) in unit.ports.iter().zip(args) {
        let port = if by.is_empty() {
            p.clone()
        } else if unit.ports.iter().any(|(n, _, _, _)| n == by) {
            by.to_string()
        } else {
            // A field of a struct implementing `Ports`, which the child
            // names `side_field`; the parent sees the field and not the
            // child's side, so the field finds the one port so named.
            let tail = format!("_{by}");
            let hits: Vec<&String> = unit
                .ports
                .iter()
                .map(|(n, _, _, _)| n)
                .filter(|n| n.ends_with(&tail))
                .collect();
            assert!(!hits.is_empty(), "`{name}` has no port `{by}`");
            assert!(
                hits.len() == 1,
                "`{name}` has more than one port for the field `{by}`: {}",
                hits.iter()
                    .map(|h| h.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            hits[0].clone()
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
    /// This unit with every reference to a renamed field written as
    /// the netlist names it. The declarations already are: they come
    /// from [`Fields::fields`], which the derive writes with the
    /// netlist's names. What the lowering wrote are the references,
    /// under the names Rust knows, since `#[lower]` reads the `impl`
    /// and never sees the struct. This is issue 222.
    pub fn renamed(mut self, pairs: &[(&'static str, &'static str)]) -> Self {
        if pairs.is_empty() {
            return self;
        }
        let of = |n: &str| -> Option<String> {
            pairs
                .iter()
                .find(|(rust, _)| *rust == n)
                .map(|(_, net)| (*net).to_string())
        };
        self.rewrite_names(&of);
        self
    }

    /// Every name the wires, the processes and the first values refer
    /// to, rewritten through `of` where it gives one.
    fn rewrite_names(&mut self, of: &dyn Fn(&str) -> Option<String>) {
        fn walk_expr(e: &mut Expr, of: &dyn Fn(&str) -> Option<String>) {
            match e {
                Expr::Name(n) => {
                    if let Some(net) = of(n) {
                        *n = net;
                    }
                }
                Expr::Num(_) | Expr::Bits(..) => {}
                Expr::Bin(_, a, b) => {
                    walk_expr(a, of);
                    walk_expr(b, of);
                }
                Expr::Not(a)
                | Expr::Sext(a, _)
                | Expr::Zext(a, _)
                | Expr::Cast(a, _) => walk_expr(a, of),
                Expr::Slice(a, _, _) => walk_expr(a, of),
                Expr::Cat(a, b) | Expr::Index(a, b) => {
                    walk_expr(a, of);
                    walk_expr(b, of);
                }
                Expr::Cond(c, a, b) => {
                    walk_expr(c, of);
                    walk_expr(a, of);
                    walk_expr(b, of);
                }
            }
        }
        fn walk_target(t: &mut Target, of: &dyn Fn(&str) -> Option<String>) {
            match t {
                Target::Name(n) => {
                    if let Some(net) = of(n) {
                        *n = net;
                    }
                }
                Target::Word(n, a) => {
                    if let Some(net) = of(n) {
                        *n = net;
                    }
                    walk_expr(a, of);
                }
            }
        }
        fn walk_drives(
            ds: &mut Vec<(Target, Expr)>,
            of: &dyn Fn(&str) -> Option<String>,
        ) {
            for (t, e) in ds {
                walk_target(t, of);
                walk_expr(e, of);
            }
        }
        fn walk_stmt(st: &mut Stmt, of: &dyn Fn(&str) -> Option<String>) {
            match st {
                Stmt::Drive(t, e) => {
                    walk_target(t, of);
                    walk_expr(e, of);
                }
                Stmt::When(c, a, b) => {
                    walk_expr(c, of);
                    walk_drives(a, of);
                    walk_drives(b, of);
                }
                Stmt::Case(arms) => {
                    for (c, ds) in arms {
                        walk_expr(c, of);
                        walk_drives(ds, of);
                    }
                }
                Stmt::If(arms, rest) => {
                    for (c, ss) in arms {
                        walk_expr(c, of);
                        for s in ss {
                            walk_stmt(s, of);
                        }
                    }
                    for s in rest {
                        walk_stmt(s, of);
                    }
                }
                Stmt::Guard(c) | Stmt::Check(_, c, _) => walk_expr(c, of),
            }
        }
        for (_, e) in &mut self.wires {
            walk_expr(e, of);
        }
        for p in &mut self.procs {
            for s in &mut p.body {
                walk_stmt(s, of);
            }
        }
        // A memory's words are given under the memory's own name,
        // and a register's first value under the register's.
        for (n, _) in &mut self.init {
            if let Some(net) = of(n) {
                *n = net;
            }
        }
        for (n, _) in &mut self.init_regs {
            if let Some(net) = of(n) {
                *n = net;
            }
        }
    }

    /// This unit as its netlists name it: every name VHDL or
    /// SystemVerilog reserves takes [`crate::reserved::ESCAPE`] after
    /// it, the same in both targets, rather than being refused (issue
    /// 497). A port called `next` is `next_rw` in both netlists.
    ///
    /// What is escaped is what the netlist declares under a plain
    /// name: the module, its ports that are one wire, its wires and
    /// nets of one wire, and its instances. A field comes escaped
    /// already, since `#[derive(Trace)]` gives the netlist and the
    /// trace the same name for it. A channel's nets are its name and
    /// `_data`, `_valid` or `_ready`, which nothing reserves, so a
    /// channel is left alone, and so is a foreign module, whose names
    /// are its vendor's.
    ///
    /// A port escaped is still recorded under its own name in the
    /// run's trace, so the ports file names that as its trace scope,
    /// and the testbench reads it from there. A port-name mismatch
    /// between the netlist and the trace fails silently, which issue
    /// 462 showed, so this is the one place both are decided.
    ///
    /// An escaped name another name already takes is refused, naming
    /// both: that is the one case left where a reserved word cannot be
    /// lowered as it is written.
    pub fn escaped(&self) -> Lowered {
        use crate::reserved::escaped as esc;
        let mut l = self.clone();
        if l.foreign.is_some() {
            return l;
        }
        let plain = |k: &Kind| matches!(k, Kind::In | Kind::Out | Kind::Pad);
        let mut moved: Vec<(String, String)> = Vec::new();
        let mut aliases = std::mem::take(&mut l.aliases);
        for (p, k, _, _) in &mut l.ports {
            let e = esc(p);
            if plain(k) && e != *p {
                match aliases.iter_mut().find(|(a, _)| a == p) {
                    Some(a) => a.0 = e.clone(),
                    None => aliases.push((e.clone(), p.clone())),
                }
                moved.push((p.clone(), e.clone()));
                *p = e;
            }
        }
        l.aliases = aliases;
        for (n, k, _, _) in &mut l.nets {
            let e = esc(n);
            if !matches!(k, Kind::Tx | Kind::Rx) && e != *n {
                moved.push((n.clone(), e.clone()));
                *n = e;
            }
        }
        for (n, _) in &mut l.wires {
            *n = esc(n);
        }
        for (_, n) in &mut l.wire_names {
            *n = esc(n);
        }
        let of = |n: &str| -> Option<String> {
            let e = esc(n);
            (e != n).then_some(e)
        };
        l.rewrite_names(&of);
        let channels: Vec<String> = l
            .ports
            .iter()
            .map(|(n, k, _, _)| (n, k))
            .chain(l.nets.iter().map(|(n, k, _, _)| (n, k)))
            .filter(|(_, k)| matches!(k, Kind::Tx | Kind::Rx))
            .map(|(n, _)| n.clone())
            .collect();
        for inst in &mut l.instances {
            let e = esc(&inst.name);
            if e != inst.name {
                moved.push((inst.name.clone(), e.clone()));
                inst.name = e;
            }
            let child = &inst.unit;
            for (port, arg) in &mut inst.conns {
                let child_plain = child.foreign.is_none()
                    && child
                        .ports
                        .iter()
                        .any(|(n, k, _, _)| n == port && plain(k));
                if child_plain {
                    *port = esc(port);
                }
                if !channels.contains(arg) {
                    *arg = esc(arg);
                }
            }
            inst.unit = inst.unit.escaped();
        }
        l.name = esc(&l.name);
        // What the module declares under its own names, once escaped:
        // an escaped name that is also one of these was taken twice.
        let mut declared: Vec<String> = Vec::new();
        for (n, k, _, _) in l.ports.iter().chain(l.nets.iter()) {
            match k {
                Kind::Tx | Kind::Rx => {
                    for end in ["data", "valid", "ready"] {
                        declared.push(format!("{n}_{end}"));
                    }
                }
                _ => declared.push(n.clone()),
            }
        }
        // A field that holds a child is the instance, counted below, and
        // not a second declaration of the name.
        declared.extend(
            l.fields
                .iter()
                .filter(|(_, k, _, _)| k.is_some())
                .map(|(n, _, _, _)| n.to_string()),
        );
        declared.extend(l.wires.iter().map(|(n, _)| n.clone()));
        declared.extend(l.instances.iter().map(|i| i.name.clone()));
        for (was, e) in &moved {
            let times = declared.iter().filter(|d| *d == e).count();
            assert!(
                times == 1,
                "`{was}` is a reserved word, and the name the netlist \
                 would give it, `{e}`, is already taken in `{}`: rename \
                 one of the two (issue 497)",
                self.name
            );
        }
        l
    }

    /// This unit with the wires its helpers asked for, which
    /// [`inlined_end`] gives (issue 504).
    #[doc(hidden)]
    pub fn with_inlined(mut self, wires: Vec<(String, Expr)>) -> Self {
        self.wires.extend(wires);
        self
    }

    /// This unit, once nothing it declares takes a clock's name.
    ///
    /// A clock reaches a module as a port named for the clock, beside
    /// everything else the module declares, so a child, a register, a
    /// port, a wire or a net of that name is declared twice. Verilog
    /// gives an instance and a port one namespace and Verilator
    /// refuses the module; VHDL keeps a label apart from a port and
    /// nvc accepts it, so a design could pass one co-simulation and
    /// fail the other, or fail only in synthesis. Refused here, at
    /// lowering, naming the thing and the clock. The thing is the one
    /// to rename: a clock's name is its type's, and every unit on that
    /// clock shares it. A child's clock counts, since the parent takes
    /// it as a port to pass on. This is issue 367.
    pub fn checked(self) -> Self {
        // An unregistered channel may not close a loop of wires
        // through the children (issue 1293).
        self.refuse_comb_loops();
        // A memory a block RAM cannot hold is refused here, not found
        // in a synthesis an hour later (issue 1285).
        self.refuse_unblockable_mems();
        // `ASYNC_REG` belongs on a register; on a wire or a memory it
        // would say nothing true (#884).
        for a in &self.async_regs {
            assert!(
                self.fields
                    .iter()
                    .any(|(n, k, _, _)| n == a && *k == Some(Kind::Reg)),
                "field `{a}` of `{}` carries #[async_reg] and is not a \
                 register; only a register can be a synchroniser's stage",
                self.name
            );
        }
        // A unit of units joins each channel port of its own to one
        // child, once. `#[lower]` checks that for a port it can see; a
        // port that is a field of a struct of ports declared elsewhere
        // has a kind the macro cannot see, so it is checked here, where
        // every port's kind is known (issue 483).
        if !self.instances.is_empty() && self.procs.is_empty() {
            for (p, k, _, _) in &self.ports {
                if !matches!(k, Kind::Tx | Kind::Rx) {
                    continue;
                }
                let n = self
                    .instances
                    .iter()
                    .flat_map(|i| &i.conns)
                    .filter(|(_, a)| a == p)
                    .count();
                assert!(
                    n == 1,
                    "port `{p}` of `{}` is a channel joined to {n} children; \
                     a channel has one sender and one receiver, so join it \
                     to exactly one",
                    self.name
                );
            }
        }
        // A channel between children has one sender and one receiver,
        // so its net is joined to exactly two ports. `#[lower]` counts
        // the ends it can see; a bundle made whole, `link::<B>()`, has
        // channels only `lowered` can list, so all are counted here
        // (issue 498).
        for (n, k, _, _) in &self.nets {
            if *k != Kind::Tx {
                continue;
            }
            let joins = self
                .instances
                .iter()
                .flat_map(|i| &i.conns)
                .filter(|(_, a)| a == n)
                .count();
            assert!(
                joins == 2,
                "the channel `{n}` of `{}` is joined to {joins} ports; a \
                 channel joins one sender to one receiver, so join each \
                 side of it once",
                self.name
            );
        }
        // A wire of the parent's own, which a tie-off is, drives what it
        // is joined to, so it may only be joined to a child's input: an
        // output joined to it would be driven twice, and a channel's
        // three nets are not one wire (issue 498).
        for i in &self.instances {
            for (port, a) in &i.conns {
                if !self.wires.iter().any(|(w, _)| w == a) {
                    continue;
                }
                let kind = i
                    .unit
                    .ports
                    .iter()
                    .find(|(p, _, _, _)| p == port)
                    .map(|(_, k, _, _)| *k);
                assert!(
                    matches!(kind, Some(Kind::In)),
                    "`{a}` of `{}` holds a constant, and is joined to the \
                     port `{port}` of `{}`, which is not an input: a \
                     constant can only be passed where an input is taken",
                    self.name,
                    i.name
                );
            }
        }
        for c in self.clocks() {
            let what = if self.instances.iter().any(|i| i.name == c) {
                Some(("field", "holds a child, and the instance"))
            } else if self.fields.iter().any(|(f, _, _, _)| *f == c) {
                Some(("field", "is a register or a memory, and it"))
            } else if self.ports.iter().any(|(p, _, _, _)| p == c) {
                Some(("port", "is the unit's, and it"))
            } else if self.wires.iter().any(|(w, _)| w == c) {
                Some(("let", "is computed, and its wire"))
            } else if self.nets.iter().any(|(n, _, _, _)| n == c) {
                Some(("channel or wire", "joins the children, and its net"))
            } else {
                None
            };
            if let Some((kind, does)) = what {
                panic!(
                    "{kind} `{c}` of `{}` {does} would take the name of the \
                     clock `{c}`, which the netlist declares as a port of \
                     the module: Verilator refuses the two under one name. \
                     Rename the {kind}; the clock's name is its type's, \
                     shared by every unit on it (see issue 367)",
                    self.name
                );
            }
        }
        self
    }

    /// Whether the unit's own `run` takes the reset as a port.
    ///
    /// A unit that reads the reset declares `rst: In<Bit>` and uses
    /// it, as the core and the timer do. The netlist then joins that
    /// port to the reset net rather than adding a second port of the
    /// same name.
    /// A port of that name that is not an input is refused, rather
    /// than quietly shadowing the reset or colliding with it in the
    /// netlist. A unit that drives a reset elsewhere is driving a
    /// request for one, and names it accordingly.
    fn declares_reset(&self) -> bool {
        let r = crate::comp::RESET_NAME;
        for (n, k, _, _) in &self.ports {
            if n == r && *k != Kind::In {
                panic!(
                    "unit `{}` has a port `{r}` that is not an In: the \
                     netlist keeps that name for the reset, which \
                     reaches every module. Name it for what it is, such \
                     as `{r}_req` for a reset a unit asks for.",
                    self.name
                );
            }
        }
        self.ports
            .iter()
            .any(|(n, k, _, _)| n == r && *k == Kind::In)
    }

    /// Whether the module has a reset at all.
    ///
    /// Anything clocked has one: a register to put back, a channel to
    /// empty, or a child with either. A module with nothing clocked
    /// has nothing a reset would do, and takes no port for it.
    fn has_reset(&self) -> bool {
        self.foreign.is_none() && !self.clocks().is_empty()
    }

    /// Whether the netlist adds the reset port, rather than the unit
    /// having declared it.
    fn adds_reset_port(&self) -> bool {
        self.has_reset() && !self.declares_reset()
    }

    /// The registers a process drives, which are the ones its clocked
    /// block puts back on reset.
    ///
    /// Per process rather than per module, because two clocked blocks
    /// that assigned one register would be two drivers of it. A
    /// memory is not among them: it holds what was loaded into it,
    /// and a reset is not a reload.
    fn reset_regs(&self, p: &Process) -> Vec<String> {
        fn walk(out: &mut Vec<String>, st: &Stmt) {
            let mut name = |t: &Target| {
                if let Target::Name(n) = t {
                    if !out.contains(n) {
                        out.push(n.clone())
                    }
                }
            };
            match st {
                Stmt::Drive(t, _) => name(t),
                Stmt::When(_, a, b) => {
                    for (t, _) in a.iter().chain(b) {
                        name(t)
                    }
                }
                Stmt::Case(arms) => {
                    for (_, ds) in arms {
                        for (t, _) in ds {
                            name(t)
                        }
                    }
                }
                Stmt::If(arms, els) => {
                    for (_, ss) in arms {
                        for s in ss {
                            walk(out, s)
                        }
                    }
                    for s in els {
                        walk(out, s)
                    }
                }
                Stmt::Guard(_) | Stmt::Check(..) => {}
            }
        }
        let mut out: Vec<String> = Vec::new();
        for st in &p.body {
            walk(&mut out, st);
        }
        out.retain(|n| {
            self.fields
                .iter()
                .any(|(f, k, _, _)| f == n && *k == Some(Kind::Reg))
        });
        out
    }

    /// Whether a process writes a memory. Its clocked block then has a
    /// reset branch even when it drives no register, so that a write
    /// under reset is dropped whether or not a register shares the
    /// process, as the runtime drops it (issue 877).
    fn writes_mem(&self, p: &Process) -> bool {
        fn word(t: &Target) -> bool {
            matches!(t, Target::Word(..))
        }
        fn walk(st: &Stmt) -> bool {
            match st {
                Stmt::Drive(t, _) => word(t),
                Stmt::When(_, a, b) => a.iter().chain(b).any(|(t, _)| word(t)),
                Stmt::Case(arms) => {
                    arms.iter().any(|(_, ds)| ds.iter().any(|(t, _)| word(t)))
                }
                Stmt::If(arms, els) => {
                    arms.iter().any(|(_, ss)| ss.iter().any(walk))
                        || els.iter().any(walk)
                }
                Stmt::Guard(_) | Stmt::Check(..) => false,
            }
        }
        p.body.iter().any(walk)
    }

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
            let (_, k, _, _) = inst
                .unit
                .ports
                .iter()
                .find(|(n, _, _, _)| n == p)
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
    /// Whether this netlist, or a child of it, has an unregistered
    /// channel, and so the second channel module (issue 1293).
    fn has_unreg_nets(&self) -> bool {
        !self.unregistered.is_empty()
            || self.instances.iter().any(|i| i.unit.has_unreg_nets())
    }
    /// The module a channel net is an instance of: the unregistered one
    /// for a net among `unregistered`, else the registered one.
    fn chan_module(&self, chan: &str, net: &str) -> String {
        if self.unregistered.iter().any(|u| u == net) {
            format!("{chan}_u")
        } else {
            chan.to_string()
        }
    }
    /// Every module or entity name this netlist defines or
    /// instantiates, its own and its children's, foreign ones included.
    fn unit_names(&self, out: &mut Vec<String>) {
        out.push(self.name.clone());
        for i in &self.instances {
            i.unit.unit_names(out);
        }
    }
    /// The channel module's name in this netlist: the top's name and
    /// `_txhdl_chan`, so that two netlists in one design, or in one
    /// file, never define the same module twice (issue 979). VHDL reads
    /// names without case, so a unit of the netlist whose name is this
    /// one in any case is refused, naming both.
    fn chan_name(&self) -> String {
        let chan = format!("{}_txhdl_chan", self.name);
        let unreg = format!("{chan}_u");
        let mut names = Vec::new();
        self.unit_names(&mut names);
        if let Some(n) = names.iter().find(|n| {
            n.eq_ignore_ascii_case(&chan) || n.eq_ignore_ascii_case(&unreg)
        }) {
            panic!(
                "`{}` names its channel module `{chan}`, which the unit \
                 `{n}` already is; rename the field that holds `{n}`",
                self.name
            );
        }
        chan
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
                Stmt::Drive(_, _) | Stmt::Guard(_) | Stmt::Check(..) => false,
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
    ///
    /// The name must be one of the unit's own memories. Anything else,
    /// a typo or a memory that has moved into a child, is refused here,
    /// naming it and the unit, rather than giving a netlist whose memory
    /// is all zeros without a word (issue 1104): a board's boot memory
    /// came close to shipping empty that way.
    pub fn init(&mut self, mem: &str, words: &[u128]) {
        self.own(mem, Kind::Mem, "memory");
        self.init.push((mem.to_string(), words.to_vec()));
    }
    /// Give a register its value before the first edge, as `Reg::new`
    /// gave it at run time, where the run's instance is not the one
    /// `Default` builds.
    ///
    /// The generated `lowered` already gives every register of the
    /// unit the start its `Default` gives it, at any depth (issue 890),
    /// so this is needed only for a top the run built some other way,
    /// with `Reg::new` and a value `Default` does not use; it names
    /// the unit's own registers, not a child's, and wins over the start
    /// `Default` gave. Without it the netlist and the run disagree from
    /// the first cycle (issue 359). The value is also what a reset puts
    /// back, in both emitters as in the runtime (issue 728).
    ///
    /// The name must be one of the unit's own registers, and a name
    /// that is not is refused, as for [`Lowered::init`] (issue 1104).
    pub fn init_reg(&mut self, reg: &str, value: u128) {
        self.own(reg, Kind::Reg, "register");
        self.init_regs.push((reg.to_string(), value));
    }
    /// Refuse a name that is not one of the unit's own fields of `kind`,
    /// saying which the unit does have.
    fn own(&self, name: &str, kind: Kind, what: &str) {
        let have: Vec<&str> = self
            .fields
            .iter()
            .filter(|(_, k, _, _)| *k == Some(kind))
            .map(|(n, _, _, _)| *n)
            .collect();
        assert!(
            have.contains(&name),
            "{} has no {what} `{name}` to give a first value: its own are \
             {have:?}, and a child's are given through the child's \
             instance",
            self.name
        );
    }
}

/// The registers of a unit that start at something other than zero,
/// and what they start at, as the run has them: the unit is built with
/// `Default` and each of its own registers read before any edge, which
/// is the value `Reg::new` was given and the one a reset puts back.
/// The generated `lowered` fills `init_regs` from this, so a unit at
/// any depth starts as the run starts it, with no `init_reg` (issue
/// 890). A child's registers are the child's own `lowered`'s business,
/// so only the unit's own fields are read: a path one step below the
/// unit. A register wider than 128 bits is left at zero, as `init_reg`
/// cannot hold it either.
#[doc(hidden)]
pub fn starts<T: Default + Traceable>() -> Vec<(String, u128)> {
    collect("unit", &T::default())
        .into_iter()
        .filter(|p| p.kind == Kind::Reg)
        .filter_map(|p| {
            let name = p.path.strip_prefix("unit.")?;
            if name.contains('.') {
                return None;
            }
            let v = u128::from_str_radix(&(p.sample)(), 2).ok()?;
            (v != 0).then(|| (name.to_string(), v))
        })
        .collect()
}

impl Lowered {
    /// What a register starts at: what `init_reg` was told last, or
    /// zero. The generated `lowered` puts the unit's own starts first
    /// (issue 890), so an `init_reg` a caller writes after it wins.
    fn reg_init(&self, reg: &str) -> u128 {
        match self.init_regs.iter().rev().find(|(n, _)| n == reg) {
            Some((_, v)) => *v,
            None => 0,
        }
    }
    /// A register's width, or one for a name that is not a field.
    fn reg_width(&self, reg: &str) -> usize {
        self.fields
            .iter()
            .find(|(f, _, _, _)| *f == reg)
            .map(|(_, _, w, _)| *w)
            .unwrap_or(1)
    }
    /// What a register starts at as the VHDL writes it, in its
    /// declaration and in its clocked block's reset, which puts back
    /// the same value (issue 728).
    fn reg_init_vhdl(&self, reg: &str) -> String {
        match (self.reg_init(reg), self.reg_width(reg)) {
            (0, 1) => "'0'".to_string(),
            (0, _) => "(others => '0')".to_string(),
            (v, 1) => format!("'{v}'"),
            (v, w) => format!("\"{v:0w$b}\""),
        }
    }
    /// Say under which trace scope a port is found, when the run named
    /// the wire or channel otherwise than the port.
    ///
    /// `port` is the port's own name, as `run` writes it, even where
    /// the netlist escapes it (issue 497).
    pub fn trace_as(&mut self, port: &str, scope: &str) {
        self.aliases.retain(|(p, _)| p != port);
        self.aliases.push((port.to_string(), scope.to_string()));
    }
    fn scope_col(&self, port: &str) -> String {
        match self.aliases.iter().find(|(p, _)| p == port) {
            Some((_, s)) => format!(" {s}"),
            None => String::new(),
        }
    }
    /// Which clock a port is on, for the ports file: `@` and the
    /// clock's name, or nothing for a port on the default clock,
    /// since that is what a reader assumes and every unit of one
    /// clock would otherwise carry the same column (issue 131).
    fn clock_col(c: &str) -> String {
        if c == DefaultClock::NAME {
            String::new()
        } else {
            format!(" @{c}")
        }
    }
    /// Verilog cannot part-select an expression, so every slice of
    /// one is hoisted into a wire of its own, `slN`, declared before
    /// the body that uses it.
    /// A bit of one is hoisted the same way, since VHDL cannot index one
    /// (issue 683), and each wire's width is kept in `HOISTED` while
    /// the pass and the emitter after it run, so a bit of it is read as
    /// a bit of that many.
    #[allow(clippy::type_complexity)]
    fn hoisted(
        &self,
    ) -> (
        Vec<Process>,
        Vec<(String, Expr)>,
        Vec<(String, usize, Expr)>,
    ) {
        HOISTED.with(|h| h.borrow_mut().clear());
        DSP_WIRES.with(|h| h.borrow_mut().clear());
        let mut temps: Vec<(String, usize, Expr)> = Vec::new();
        // A computed value as a wire of its own, named `sl<n>`; the
        // same expression twice is one wire.
        fn wire(
            a: Expr,
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Expr {
            let same = format!("{a:?}");
            let found = t.iter().find(|(_, _, e)| format!("{e:?}") == same);
            let name = match found {
                Some((n, _, _)) => n.clone(),
                None => {
                    let n = format!("sl{}", t.len());
                    let w = l.ewidth(&a);
                    HOISTED.with(|h| h.borrow_mut().push((n.clone(), w)));
                    t.push((n.clone(), w, a));
                    n
                }
            };
            Expr::Name(name)
        }
        // Whether Verilog can select a value's top bit where it stands,
        // as a sign extension does: a name, a memory's word, a slice of
        // one, a concatenation whose high part is, or a single bit,
        // which is its own top bit.
        fn top_in_place(e: &Expr, l: &Lowered) -> bool {
            match e {
                Expr::Name(_) | Expr::Slice(_, _, _) => true,
                Expr::Index(m, _) => {
                    matches!(&**m, Expr::Name(n) if l.is_mem(n))
                }
                Expr::Cat(a, _) => top_in_place(a, l),
                _ => l.ewidth(e) <= 1,
            }
        }
        // Every product under the top of a register's next value as a
        // wire of its own, carrying the register's `use_dsp` (issue
        // 1343). Vivado applies the attribute only to the arithmetic
        // that drives the signal it is on, so a register's did not
        // reach the products under its add, which took the memory read
        // before them into a DSP and left the memory in LUTs. A product
        // that is the whole value is the register's own.
        fn dsp_products(
            e: Expr,
            v: &'static str,
            top: bool,
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Expr {
            let d = |x: Box<Expr>, t: &mut Vec<(String, usize, Expr)>| {
                Box::new(dsp_products(*x, v, false, l, t))
            };
            match e {
                Expr::Bin("*", a, c) if !top => {
                    let p = Expr::Bin("*", d(a, t), d(c, t));
                    let w = wire(p, l, t);
                    if let Expr::Name(n) = &w {
                        DSP_WIRES.with(|h| {
                            let mut h = h.borrow_mut();
                            if !h.iter().any(|(m, _)| m == n) {
                                h.push((n.clone(), v));
                            }
                        });
                    }
                    w
                }
                // A product named by a `let` is a wire of the unit's
                // own, which carries the attribute where it is declared.
                Expr::Name(n) => {
                    mark_named(&n, v, l);
                    Expr::Name(n)
                }
                Expr::Bin(op, a, c) => Expr::Bin(op, d(a, t), d(c, t)),
                Expr::Not(a) => Expr::Not(d(a, t)),
                Expr::Cond(c, a, b) => Expr::Cond(d(c, t), d(a, t), d(b, t)),
                Expr::Cat(a, c) => Expr::Cat(d(a, t), d(c, t)),
                Expr::Sext(a, m) => Expr::Sext(d(a, t), m),
                Expr::Zext(a, m) => Expr::Zext(d(a, t), m),
                e => e,
            }
        }
        // A named wire whose value is a product takes the attribute, and
        // so do the named wires it reads that are.
        fn mark_named(n: &str, v: &'static str, l: &Lowered) {
            let Some((_, e)) = l.wires.iter().find(|(m, _)| m == n) else {
                return;
            };
            fn names<'a>(e: &'a Expr, out: &mut Vec<&'a str>) {
                match e {
                    Expr::Name(n) => out.push(n),
                    Expr::Bin(_, a, c) | Expr::Cat(a, c) => {
                        names(a, out);
                        names(c, out);
                    }
                    Expr::Not(a)
                    | Expr::Sext(a, _)
                    | Expr::Zext(a, _)
                    | Expr::Cast(a, _)
                    | Expr::Slice(a, _, _) => names(a, out),
                    Expr::Cond(c, a, b) => {
                        names(c, out);
                        names(a, out);
                        names(b, out);
                    }
                    _ => {}
                }
            }
            if matches!(e, Expr::Bin("*", _, _)) {
                DSP_WIRES.with(|h| {
                    let mut h = h.borrow_mut();
                    if !h.iter().any(|(m, _)| m == n) {
                        h.push((n.to_string(), v));
                    }
                });
            }
            let mut read = Vec::new();
            names(e, &mut read);
            for m in read {
                mark_named(m, v, l);
            }
        }
        // A register's next value, hoisted, and its products too when
        // it says `use_dsp`.
        fn value(
            x: &Target,
            e: &Expr,
            l: &Lowered,
            t: &mut Vec<(String, usize, Expr)>,
        ) -> Expr {
            let e = go(e, l, t);
            match x {
                Target::Name(n) => match l.use_dsp_of(n) {
                    Some(v) => dsp_products(e, v, true, l, t),
                    None => e,
                },
                Target::Word(..) => e,
            }
        }
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
                    Expr::Slice(Box::new(wire(a, l, t)), *lo, *len)
                }
                Expr::Bin(op, a, c) => Expr::Bin(op, b(a, t), b(c, t)),
                Expr::Not(a) => Expr::Not(b(a, t)),
                Expr::Cond(c, a, d) => Expr::Cond(b(c, t), b(a, t), b(d, t)),
                // A bit of a computed value is a bit of a wire, as a
                // slice of one is: VHDL indexes only a name or a call,
                // and nvc refuses `unsigned'(..)(1)` (issue 683). A
                // memory's word and a one-bit value, which the
                // emitters index as they are, stay as they are.
                Expr::Index(a, i) => {
                    let a = go(a, l, t);
                    let i = b(i, t);
                    let plain = matches!(&a, Expr::Name(_))
                        || matches!(&a, Expr::Index(m, _)
                            if matches!(&**m, Expr::Name(n) if l.is_mem(n)))
                        || l.ewidth(&a) == 1
                        || l.ewidth(&a) == 0;
                    if plain {
                        return Expr::Index(Box::new(a), i);
                    }
                    Expr::Index(Box::new(wire(a, l, t)), i)
                }
                Expr::Cat(a, c) => Expr::Cat(b(a, t), b(c, t)),
                // A sign extension repeats its operand's top bit, which
                // Verilog selects of a name and not of an expression: a
                // product extended was `(x * y)[15]`, which Verilator
                // refuses (issue 1369). An operand whose top bit cannot
                // be selected where it stands is a wire of its own.
                // An extension to fewer bits than its operand has is a
                // truncation, as the runtime has it: the low bits, so a
                // slice and so perhaps a wire. The emitters wrote the
                // operand whole in Verilog, which pushed what stood
                // above it in a concatenation out of the word, and VHDL
                // resized a signed value keeping its sign bit (issue
                // 1387).
                Expr::Sext(a, m) | Expr::Zext(a, m)
                    if l.ewidth(a) > *m && *m > 0 =>
                {
                    go(&Expr::Slice(a.clone(), 0, *m), l, t)
                }
                Expr::Sext(a, m) => {
                    let a = go(a, l, t);
                    let w = l.ewidth(&a);
                    if w > 1 && w < *m && !top_in_place(&a, l) {
                        return Expr::Sext(Box::new(wire(a, l, t)), *m);
                    }
                    Expr::Sext(Box::new(a), *m)
                }
                // A cast is the value when it is no wider, and its low
                // bits, a slice and so perhaps a wire, when it is
                // (issue 496). Both emitters read it from here.
                Expr::Cast(a, n) => match l.ewidth(a) {
                    w if w == 0 || w <= *n => go(a, l, t),
                    _ => go(&Expr::Slice(a.clone(), 0, *n), l, t),
                },
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
                .map(|(x, e)| (target(x, l, t), value(x, e, l, t)))
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
                    Stmt::Drive(target(x, l, temps), value(x, e, l, temps))
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
                Stmt::Check(k, c, m) => {
                    Stmt::Check(*k, go(c, l, temps), m.clone())
                }
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

    /// What register `r` says of DSP slices: its `#[use_dsp]` (issue
    /// 1383).
    pub fn use_dsp_of(&self, r: &str) -> Option<&'static str> {
        self.use_dsp.iter().find(|(n, _)| *n == r).map(|(_, v)| *v)
    }

    /// What memory `m` asks Vivado to make it: its `#[ram_style]`, or
    /// `"distributed"` for `#[distributed]` (issue 1371).
    pub fn ram_style(&self, m: &str) -> Option<&'static str> {
        self.ram_styles
            .iter()
            .find(|(n, _)| *n == m)
            .map(|(_, st)| *st)
            .or_else(|| self.distributed.contains(&m).then_some("distributed"))
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
            Expr::Cast(a, n) => match self.ewidth(a) {
                0 => 0,
                w => w.min(*n),
            },
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
        for (p, k, w, _) in &self.ports {
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
        if let Some(w) = HOISTED
            .with(|h| h.borrow().iter().find(|(t, _)| t == n).map(|(_, w)| *w))
        {
            return w;
        }
        1
    }
    /// The ports as a sidecar for a testbench generator: one per line,
    /// `name direction width`, the clock first. A channel's three
    /// wires carry their side, `rxin`, `rxout`, `txin` or `txout`,
    /// since the trace names them by side and a channel's inputs are
    /// registered where a wire's are not.
    pub fn ports_file(&self) -> String {
        self.escaped().ports_file_in()
    }
    fn ports_file_in(&self) -> String {
        let mut out = String::new();
        // A clock is marked as one rather than as an input, so that a
        // testbench drives it as a clock and a unit of several clocks
        // can be told apart from a unit with a clock-shaped input
        // (issue 131).
        for c in self.clocks() {
            out.push_str(&format!("{c} clock 1\n"));
        }
        // The reset is an input the testbench drives, and the trace
        // holds it for every run, so it is read by its bare name like
        // any other input. A unit that declared the port itself has
        // it among `ports` already.
        if self.adds_reset_port() {
            out.push_str(&format!("{} in 1\n", crate::comp::RESET_NAME));
        }
        for (n, k, w, c) in &self.ports {
            // The clock the port is on, last on the line and marked,
            // so that a reader which does not know about it is not
            // confused by a column it was not expecting: the scope is
            // the fourth field when there is one, and a memory's depth
            // is the fourth field of its own line (issue 131).
            let s = format!("{}{}", self.scope_col(n), Self::clock_col(c));
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
        let esc = self.escaped();
        let chans = self.has_chan_nets();
        let chan = if chans {
            esc.chan_name()
        } else {
            format!("{}_txhdl_chan", esc.name)
        };
        let mut out = esc.verilog_in(&chan);
        if chans {
            out.push('\n');
            out.push_str(&CHAN_VERILOG.replace("txhdl_chan", &chan));
        }
        if esc.has_unreg_nets() {
            out.push('\n');
            out.push_str(&CHAN_VERILOG_U.replace("txhdl_chan", &chan));
        }
        out
    }
    /// The `let`s the netlist named differently, as comment lines in
    /// the target's own comment syntax. A wire takes another name when
    /// the one written is a port's, another wire's, a field's or a
    /// word the target reserves, and a reader who cannot see why a
    /// wire is called `count_w` is owed the answer (issue 171).
    fn renamed_lets(&self, lead: &str) -> String {
        let mut out = String::new();
        for (l, w) in &self.wire_names {
            if l != w {
                let _ = writeln!(out, "{lead} `let {l}` is the wire {w}.");
            }
        }
        out
    }

    /// This unit's module and its children's, without the channel
    /// module, which the outermost unit adds once, as `chan`.
    fn verilog_in(&self, chan: &str) -> String {
        // A foreign module comes as its own source.
        if self.foreign.is_some() {
            return String::new();
        }
        let name = &self.name;
        let l = self;
        let mut out = String::new();
        let mut plist: Vec<String> =
            self.clocks().iter().map(|c| format!("input {c}")).collect();
        // The reset, beside the clocks and for the same reason: it
        // reaches everything clocked, and a unit that does not read it
        // still has registers to put back. A unit that does read it
        // declared the port itself, and it is written below with the
        // rest of the ports.
        if self.adds_reset_port() {
            plist.push(format!("input {}", crate::comp::RESET_NAME));
        }
        for (n, k, w, _) in &self.ports {
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
        out.push_str(&self.renamed_lets("//"));
        writeln!(out, "module {name}(\n  {}\n);", plist.join(",\n  ")).unwrap();
        for (n, k, w, d) in &self.fields {
            match k {
                // What the runtime's register starts at: what the unit's
                // `Default` gave it, or `init_reg` for a top built
                // otherwise, and zero if neither names it.
                // A synchroniser's stage says so, as `chan_cdc.v` does by
                // hand (#884).
                Some(Kind::Reg) => writeln!(
                    out,
                    "  {}{}reg {}{n} = {w}'h{:x};",
                    if self.async_regs.contains(n) {
                        "(* ASYNC_REG = \"TRUE\" *) "
                    } else {
                        ""
                    },
                    // Whether Vivado may use DSP slices for what drives
                    // it (issue 1383).
                    match self.use_dsp_of(n) {
                        Some(v) => format!("(* use_dsp = \"{v}\" *) "),
                        None => String::new(),
                    },
                    range(*w),
                    self.reg_init(n)
                )
                .unwrap(),
                // A wire kept as a field: declared here, driven below.
                Some(Kind::Wire) => {
                    writeln!(out, "  wire {}{n};", range(*w)).unwrap()
                }
                // A memory, zero at the start as the runtime's is, then
                // its first words if the example gave them.
                Some(Kind::Mem) => {
                    // What the memory asks Vivado to make it, said where
                    // it is declared (issue 1371).
                    if let Some(st) = self.ram_style(n) {
                        writeln!(out, "  (* ram_style = \"{st}\" *)").unwrap();
                    }
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
                        // Words of zero are what the loop above wrote,
                        // so a memory given only those has no block
                        // (issue 903).
                        if m != n || words.iter().all(|v| *v == 0) {
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
                     {m} #(.W({w})) {n}_chan(\n    .clk({c}), \
                     .rst({rs}),\n    \
                     .tx_data({n}_tx_data), .tx_valid({n}_tx_valid), \
                     .tx_ready({n}_tx_ready),\n    \
                     .rx_data({n}_rx_data), .rx_valid({n}_rx_valid), \
                     .rx_ready({n}_rx_ready)\n  );",
                    r = range(*w),
                    m = self.chan_module(chan, n),
                    rs = crate::comp::RESET_NAME
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
            // The reset goes down to a lowered child by the name it
            // has here, as a clock does. A child that declared the
            // port itself is joined to it by `joins` below, with its
            // other ports, so only the added one is wired here.
            if inst.unit.adds_reset_port() {
                let r = crate::comp::RESET_NAME;
                conns.push(format!(".{r}({r})"));
            }
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
            writeln!(
                out,
                "  {}wire {}{n};",
                dsp_wire(n)
                    .map(|v| format!("(* use_dsp = \"{v}\" *) "))
                    .unwrap_or_default(),
                range(w)
            )
            .unwrap();
        }
        for (t, w, e) in &temps {
            writeln!(
                out,
                "  {}wire {}{t} = {};",
                dsp_wire(t)
                    .map(|v| format!("(* use_dsp = \"{v}\" *) "))
                    .unwrap_or_default(),
                range(*w),
                vexpr(e, l)
            )
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
            rst: Option<&str>,
        ) {
            let inner = format!("{ind}  ");
            let open = |i: usize, c: &str| {
                let kw = if i == 0 { "if" } else { "end else if" };
                format!("{ind}{kw} ({c}) begin")
            };
            let else_line = format!("{ind}end else begin");
            let end = format!("{ind}end");
            match st {
                Stmt::Guard(_) => {}
                // An immediate assertion, assumption or cover in the
                // clocked block, where only a formal tool sees it: a
                // synthesis tool reading plain Verilog does not know the
                // words (issue 502).
                Stmt::Check(k, c, m) => {
                    let kw = match k {
                        Checked::Assert => "assert",
                        Checked::Assume => "assume",
                        Checked::Cover => "cover",
                    };
                    // Out of reset only: a check of what the reset
                    // puts back, or of an output it holds, would
                    // otherwise fail in the reset cycles for no fault
                    // of the design, whichever side declared the port
                    // (issue 633).
                    let gate =
                        rst.map_or(String::new(), |r| format!("if (!{r}) "));
                    seq.push("`ifdef FORMAL".to_string());
                    seq.push(format!(
                        "{ind}{gate}{kw} ({}); // {m}",
                        vexpr(c, l)
                    ));
                    seq.push("`endif".to_string());
                }
                Stmt::Drive(t, e) => drive(seq, comb, ind, t, e),
                Stmt::When(c, then, otherwise) => {
                    let mut a = Vec::new();
                    for (t, e) in then {
                        drive(&mut a, comb, &inner, t, e);
                    }
                    let mut b = Vec::new();
                    for (t, e) in otherwise {
                        drive(&mut b, comb, &inner, t, e);
                    }
                    let arms = vec![(vexpr(c, l), a)];
                    chain(seq, arms, b, &open, &else_line, &end);
                }
                Stmt::Case(arms) => {
                    let arms = arms
                        .iter()
                        .map(|(c, drives)| {
                            let mut b = Vec::new();
                            for (t, e) in drives {
                                drive(&mut b, comb, &inner, t, e);
                            }
                            (vexpr(c, l), b)
                        })
                        .collect();
                    chain(seq, arms, Vec::new(), &open, &else_line, &end);
                }
                Stmt::If(arms, els) => {
                    let arms = arms
                        .iter()
                        .map(|(c, body)| {
                            let mut b = Vec::new();
                            for s in body {
                                stmt(&mut b, comb, &inner, s, drive, l, rst);
                            }
                            (vexpr(c, l), b)
                        })
                        .collect();
                    let mut b = Vec::new();
                    for s in els {
                        stmt(&mut b, comb, &inner, s, drive, l, rst);
                    }
                    chain(seq, arms, b, &open, &else_line, &end);
                }
            }
        }
        // The reset a statement is gated on, where the module has one,
        // whichever side declared it (issue 633).
        let rst = self.has_reset().then_some(crate::comp::RESET_NAME);
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
                stmt(&mut seq, &mut comb, ind, st, &drive, l, rst);
            }
            if guard {
                seq.push("    end".into());
            }
            if seq.iter().any(|l| l.contains("<=") || l == "`endif") {
                let edge = if p.falling { "negedge" } else { "posedge" };
                writeln!(out, "  always @({edge} {}) begin", p.clock).unwrap();
                // The reset, inside the clocked block: synchronous, so
                // that it is a value on the edge like any other and
                // needs no second sensitivity. Only the registers this
                // process drives, since a register put back by two
                // blocks would have two drivers.
                // Only where the netlist added the port. A unit that
                // declared `rst` itself answers it in its own body, in
                // whatever way it means, and the runtime replays that
                // body exactly; a clearing branch added on top would
                // happen in the netlist and not in the Rust.
                let regs = if self.adds_reset_port() {
                    self.reset_regs(p)
                } else {
                    Vec::new()
                };
                // A memory's write sits in the branch out of reset too,
                // with nothing to put back in the other (issue 877).
                let gated = !regs.is_empty()
                    || (self.adds_reset_port() && self.writes_mem(p));
                if gated {
                    writeln!(out, "    if ({}) begin", crate::comp::RESET_NAME)
                        .unwrap();
                    // Back to the value it started at, which is what the
                    // runtime's reset puts back (issue 728).
                    for r in &regs {
                        let v = match self.reg_init(r) {
                            0 => "0".to_string(),
                            v => format!("{}'h{v:x}", self.reg_width(r)),
                        };
                        writeln!(out, "      {r} <= {v};").unwrap();
                    }
                    writeln!(out, "    end else begin").unwrap();
                }
                for l in &seq {
                    let pad = if gated { "  " } else { "" };
                    writeln!(out, "{pad}{l}").unwrap();
                }
                if gated {
                    writeln!(out, "    end").unwrap();
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
            out.push_str(&inst.unit.verilog_in(chan));
        }
        out
    }

    /// The VHDL, 2008: an entity, a clocked process for each of the
    /// unit's processes, on its clock's rising edge or its falling one
    /// as the process waits, and a concurrent assignment per wire.
    pub fn vhdl(&self) -> String {
        let esc = self.escaped();
        let chans = self.has_chan_nets();
        let chan = if chans {
            esc.chan_name()
        } else {
            format!("{}_txhdl_chan", esc.name)
        };
        let mut out = String::new();
        if chans {
            out.push_str(&CHAN_VHDL.replace("txhdl_chan", &chan));
            out.push('\n');
        }
        if esc.has_unreg_nets() {
            out.push_str(&CHAN_VHDL_U.replace("txhdl_chan", &chan));
            out.push('\n');
        }
        out.push_str(&esc.vhdl_in(&chan));
        out
    }
    /// This unit's entity and its children's, without the channel
    /// entity, which the outermost unit puts first once.
    fn vhdl_in(&self, chan: &str) -> String {
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
        // The reset, beside the clocks, as in the Verilog.
        if self.adds_reset_port() {
            plist.push(format!("{} : in std_logic", crate::comp::RESET_NAME));
        }
        for (n, k, w, _) in &self.ports {
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
            let child = inst.unit.vhdl_in(chan);
            if !child.is_empty() {
                out.push_str(&child);
                out.push('\n');
            }
        }
        out.push_str(
            "library ieee;\nuse ieee.std_logic_1164.all;\n\
             use ieee.numeric_std.all;\n\n",
        );
        out.push_str(&self.renamed_lets("--"));
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
        // A synchroniser's stages say so (#884); the attribute is
        // declared once for the architecture and given per signal.
        if !self.async_regs.is_empty() {
            out.push_str("  attribute ASYNC_REG : string;\n");
        }
        // Whether `attribute ram_style : string;` is declared yet.
        let mut styled = false;
        // And `attribute use_dsp : string;` (issue 1383).
        let mut dsp_declared = false;
        for (n, k, w, d) in &self.fields {
            let init = if *w == 1 {
                "'0'".to_string()
            } else {
                "(others => '0')".to_string()
            };
            match k {
                // What the unit's `Default` gave it, or `init_reg` for a
                // top built otherwise; zero if neither names it.
                Some(Kind::Reg) => {
                    let start = self.reg_init_vhdl(n);
                    writeln!(out, "  signal {n} : {} := {start};", ty(*w))
                        .unwrap();
                    if self.async_regs.contains(n) {
                        writeln!(
                            out,
                            "  attribute ASYNC_REG of {n} : signal is \"TRUE\";"
                        )
                        .unwrap()
                    }
                    if let Some(v) = self.use_dsp_of(n) {
                        if !std::mem::replace(&mut dsp_declared, true) {
                            writeln!(out, "  attribute use_dsp : string;")
                                .unwrap();
                        }
                        writeln!(
                            out,
                            "  attribute use_dsp of {n} : signal is \"{v}\";"
                        )
                        .unwrap();
                    }
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
                    .unwrap();
                    // The attribute is declared once in the
                    // architecture, and given to each memory that asks
                    // (issue 1371).
                    if let Some(st) = self.ram_style(n) {
                        if !std::mem::replace(&mut styled, true) {
                            writeln!(out, "  attribute ram_style : string;")
                                .unwrap();
                        }
                        writeln!(
                            out,
                            "  attribute ram_style of {n} : signal is \"{st}\";"
                        )
                        .unwrap();
                    }
                }
                _ => {}
            }
        }
        let (procs, wires, temps) = self.hoisted();
        for (n, e) in &wires {
            let w = self.ewidth(e);
            assert!(w > 0, "wire `{n}` has no width: size its literals");
            writeln!(out, "  signal {n} : {};", ty(w)).unwrap();
            // A product a register's `use_dsp` reads (issue 1343).
            if let Some(v) = dsp_wire(n) {
                if !std::mem::replace(&mut dsp_declared, true) {
                    writeln!(out, "  attribute use_dsp : string;").unwrap();
                }
                writeln!(
                    out,
                    "  attribute use_dsp of {n} : signal is \"{v}\";"
                )
                .unwrap();
            }
        }
        for (t, w, _) in &temps {
            writeln!(out, "  signal {t} : {};", ty(*w)).unwrap();
            // A product under a register's `use_dsp` (issue 1343).
            if let Some(v) = dsp_wire(t) {
                if !std::mem::replace(&mut dsp_declared, true) {
                    writeln!(out, "  attribute use_dsp : string;").unwrap();
                }
                writeln!(
                    out,
                    "  attribute use_dsp of {t} : signal is \"{v}\";"
                )
                .unwrap();
            }
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
                .chain(inst.unit.ports.iter().map(|(p, k, w, _)| {
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
                // A one-bit channel's nets are `std_logic`, as the
                // children's ports are, and the channel's data ports are
                // `unsigned(0 downto 0)`, so the bit is joined by its
                // element (issue 953).
                let (td, rd) = if *w == 1 {
                    ("tx_data(0)", "rx_data(0)")
                } else {
                    ("tx_data", "rx_data")
                };
                writeln!(
                    out,
                    "  {n}_chan : entity work.{m} \
                     generic map (W => {w}) port map (\n    clk => {c}, \
                     rst => {rs},\n    \
                     {td} => {n}_tx_data, tx_valid => {n}_tx_valid, \
                     tx_ready => {n}_tx_ready,\n    \
                     {rd} => {n}_rx_data, rx_valid => {n}_rx_valid, \
                     rx_ready => {n}_rx_ready\n  );",
                    m = self.chan_module(chan, n),
                    rs = crate::comp::RESET_NAME
                )
                .unwrap();
            }
        }
        // An instance's label, which VHDL keeps in one region with the
        // components and reads without case, so a field named as a
        // component's module is suffixed until it is neither that nor
        // another child's label; the Verilog keeps the field's name,
        // since it keeps modules and instances apart (#816).
        let label = |inst: &Instance| -> String {
            let taken = |l: &str| {
                declared.iter().any(|m| m.eq_ignore_ascii_case(l))
                    || self.instances.iter().any(|i| {
                        !std::ptr::eq(i, inst) && i.name.eq_ignore_ascii_case(l)
                    })
            };
            let mut l = inst.name.clone();
            while taken(&l) {
                l.push_str("_i");
            }
            l
        };
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
            // The reset down to a lowered child, as in the Verilog.
            if inst.unit.adds_reset_port() {
                let r = crate::comp::RESET_NAME;
                conns.push(format!("{r} => {r}"));
            }
            if let Some(f) = &inst.unit.foreign {
                // The component's vectors are `std_logic_vector` and the
                // netlist's are `unsigned`, so a vector crosses with a
                // conversion: on the actual for an input, on the formal
                // for an output. A pad is `std_logic_vector` on both.
                let joins = self.joins(inst);
                for (a, b) in &joins {
                    let (_, k, w, _) = inst
                        .unit
                        .ports
                        .iter()
                        .find(|(n, _, _, _)| n == a)
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
                    label(inst),
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
                label(inst),
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
                Expr::Num(k) if w > 1 => hnum(*k, w),
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
                let word = format!("{m}({})", hint(a, self));
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
            rst: Option<&str>,
        ) {
            let inner = format!("{ind}  ");
            let open = |i: usize, c: &str| {
                let kw = if i == 0 { "if" } else { "elsif" };
                format!("{ind}{kw} {c} then")
            };
            let else_line = format!("{ind}else");
            let end = format!("{ind}end if;");
            match st {
                Stmt::Guard(_) => {}
                // A check or an assumption is VHDL's own `assert`, which a
                // simulator checks; a cover point reports its message
                // when it is reached (issue 502).
                Stmt::Check(k, c, m) => {
                    let m = m.replace('"', "\"\"");
                    // Out of reset only, as in the Verilog (issue 633).
                    let (held, out) = match rst {
                        Some(r) => (
                            format!("{r} = '1' or "),
                            format!("{r} = '0' and "),
                        ),
                        None => (String::new(), String::new()),
                    };
                    match k {
                        Checked::Assert | Checked::Assume => seq.push(format!(
                            "{ind}assert {held}({}) report \"{m}\" \
                             severity failure;",
                            hbool(c, l)
                        )),
                        Checked::Cover => seq.push(format!(
                            "{ind}if {out}({}) then report \"cover: {m}\" \
                             severity note; end if;",
                            hbool(c, l)
                        )),
                    }
                }
                Stmt::Drive(t, e) => drive(seq, comb, ind, t, e),
                Stmt::When(c, then, otherwise) => {
                    let mut a = Vec::new();
                    for (t, e) in then {
                        drive(&mut a, comb, &inner, t, e);
                    }
                    let mut b = Vec::new();
                    for (t, e) in otherwise {
                        drive(&mut b, comb, &inner, t, e);
                    }
                    let arms = vec![(hbool(c, l), a)];
                    chain(seq, arms, b, &open, &else_line, &end);
                }
                Stmt::Case(arms) => {
                    let arms = arms
                        .iter()
                        .map(|(c, drives)| {
                            let mut b = Vec::new();
                            for (t, e) in drives {
                                drive(&mut b, comb, &inner, t, e);
                            }
                            (hbool(c, l), b)
                        })
                        .collect();
                    chain(seq, arms, Vec::new(), &open, &else_line, &end);
                }
                Stmt::If(arms, els) => {
                    let arms = arms
                        .iter()
                        .map(|(c, body)| {
                            let mut b = Vec::new();
                            for s in body {
                                stmt(&mut b, comb, &inner, s, drive, l, rst);
                            }
                            (hbool(c, l), b)
                        })
                        .collect();
                    let mut b = Vec::new();
                    for s in els {
                        stmt(&mut b, comb, &inner, s, drive, l, rst);
                    }
                    chain(seq, arms, b, &open, &else_line, &end);
                }
            }
        }
        // The reset a statement is gated on, where the module has one,
        // whichever side declared it (issue 633).
        let rst = self.has_reset().then_some(crate::comp::RESET_NAME);
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
                stmt(&mut seq, &mut comb, ind, st, &drive, self, rst);
            }
            if guard {
                seq.push("      end if;".into());
            }
            if seq
                .iter()
                .any(|l| l.contains("<=") || l.contains(" severity "))
            {
                let c = p.clock;
                let edge = if p.falling { "falling" } else { "rising" };
                writeln!(
                    out,
                    "  process ({c})\n  begin\n    if {edge}_edge({c}) then"
                )
                .unwrap();
                // The reset, inside the clocked part, as in the
                // Verilog: only the registers this process drives.
                let regs = if self.adds_reset_port() {
                    self.reset_regs(p)
                } else {
                    Vec::new()
                };
                let gated = !regs.is_empty()
                    || (self.adds_reset_port() && self.writes_mem(p));
                if gated {
                    writeln!(
                        out,
                        "      if {} = '1' then",
                        crate::comp::RESET_NAME
                    )
                    .unwrap();
                    for r in &regs {
                        let z = self.reg_init_vhdl(r);
                        writeln!(out, "        {r} <= {z};").unwrap();
                    }
                    if regs.is_empty() {
                        writeln!(out, "        null;").unwrap();
                    }
                    writeln!(out, "      else").unwrap();
                }
                for l in &seq {
                    let pad = if gated { "  " } else { "" };
                    writeln!(out, "{pad}{l}").unwrap();
                }
                if gated {
                    writeln!(out, "      end if;").unwrap();
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
/// first free place, the take first. A free place loads whatever is
/// offered, valid or not, and only its valid bit says whether it holds
/// anything, so the sender's valid enables no data bit (issue 1321).
/// In Verilog, a module. Its name,
/// `txhdl_chan` here, is replaced by the netlist's own, `<top>_txhdl_chan`
/// (issue 979).
const CHAN_VERILOG: &str = "`timescale 1ns/1ps
module txhdl_chan #(parameter W = 1)(
  input clk,
  input rst,
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
    if (rst) begin
      // The two valid bits and nothing else: a channel is empty when
      // nothing in it is valid, and the data behind them is not read.
      head_v <= 1'b0;
      tail_v <= 1'b0;
    end else begin
      // The head loads whenever it will be empty and the tail
      // whenever it is, whatever is offered, so the sender's valid is
      // on the two valid bits and not on every data bit's enable
      // (issue 1321). What a free register takes is read only once its
      // valid says so.
      head_v <= hv1 | tx_valid;
      head <= hv1 ? h1 : tx_data;
      tail_v <= tv1 | (tx_valid & hv1);
      tail <= tv1 ? tail : tx_data;
    end
  end
  assign rx_data = head_v ? head : {W{1'b0}};
  assign rx_valid = head_v;
  assign tx_ready = ~tail_v;
endmodule
";

/// The unregistered channel (issue 1293): the same buffer of two, with
/// the receiver's `valid` and `data` passing the sender's straight
/// through while the buffer is empty. An offer the receiver takes in
/// that cycle, `thru`, does not enter the buffer; one it does not take
/// is buffered as the registered channel's is. `tx_ready` is the tail's
/// room, a register, so no path runs from the receiver back to the
/// sender. Named `<top>_txhdl_chan_u`.
const CHAN_VERILOG_U: &str = "`timescale 1ns/1ps
module txhdl_chan_u #(parameter W = 1)(
  input clk,
  input rst,
  input [W-1:0] tx_data, input tx_valid, output tx_ready,
  output [W-1:0] rx_data, output rx_valid, input rx_ready
);
  reg [W-1:0] head = 0;
  reg [W-1:0] tail = 0;
  reg head_v = 0;
  reg tail_v = 0;
  wire thru = tx_valid & ~head_v & rx_ready;
  wire push = tx_valid & ~thru;
  wire pop = rx_ready & head_v;
  wire hv1 = pop ? tail_v : head_v;
  wire [W-1:0] h1 = pop ? tail : head;
  wire tv1 = pop ? 1'b0 : tail_v;
  always @(posedge clk) begin
    if (rst) begin
      head_v <= 1'b0;
      tail_v <= 1'b0;
    end else begin
      // Free registers load whatever is offered, as the registered
      // channel's do (issue 1321).
      head_v <= hv1 | push;
      head <= hv1 ? h1 : tx_data;
      tail_v <= tv1 | (push & hv1);
      tail <= tv1 ? tail : tx_data;
    end
  end
  assign rx_data = head_v ? head : tx_valid ? tx_data : {W{1'b0}};
  assign rx_valid = head_v | tx_valid;
  assign tx_ready = ~tail_v;
endmodule
";

/// The unregistered channel in VHDL, named per netlist the same way.
const CHAN_VHDL_U: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity txhdl_chan_u is
  generic (W : natural);
  port (
    clk : in std_logic;
    rst : in std_logic;
    tx_data : in unsigned(W - 1 downto 0);
    tx_valid : in std_logic;
    tx_ready : out std_logic;
    rx_data : out unsigned(W - 1 downto 0);
    rx_valid : out std_logic;
    rx_ready : in std_logic
  );
end entity;

architecture rtl of txhdl_chan_u is
  signal head, tail : unsigned(W - 1 downto 0) := (others => '0');
  signal head_v, tail_v : std_logic := '0';
begin
  process (clk)
    variable h, t : unsigned(W - 1 downto 0);
    variable hv, tv, push : std_logic;
  begin
    if rising_edge(clk) then
      if rst = '1' then
        head_v <= '0'; tail_v <= '0';
      else
        h := head; t := tail; hv := head_v; tv := tail_v;
        push := tx_valid;
        if tx_valid = '1' and head_v = '0' and rx_ready = '1' then
          push := '0';
        end if;
        if rx_ready = '1' and head_v = '1' then
          h := tail; hv := tail_v; tv := '0';
        end if;
        -- Free registers load whatever is offered (issue 1321).
        if hv = '0' then h := tx_data; end if;
        if tv = '0' then t := tx_data; end if;
        if push = '1' then
          if hv = '0' then hv := '1'; else tv := '1'; end if;
        end if;
        head <= h; tail <= t; head_v <= hv; tail_v <= tv;
      end if;
    end if;
  end process;
  rx_data <= head when head_v = '1' else
    tx_data when tx_valid = '1' else (others => '0');
  rx_valid <= head_v or tx_valid;
  tx_ready <= not tail_v;
end architecture;
";

/// The same channel in VHDL, an entity with the width as a generic,
/// named per netlist the same way.
const CHAN_VHDL: &str = "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity txhdl_chan is
  generic (W : natural);
  port (
    clk : in std_logic;
    rst : in std_logic;
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
      if rst = '1' then
        -- The two valid bits and nothing else: a channel is empty
        -- when nothing in it is valid, and the data is not read.
        head_v <= '0'; tail_v <= '0';
      else
        h := head; t := tail; hv := head_v; tv := tail_v;
        if rx_ready = '1' and head_v = '1' then
          h := tail; hv := tail_v; tv := '0';
        end if;
        -- The head loads whenever it will be empty and the tail
        -- whenever it is, so the sender's valid is on the valid bits
        -- alone and not on every data bit's enable (issue 1321).
        if hv = '0' then h := tx_data; end if;
        if tv = '0' then t := tx_data; end if;
        if tx_valid = '1' then
          if hv = '0' then hv := '1'; else tv := '1'; end if;
        end if;
        head <= h; tail <= t; head_v <= hv; tail_v <= tv;
      end if;
    end if;
  end process;
  rx_data <= head when head_v = '1' else (others => '0');
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
            .map(|l| format!("entity {}\n{}", l.escaped().name, l.ports_file()))
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
        // A value of one bit is its own top bit, and has to be written
        // that way: a wire or a register of one bit is a scalar in
        // Verilog, which has no bit to select, so `x[0]` is an error
        // rather than the bit. See issue 247.
        _ if l.ewidth(e) <= 1 => vexpr(e, l),
        _ => format!("{}[{}]", vexpr(e, l), l.ewidth(e).max(1) - 1),
    }
}

/// An expression in Verilog.
/// A chain of conditions in a clocked block, its bodies already
/// written: `open` writes the line before arm `i` with its condition,
/// `els` the line before the statements under no condition, `end` the
/// line that closes the chain. A body with nothing in the block, such
/// as a send under an `if`, whose drives are all wires, is no reason
/// to write its test: an empty `else` is left out, then the empty arms
/// at the end, and a chain with nothing left is not written at all
/// (issue 903). An empty arm before one that is kept stays, since it
/// is what keeps the later arm from taking its case.
fn chain(
    seq: &mut Vec<String>,
    mut arms: Vec<(String, Vec<String>)>,
    els: Vec<String>,
    open: &dyn Fn(usize, &str) -> String,
    else_line: &str,
    end: &str,
) {
    if els.is_empty() {
        while arms.last().is_some_and(|(_, b)| b.is_empty()) {
            arms.pop();
        }
    }
    if arms.is_empty() {
        seq.extend(els);
        return;
    }
    for (i, (c, body)) in arms.into_iter().enumerate() {
        seq.push(open(i, &c));
        seq.extend(body);
    }
    if !els.is_empty() {
        seq.push(else_line.to_string());
        seq.extend(els);
    }
    seq.push(end.to_string());
}

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
        // A number beside a value of known width is sized to it: an
        // unsized decimal is a 32-bit signed integer in Verilog.
        Expr::Bin(op, a, b) => {
            let w = l.ewidth(a).max(l.ewidth(b));
            let side = |x: &Expr| match x {
                Expr::Num(k) if w > 0 => vnum(*k, w),
                x => vexpr(x, l),
            };
            format!("({} {op} {})", side(a), side(b))
        }
        Expr::Not(a) if a.is_bool() => format!("(!{})", vexpr(a, l)),
        Expr::Not(a) => format!("(~{})", vexpr(a, l)),
        // A number in a branch takes the other branch's width: unsized,
        // it would be 32 bits wide inside a concatenation.
        Expr::Cond(c, a, b) => {
            let w = l.ewidth(a).max(l.ewidth(b));
            let side = |x: &Expr| match x {
                Expr::Num(k) if w > 0 => vnum(*k, w),
                x => vexpr(x, l),
            };
            format!("({} ? {} : {})", vexpr(c, l), side(a), side(b))
        }
        // A bit of a one-bit value, which is declared without a range
        // and cannot be selected from: bit nought is the value itself
        // (issue 647). A memory's word is indexed as ever.
        Expr::Index(a, i)
            if l.ewidth(a) == 1
                && matches!(**i, Expr::Num(_))
                && !matches!(&**a, Expr::Name(m) if l.is_mem(m)) =>
        {
            if matches!(**i, Expr::Num(0)) {
                vexpr(a, l)
            } else {
                "1'b0".into()
            }
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
        // `hoisted` resolves every cast before an emitter sees it.
        Expr::Cast(a, _) => vexpr(a, l),
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
            "&" | "|" | "^" | "<<" | ">>" | ">>>" | "+" | "-" | "*" | "%",
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

/// A number as bits, most significant first, in `w` of them.
fn numbits(k: u128, w: usize) -> String {
    (0..w)
        .rev()
        .map(|i| if (k >> i) & 1 == 1 { '1' } else { '0' })
        .collect()
}

/// A number as a VHDL value of width `w`. `to_unsigned` reads well and
/// is what a small number becomes, but its first argument is a
/// `natural`, and VHDL guarantees a natural only to 2^31 - 1, so a
/// number at or above that is written as a bit string instead (issue
/// 130).
fn hnum(k: u128, w: usize) -> String {
    if k < (1u128 << 31) {
        format!("to_unsigned({k}, {w})")
    } else {
        format!("unsigned'(\"{}\")", numbits(k, w))
    }
}

/// A number as a Verilog value of width `w`. An unsized decimal is a
/// 32-bit signed integer there, so a number at or above 2^31 overflows
/// it; every number a width is known for is therefore sized (issue
/// 130).
fn vnum(k: u128, w: usize) -> String {
    let w = w.max(128 - k.leading_zeros() as usize).max(1);
    format!("{w}'h{k:x}")
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
        // A number a width is known for is written at that width; one
        // in a context with no width stays an integer, which is what
        // VHDL's own arithmetic on a natural wants.
        Expr::Num(k) if w > 1 => hnum(*k, w),
        Expr::Num(k) => k.to_string(),
        Expr::Bits(bw, b) if *bw == 1 => format!("'{b}'"),
        Expr::Bits(_, b) => format!("unsigned'(\"{b}\")"),
        // One bit is a `std_logic`, which has no `+`, `-` or `*`. Its
        // arithmetic is modulo two: a sum and a difference are both
        // the exclusive or, and a product the and, of the two bits,
        // which is what a one-bit register incremented wraps to and
        // what the Verilog does at that width by itself. A number is
        // its low bit there, so `+ 2` adds nothing. See issue 461.
        Expr::Bin(op @ ("+" | "-" | "*"), a, b)
            if w == 1 || (w == 0 && l.ewidth(e) == 1) =>
        {
            let bit = |x: &Expr| match x {
                Expr::Num(k) => format!("'{}'", k & 1),
                x => hval(x, 1, l),
            };
            let vop = if *op == "*" { "and" } else { "xor" };
            format!("({} {vop} {})", bit(a), bit(b))
        }
        Expr::Bin(op @ ("+" | "-"), a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
        // A product is twice as wide as its operands in VHDL; the
        // lowering states its width, so the product is cut to it.
        Expr::Bin("*", a, b) => {
            let w = if w == 0 { l.ewidth(e) } else { w };
            format!("resize(({} * {}), {w})", hval(a, w, l), hval(b, w, l))
        }
        // A remainder is `rem`, which numeric_std answers at the right
        // operand's width, so it is resized to the expression's (issue
        // 496). Zero on the right stops nvc, as it stops the Rust run.
        Expr::Bin("%", a, b) => {
            let w = if w == 0 { l.ewidth(e) } else { w };
            format!("resize(({} rem {}), {w})", hval(a, w, l), hval(b, w, l))
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
                Expr::Num(k) if w > 1 => hnum(*k, w),
                x => hval(x, w, l),
            };
            format!("({} {vop} {})", side(a), side(b))
        }
        // A one-bit value is a `std_logic`, which numeric_std does not
        // shift: moved by nought it is itself, and by more it is gone.
        // A unit over an array of one meets it, `1 << i` for `i` of
        // nought (issue 647).
        Expr::Bin("<<" | ">>", a, b)
            if w == 1 && l.ewidth(a) <= 1 && matches!(**b, Expr::Num(_)) =>
        {
            if matches!(**b, Expr::Num(0)) {
                hval(a, 1, l)
            } else {
                "'0'".into()
            }
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
        // A slice of one bit where a bit is wanted is an index and
        // not a range. `a(7 downto 7)` is an `unsigned` of one
        // element, and there is no `=` between that and `'1'`, so
        // nvc calls the literal ambiguous; `a(7)` is `std_logic`,
        // which is what every other one-bit value here is. See
        // issue 160.
        Expr::Slice(a, lo, 1) if w == 1 => {
            format!("{}({lo})", hval(a, 0, l))
        }
        Expr::Slice(a, lo, len) => {
            format!("{}({} downto {})", hval(a, 0, l), lo + len - 1, lo)
        }
        // Qualified, since every array type in scope has a `&` too.
        Expr::Cat(a, b) => {
            format!("unsigned'({} & {})", hval(a, 0, l), hval(b, 0, l))
        }
        // A value of one bit is a `std_logic`, and `signed` converts
        // only between closely related types, so nvc refuses
        // `signed(x)` on it. Its sign extension is that bit repeated,
        // which an aggregate says outright. See issue 247.
        Expr::Sext(a, m) if l.ewidth(a) == 1 => {
            format!("unsigned'({} downto 0 => {})", m - 1, hval(a, 1, l))
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
        // `hoisted` resolves every cast before an emitter sees it.
        Expr::Cast(a, _) => hval(a, w, l),
        // A word of a memory, or a bit of a value.
        Expr::Index(a, i) => match &**a {
            Expr::Name(m) if l.is_mem(m) => {
                format!("{m}({})", hint(i, l))
            }
            // A bit of a one-bit value, which VHDL holds as a
            // `std_logic` and does not index: bit nought is the value
            // itself (issue 647).
            _ if l.ewidth(a) == 1 => match &**i {
                Expr::Num(0) => hval(a, 1, l),
                Expr::Num(_) => "'0'".into(),
                _ => format!("{}({})", hval(a, 0, l), hint(i, l)),
            },
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
                Expr::Num(k) if w > 1 => hnum(*k, w),
                x => hval(x, w, l),
            };
            format!("mux({}, {}, {})", hbool(c, l), side(a), side(b))
        }
        Expr::Bin(op, a, b) => {
            format!("({} {op} {})", hval(a, w, l), hval(b, w, l))
        }
    }
}

// ---------------------------------------------------------------------
// Combinational paths through a unit, and the loops unregistered
// channels can close (issue 1293)

impl Expr {
    /// Every name the expression reads.
    fn names_into(&self, out: &mut Vec<String>) {
        match self {
            Expr::Name(n) => out.push(n.clone()),
            Expr::Num(_) | Expr::Bits(_, _) => {}
            Expr::Bin(_, a, b) | Expr::Index(a, b) | Expr::Cat(a, b) => {
                a.names_into(out);
                b.names_into(out);
            }
            Expr::Not(a)
            | Expr::Slice(a, _, _)
            | Expr::Sext(a, _)
            | Expr::Zext(a, _)
            | Expr::Cast(a, _) => a.names_into(out),
            Expr::Cond(c, a, b) => {
                c.names_into(out);
                a.names_into(out);
                b.names_into(out);
            }
        }
    }
}

/// The drives of a body, each with every name its value and the
/// conditions over it read: what an output driven by it waits on in
/// the cycle.
fn drives_of(
    body: &[Stmt],
    ctx: &[String],
    out: &mut Vec<(String, Vec<String>)>,
) {
    let mut ctx = ctx.to_vec();
    let drive = |t: &Target,
                 e: &Expr,
                 ctx: &[String],
                 out: &mut Vec<(String, Vec<String>)>| {
        if let Target::Name(n) = t {
            let mut names = ctx.to_vec();
            e.names_into(&mut names);
            out.push((n.clone(), names));
        }
    };
    for st in body {
        match st {
            Stmt::Drive(t, e) => drive(t, e, &ctx, out),
            Stmt::When(c, a, b) => {
                let mut cx = ctx.clone();
                c.names_into(&mut cx);
                for (t, e) in a.iter().chain(b) {
                    drive(t, e, &cx, out);
                }
            }
            Stmt::Case(arms) => {
                let mut cx = ctx.clone();
                for (c, ds) in arms {
                    c.names_into(&mut cx);
                    for (t, e) in ds {
                        drive(t, e, &cx, out);
                    }
                }
            }
            Stmt::If(arms, els) => {
                let mut cx = ctx.clone();
                for (c, ss) in arms {
                    c.names_into(&mut cx);
                    drives_of(ss, &cx, out);
                }
                drives_of(els, &cx, out);
            }
            Stmt::Guard(c) => c.names_into(&mut ctx),
            Stmt::Check(..) => {}
        }
    }
}

impl Lowered {
    /// The input port a net belongs to, on the forward path: a
    /// receiving port's `valid` or `data`, or a wire in.
    fn in_port_of(&self, net: &str) -> Option<String> {
        self.ports.iter().find_map(|(p, k, _, _)| match k {
            Kind::Rx
                if net == format!("{p}_valid")
                    || net == format!("{p}_data") =>
            {
                Some(p.clone())
            }
            Kind::In if net == p => Some(p.clone()),
            _ => None,
        })
    }
    /// The output port a net belongs to, on the forward path: a
    /// sending port's `valid` or `data`, or a wire out.
    fn out_port_of(&self, net: &str) -> Option<String> {
        self.ports.iter().find_map(|(p, k, _, _)| match k {
            Kind::Tx
                if net == format!("{p}_valid")
                    || net == format!("{p}_data") =>
            {
                Some(p.clone())
            }
            Kind::Out if net == p => Some(p.clone()),
            _ => None,
        })
    }
    /// The pairs of an input port and an output port this unit joins
    /// through wires in one cycle, with no register between: a sending
    /// port's `valid` or `data` that depends on a receiving port's
    /// `valid` or `data`, or on a wire in (issue 1293). A foreign
    /// unit, whose netlist is not this one's to read, joins every input
    /// to every output.
    pub fn comb_paths(&self) -> Vec<(String, String)> {
        let ins = |k: &Kind| matches!(k, Kind::Rx | Kind::In);
        let outs = |k: &Kind| matches!(k, Kind::Tx | Kind::Out);
        if self.foreign.is_some() {
            let mut v = Vec::new();
            for (p, k, _, _) in &self.ports {
                for (q, j, _, _) in &self.ports {
                    if ins(k) && outs(j) {
                        v.push((p.clone(), q.clone()));
                    }
                }
            }
            return v;
        }
        if !self.instances.is_empty() {
            let (edges, _) = self.port_graph();
            let mut v = Vec::new();
            for (p, k, _, _) in &self.ports {
                if !ins(k) {
                    continue;
                }
                let reach = reachable(&edges, &format!("port:{p}"));
                for (q, j, _, _) in &self.ports {
                    if outs(j) && reach.contains(&format!("port:{q}")) {
                        v.push((p.clone(), q.clone()));
                    }
                }
            }
            return v;
        }
        let wires: std::collections::HashMap<&str, &Expr> =
            self.wires.iter().map(|(n, e)| (n.as_str(), e)).collect();
        let mut drives = Vec::new();
        for p in &self.procs {
            drives_of(&p.body, &[], &mut drives);
        }
        let mut v: Vec<(String, String)> = Vec::new();
        for (t, names) in &drives {
            let Some(q) = self.out_port_of(t) else {
                continue;
            };
            // Through the wires to what they read, stopping at
            // registers, memories and ports.
            let mut seen: Vec<String> = Vec::new();
            let mut todo = names.clone();
            while let Some(n) = todo.pop() {
                if seen.contains(&n) {
                    continue;
                }
                if let Some(e) = wires.get(n.as_str()) {
                    e.names_into(&mut todo);
                }
                if let Some(p) = self.in_port_of(&n) {
                    if !v.contains(&(p.clone(), q.clone())) {
                        v.push((p, q.clone()));
                    }
                }
                seen.push(n);
            }
        }
        v
    }
    /// The graph of a unit of units' combinational paths, over its own
    /// ports, `port:P`, and its children's, `CHILD.P`: each child's own
    /// paths; an unregistered channel from its sender to its receiver;
    /// a wire from its driver to its readers; and a child's port joined
    /// to one of the unit's own. With it, the edges that are an
    /// unregistered channel's.
    fn port_graph(&self) -> (Vec<(String, String)>, Vec<(String, String)>) {
        let mut edges: Vec<(String, String)> = Vec::new();
        let mut unreg: Vec<(String, String)> = Vec::new();
        let kind_of = |inst: &Instance, p: &str| {
            inst.unit
                .ports
                .iter()
                .find(|(n, _, _, _)| n == p)
                .map(|(_, k, _, _)| *k)
        };
        for inst in &self.instances {
            for (p, q) in inst.unit.comb_paths() {
                edges.push((
                    format!("{}.{p}", inst.name),
                    format!("{}.{q}", inst.name),
                ));
            }
            for (cp, to) in &inst.conns {
                let node = format!("{}.{cp}", inst.name);
                let Some(k) = kind_of(inst, cp) else {
                    continue;
                };
                if let Some((_, pk, _, _)) =
                    self.ports.iter().find(|(n, _, _, _)| n == to)
                {
                    // A child's port joined to one of the unit's own.
                    match (pk, k) {
                        (Kind::Rx | Kind::In, _) => {
                            edges.push((format!("port:{to}"), node))
                        }
                        (Kind::Tx | Kind::Out, _) => {
                            edges.push((node, format!("port:{to}")))
                        }
                        _ => {}
                    }
                    continue;
                }
                // A net between children: an edge from each driver's
                // port to each reader's, for a wire and for an
                // unregistered channel.
                let Some((_, nk, _, _)) =
                    self.nets.iter().find(|(n, _, _, _)| n == to)
                else {
                    continue;
                };
                let is_unreg = self.unregistered.iter().any(|u| u == to);
                let wire = matches!(nk, Kind::Out);
                if !(wire || is_unreg) || !matches!(k, Kind::Tx | Kind::Out) {
                    continue;
                }
                for other in &self.instances {
                    for (op, oto) in &other.conns {
                        if oto != to
                            || matches!(
                                kind_of(other, op),
                                Some(Kind::Tx | Kind::Out)
                            )
                        {
                            continue;
                        }
                        let e = (node.clone(), format!("{}.{op}", other.name));
                        if is_unreg {
                            unreg.push(e.clone());
                        }
                        edges.push(e);
                    }
                }
            }
        }
        (edges, unreg)
    }
    /// Refuse a unit of units whose unregistered channels close a loop
    /// of wires: a ring through its children, each joined to the next
    /// with no register between, in which one of the joins is an
    /// unregistered channel (issue 1293). Such a ring is a
    /// combinational loop in the netlist, and no order of the run's
    /// processes lets every sender run before its receiver.
    fn refuse_comb_loops(&self) {
        if self.unregistered.is_empty() {
            return;
        }
        let (edges, unreg) = self.port_graph();
        for (a, b) in &unreg {
            // `a` to `b` is the channel; a way back from `b` to `a`
            // closes the ring.
            if reachable(&edges, b).contains(a) {
                let ring = path(&edges, b, a);
                panic!(
                    "`{}` closes a combinational loop through an \
                     unregistered channel, from `{a}` to `{b}` and back: \
                     {}. Leave one of its channels registered (issue 1293)",
                    self.name,
                    ring.join(" -> ")
                );
            }
        }
    }
}

/// Every address `e` reads `mem` at: each `Index` of the memory's name.
fn mem_reads(e: &Expr, mem: &str, out: &mut Vec<String>) {
    match e {
        Expr::Index(m, a) => {
            if matches!(m.as_ref(), Expr::Name(n) if n == mem) {
                let k = format!("{a:?}");
                if !out.contains(&k) {
                    out.push(k);
                }
            } else {
                mem_reads(m, mem, out);
            }
            mem_reads(a, mem, out);
        }
        Expr::Name(_) | Expr::Num(_) | Expr::Bits(_, _) => {}
        Expr::Bin(_, a, b) | Expr::Cat(a, b) => {
            mem_reads(a, mem, out);
            mem_reads(b, mem, out);
        }
        Expr::Not(a)
        | Expr::Slice(a, _, _)
        | Expr::Sext(a, _)
        | Expr::Zext(a, _)
        | Expr::Cast(a, _) => mem_reads(a, mem, out),
        Expr::Cond(c, a, b) => {
            mem_reads(c, mem, out);
            mem_reads(a, mem, out);
            mem_reads(b, mem, out);
        }
    }
}

/// Every address a body reads `mem` at, into `reads`, and writes it at,
/// into `writes`, the conditions over the statements included.
fn mem_sites(
    body: &[Stmt],
    mem: &str,
    reads: &mut Vec<String>,
    writes: &mut Vec<String>,
) {
    let drive = |t: &Target,
                 e: &Expr,
                 reads: &mut Vec<String>,
                 writes: &mut Vec<String>| {
        if let Target::Word(m, a) = t {
            if m == mem {
                let k = format!("{a:?}");
                if !writes.contains(&k) {
                    writes.push(k);
                }
            }
            mem_reads(a, mem, reads);
        }
        mem_reads(e, mem, reads);
    };
    for st in body {
        match st {
            Stmt::Drive(t, e) => drive(t, e, reads, writes),
            Stmt::When(c, a, b) => {
                mem_reads(c, mem, reads);
                for (t, e) in a.iter().chain(b) {
                    drive(t, e, reads, writes);
                }
            }
            Stmt::Case(arms) => {
                for (c, ds) in arms {
                    mem_reads(c, mem, reads);
                    for (t, e) in ds {
                        drive(t, e, reads, writes);
                    }
                }
            }
            Stmt::If(arms, els) => {
                for (c, ss) in arms {
                    mem_reads(c, mem, reads);
                    mem_sites(ss, mem, reads, writes);
                }
                mem_sites(els, mem, reads, writes);
            }
            Stmt::Guard(c) | Stmt::Check(_, c, _) => mem_reads(c, mem, reads),
        }
    }
}

impl Lowered {
    /// Refuse a memory of more than 4096 bits whose accesses no block
    /// RAM has the ports for (issue 1285). A block RAM has two ports,
    /// and a read at the address a write is at shares the write's port,
    /// as a read-modify-write does. So it holds a memory written at one
    /// address and read at one other, or read at two and not written;
    /// a memory written at two addresses, or reached at three, Vivado
    /// builds from LUTs, and says so to no test. A smaller memory, a
    /// register file or a FIFO, is meant to be LUT RAM, as is a field
    /// marked `#[distributed]`.
    fn refuse_unblockable_mems(&self) {
        for (m, k, w, d) in &self.fields {
            if *k != Some(Kind::Mem)
                || w * d <= 4096
                || self.distributed.contains(m)
                || matches!(
                    self.ram_style(m),
                    Some("distributed" | "registers")
                )
            {
                continue;
            }
            let (mut reads, mut writes) = (Vec::new(), Vec::new());
            for p in &self.procs {
                mem_sites(&p.body, m, &mut reads, &mut writes);
            }
            for (_, e) in &self.wires {
                mem_reads(e, m, &mut reads);
            }
            let mut ports = writes.clone();
            for r in &reads {
                if !ports.contains(r) {
                    ports.push(r.clone());
                }
            }
            let too_many = writes.len() > 1 || ports.len() > 2;
            if too_many {
                panic!(
                    "memory `{m}` of `{}`, {d} words of {w} bits, is reached \
                     at {} addresses and written at {}, which no block RAM \
                     has the ports for, so Vivado would build it from LUTs. \
                     Read it at one address a cycle beside the write's, and \
                     write it at one, or mark the field #[distributed] if LUT \
                     RAM is meant (issue 1285). Read at: {}. Written at: {}",
                    self.name,
                    ports.len(),
                    writes.len(),
                    reads.join("; "),
                    writes.join("; ")
                );
            }
        }
    }
}

/// Every node reachable from `from` along `edges`, `from` included.
fn reachable(edges: &[(String, String)], from: &str) -> Vec<String> {
    let mut seen = vec![from.to_string()];
    let mut todo = vec![from.to_string()];
    while let Some(n) = todo.pop() {
        for (a, b) in edges {
            if *a == n && !seen.contains(b) {
                seen.push(b.clone());
                todo.push(b.clone());
            }
        }
    }
    seen
}

/// One path from `from` to `to` along `edges`, by breadth first.
fn path(edges: &[(String, String)], from: &str, to: &str) -> Vec<String> {
    let mut prev: Vec<(String, String)> = Vec::new();
    let mut seen = vec![from.to_string()];
    let mut q = std::collections::VecDeque::from([from.to_string()]);
    while let Some(n) = q.pop_front() {
        if n == to {
            break;
        }
        for (a, b) in edges {
            if *a == n && !seen.contains(b) {
                seen.push(b.clone());
                prev.push((b.clone(), n.clone()));
                q.push_back(b.clone());
            }
        }
    }
    let mut out = vec![to.to_string()];
    let mut cur = to.to_string();
    while let Some((_, p)) = prev.iter().find(|(b, _)| *b == cur) {
        out.push(p.clone());
        cur = p.clone();
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit of three registers and nothing else, which is enough to
    /// read the declarations both emitters write.
    fn three_regs() -> Lowered {
        Lowered {
            name: "regs".to_string(),
            fields: vec![
                ("flag", Some(Kind::Reg), 1, 0),
                ("count", Some(Kind::Reg), 8, 0),
                ("wide", Some(Kind::Reg), 32, 0),
            ],
            ports: Vec::new(),
            wires: Vec::new(),
            wire_names: Vec::new(),
            procs: Vec::new(),
            init: Vec::new(),
            init_regs: Vec::new(),
            async_regs: Vec::new(),
            distributed: Vec::new(),
            ram_styles: Vec::new(),
            use_dsp: Vec::new(),
            aliases: Vec::new(),
            nets: Vec::new(),
            unregistered: Vec::new(),
            instances: Vec::new(),
            foreign: None,
        }
    }

    /// A memory's first words, and a register's first value, are given
    /// only to the unit's own: a name it does not have is refused,
    /// naming it and the unit (issue 1104).
    #[test]
    fn a_first_value_for_a_name_the_unit_lacks_is_refused() {
        let mut net = three_regs();
        net.fields.push(("words", Some(Kind::Mem), 32, 4));
        net.init("words", &[1, 2, 3]);
        net.init_reg("flag", 1);
        let mem = std::panic::catch_unwind(|| {
            let mut n = three_regs();
            n.init("imem", &[1, 2, 3]);
        })
        .expect_err("a memory the unit lacks");
        let said = mem.downcast_ref::<String>().unwrap();
        assert!(said.contains("regs has no memory `imem`"), "{said}");
        let reg = std::panic::catch_unwind(|| {
            let mut n = three_regs();
            n.init_reg("flags", 1);
        })
        .expect_err("a register the unit lacks");
        let said = reg.downcast_ref::<String>().unwrap();
        assert!(said.contains("regs has no register `flags`"), "{said}");
        assert!(said.contains("\"flag\""), "and says what it has: {said}");
        let not_mem = std::panic::catch_unwind(|| {
            let mut n = three_regs();
            n.init("count", &[1]);
        });
        assert!(not_mem.is_err(), "a register is not a memory");
    }

    /// A unit of units whose foreign children sit in fields named as
    /// their modules, one in the same case and one not.
    fn named_as_their_modules() -> Lowered {
        let child = |module: &str| {
            foreign(module, module, &[("EOS", Kind::Out, 1)], &[], &[])
        };
        let mut net = three_regs();
        net.ports = vec![
            ("eos".to_string(), Kind::Out, 1, "clk"),
            ("done".to_string(), Kind::Out, 1, "clk"),
        ];
        net.instances = vec![
            Instance {
                name: "startup".to_string(),
                unit: child("startup"),
                conns: vec![("EOS".to_string(), "eos".to_string())],
            },
            Instance {
                name: "flash".to_string(),
                unit: child("FLASH"),
                conns: vec![("EOS".to_string(), "done".to_string())],
            },
        ];
        net
    }

    /// VHDL puts a component and an instance label in one region and
    /// reads both without case, so a foreign child in a field named as
    /// its module was declared twice, which nvc refuses (#816). Every
    /// label differs from every component, ignoring case; the Verilog
    /// keeps the field's name, since it keeps the two apart.
    #[test]
    fn a_label_is_never_a_components_name() {
        let net = named_as_their_modules();
        let vhdl = net.vhdl();
        let components: Vec<String> = vhdl
            .lines()
            .filter_map(|l| l.strip_prefix("  component "))
            .map(|m| m.trim().to_lowercase())
            .collect();
        assert_eq!(components, ["startup", "flash"], "{vhdl}");
        let labels: Vec<String> = vhdl
            .split("\nbegin\n")
            .nth(1)
            .expect("an architecture body")
            .lines()
            .filter_map(|l| l.strip_prefix("  ")?.split_once(" : "))
            .map(|(label, _)| label.to_lowercase())
            .collect();
        assert_eq!(labels.len(), 2, "{vhdl}");
        for l in &labels {
            assert!(!components.contains(l), "label `{l}`:\n{vhdl}");
        }
        let verilog = net.verilog();
        assert!(verilog.contains("startup startup"), "{verilog}");
        assert!(verilog.contains("FLASH flash"), "{verilog}");
    }

    /// A one-bit value is a `std_logic` in the VHDL and a scalar in
    /// the Verilog, neither of which can be indexed or, in the VHDL,
    /// shifted: bit nought of it is the value, and a move by nought
    /// leaves it. A unit over an array of one meets both, `sel.bit(i)`
    /// and `1 << i` for `i` of nought (issue 647). A wider value is
    /// indexed as ever.
    #[test]
    fn a_bit_of_a_one_bit_value_is_the_value() {
        let net = three_regs();
        let bit = |n: &str, k| {
            Expr::Index(Box::new(Expr::name(n)), Box::new(Expr::Num(k)))
        };
        assert_eq!(vexpr(&bit("flag", 0), &net), "flag");
        assert_eq!(vexpr(&bit("flag", 1), &net), "1'b0");
        assert_eq!(vexpr(&bit("count", 0), &net), "count[0]");
        assert_eq!(hval(&bit("flag", 0), 1, &net), "flag");
        assert_eq!(hval(&bit("flag", 1), 1, &net), "'0'");
        assert_eq!(hval(&bit("count", 3), 1, &net), "count(3)");
        let one = || Expr::Bits(1, "1".to_string());
        let shl = |k| Expr::bin("<<", one(), Expr::Num(k));
        assert_eq!(hval(&shl(0), 1, &net), "'1'");
        assert_eq!(hval(&shl(1), 1, &net), "'0'");
    }

    /// A bit or a part of a literal is a literal, so no netlist indexes
    /// one (issue 555). `1010011` is bit 6 down to bit 0, so bit 0 is
    /// the last character and bits 3 to 1 are `001`.
    #[test]
    fn a_bit_or_a_part_of_a_literal_is_a_literal() {
        let lit = || Expr::Bits(7, "1010011".to_string());
        let bits = |e: Expr| match e {
            Expr::Bits(w, b) => (w, b),
            other => panic!("not folded: {other:?}"),
        };
        assert_eq!(bits(Expr::index(lit(), Expr::Num(0))), (1, "1".into()));
        assert_eq!(bits(Expr::index(lit(), Expr::Num(5))), (1, "0".into()));
        assert_eq!(bits(Expr::index(lit(), Expr::Num(6))), (1, "1".into()));
        assert_eq!(bits(Expr::slice(lit(), 1, 3)), (3, "001".into()));
        assert_eq!(bits(Expr::slice(lit(), 5, 2)), (2, "10".into()));
        // A bit of a signal is still an index.
        assert!(matches!(
            Expr::index(Expr::name("crc"), Expr::Num(6)),
            Expr::Index(..)
        ));
    }

    /// A register nobody says anything about starts at zero, which is
    /// what the runtime's `Reg::default` does.
    #[test]
    fn a_register_starts_at_zero_by_itself() {
        let net = three_regs();
        assert!(net.verilog().contains("reg flag = 1'h0;"), "the flag");
        assert!(
            net.verilog().contains("reg [7:0] count = 8'h0;"),
            "the counter"
        );
        assert!(
            net.vhdl().contains("signal flag : std_logic := '0';"),
            "the flag in VHDL"
        );
        assert!(net.vhdl().contains("(others => '0')"), "the counter");
    }

    /// A one-bit register incremented is its exclusive or with the
    /// number's low bit in VHDL, since a `std_logic` has no `+`;
    /// before issue 461 it was written `flag + '1'`, which nvc refuses
    /// as having no such subprogram. A subtraction is the same, a
    /// product is the and, and the Verilog keeps its own arithmetic,
    /// which wraps at that width by itself.
    #[test]
    fn a_one_bit_register_counts_modulo_two() {
        let flag = || Box::new(Expr::Name("flag".to_string()));
        let mut net = three_regs();
        // One process per drive, each of the one-bit register itself.
        for e in [
            Expr::Bin("+", flag(), Box::new(Expr::Num(1))),
            Expr::Bin("-", flag(), Box::new(Expr::Num(2))),
            Expr::Bin("*", flag(), flag()),
        ] {
            net.procs.push(Process {
                clock: "clk",
                falling: false,
                body: vec![Stmt::Drive(Target::Name("flag".to_string()), e)],
            });
        }
        let h = net.vhdl();
        assert!(h.contains("flag <= (flag xor '1');"), "the sum: {h}");
        assert!(!h.contains("flag + '1'"), "no plus on a bit: {h}");
        assert!(
            h.contains("flag <= (flag xor '0');"),
            "two adds nothing: {h}"
        );

        assert!(h.contains("flag <= (flag and flag);"), "the product: {h}");
        let v = net.verilog();
        assert!(v.contains("flag <= (flag + 1'h1);"), "the Verilog: {v}");
    }

    /// And one that `Reg::new` gave a value starts there, in both
    /// netlists, once whoever lowered it has said so. Before issue 359
    /// there was no way to say it and both emitters wrote zero, so a
    /// unit built that way ran one way in Rust and another in hardware
    /// with nothing said.
    #[test]
    fn a_registers_first_value_reaches_both_netlists() {
        let mut net = three_regs();
        net.init_reg("flag", 1);
        net.init_reg("count", 0x2a);
        net.init_reg("wide", 0xc0ff_ee11);
        let v = net.verilog();
        assert!(v.contains("reg flag = 1'h1;"), "the flag: {v}");
        assert!(v.contains("reg [7:0] count = 8'h2a;"), "the counter");
        assert!(v.contains("reg [31:0] wide = 32'hc0ffee11;"), "the word");
        let h = net.vhdl();
        assert!(h.contains("signal flag : std_logic := '1';"), "the flag");
        assert!(h.contains(":= \"00101010\";"), "the counter: {h}");
        assert!(
            h.contains(":= \"11000000111111111110111000010001\";"),
            "the word"
        );
    }

    /// And a reset puts it back there, as the runtime's does, rather
    /// than at zero; one left unsaid still goes back to zero, in the
    /// same words as before (issue 728).
    #[test]
    fn a_reset_puts_a_register_back_at_its_first_value() {
        let mut net = three_regs();
        for r in ["flag", "count", "wide"] {
            net.procs.push(Process {
                clock: "clk",
                falling: false,
                body: vec![Stmt::Drive(
                    Target::Name(r.to_string()),
                    Expr::Name(r.to_string()),
                )],
            });
        }
        net.init_reg("flag", 1);
        net.init_reg("count", 0x2a);
        let v = net.verilog();
        assert!(v.contains("flag <= 1'h1;"), "the flag: {v}");
        assert!(v.contains("count <= 8'h2a;"), "the counter: {v}");
        assert!(v.contains("wide <= 0;"), "the word, unsaid: {v}");
        let h = net.vhdl();
        assert!(h.contains("flag <= '1';"), "the flag: {h}");
        assert!(h.contains("count <= \"00101010\";"), "the counter: {h}");
        assert!(h.contains("wide <= (others => '0');"), "the word: {h}");
    }

    /// A field the struct renamed is said under the name the netlist
    /// knows, as a memory's words already are.
    #[test]
    fn a_renamed_register_keeps_its_first_value() {
        let mut net = three_regs();
        net.init_reg("flag", 1);
        let net = net.renamed(&[("flag", "banner")]);
        assert_eq!(net.init_regs[0].0, "banner");
    }

    /// A parent holding a child on a clock called `slow`, in a field
    /// of the name given: what the two-clock example of issue 131
    /// wrote before it renamed the field to `ticker`.
    fn parent_with_child_named(field: &str) -> Lowered {
        let mut child = three_regs();
        child.name = "two_slow".to_string();
        child.procs.push(Process {
            clock: "slow",
            falling: false,
            body: Vec::new(),
        });
        let mut parent = three_regs();
        parent.name = "two".to_string();
        parent.procs.push(Process {
            clock: "clk",
            falling: false,
            body: Vec::new(),
        });
        parent.instances.push(Instance {
            name: field.to_string(),
            unit: child,
            conns: Vec::new(),
        });
        parent
    }

    /// A unit of units with a channel port, `bus_req`, joined to its
    /// child `n` times: what `#[lower]` cannot check for a field of a
    /// struct of ports it cannot see, and `checked` does (issue 483).
    fn parent_joining_a_channel(n: usize) -> Lowered {
        let mut parent = parent_with_child_named("ticker");
        parent.procs.clear();
        parent
            .ports
            .push(("bus_req".to_string(), Kind::Rx, 8, "clk"));
        parent.instances[0].conns = (0..n)
            .map(|k| (format!("in{k}"), "bus_req".to_string()))
            .collect();
        parent
    }

    #[test]
    #[should_panic(
        expected = "port `bus_req` of `two` is a channel joined to 2"
    )]
    fn a_channel_port_joined_twice_is_refused() {
        parent_joining_a_channel(2).checked();
    }

    #[test]
    #[should_panic(
        expected = "port `bus_req` of `two` is a channel joined to 0"
    )]
    fn a_channel_port_joined_nowhere_is_refused() {
        parent_joining_a_channel(0).checked();
    }

    #[test]
    fn a_channel_port_joined_once_is_accepted() {
        parent_joining_a_channel(1).checked();
    }

    /// A register named in `async_regs` is marked `ASYNC_REG` in both
    /// netlists, and the others are not (#884).
    #[test]
    fn an_async_reg_is_marked_in_both_netlists() {
        let mut net = three_regs();
        net.async_regs = vec!["count"];
        let net = net.checked();
        let v = net.verilog();
        assert!(
            v.contains("  (* ASYNC_REG = \"TRUE\" *) reg [7:0] count = 8'h0;"),
            "{v}"
        );
        assert_eq!(v.matches("ASYNC_REG").count(), 1, "{v}");
        let h = net.vhdl();
        assert!(h.contains("  attribute ASYNC_REG : string;\n"), "{h}");
        assert!(
            h.contains("  attribute ASYNC_REG of count : signal is \"TRUE\";"),
            "{h}"
        );
        assert_eq!(h.matches("attribute ASYNC_REG of").count(), 1, "{h}");
    }

    /// Without one, neither netlist declares the attribute at all.
    #[test]
    fn no_async_reg_declares_nothing() {
        let net = three_regs();
        assert!(!net.verilog().contains("ASYNC_REG"));
        assert!(!net.vhdl().contains("ASYNC_REG"));
    }

    #[test]
    #[should_panic(expected = "field `count` of `regs` carries #[async_reg] \
                               and is not a register")]
    fn an_async_reg_on_a_wire_is_refused() {
        let mut net = three_regs();
        net.fields[1].1 = Some(Kind::Wire);
        net.async_regs = vec!["count"];
        net.checked();
    }

    /// The case of issue 485: a module lowered as `shared`, which VHDL
    /// reserves. It was refused as it was written; now both netlists
    /// call it `shared_rw` (issue 497).
    #[test]
    fn a_module_named_with_a_reserved_word_is_escaped() {
        let mut net = three_regs();
        net.name = "shared".to_string();
        assert!(net.vhdl().contains("entity shared_rw is"), "VHDL");
        assert!(net.verilog().contains("module shared_rw("), "Verilog");
    }

    /// Both targets take the same name, whichever one reserves it, so
    /// one testbench binds either netlist. `always` is a word of
    /// Verilog alone and is escaped in the VHDL too; `Signal` is VHDL's
    /// `signal`, since VHDL ignores case; `buffer`, which the
    /// datasheets lower `Buffer` as, is VHDL's and escaped in Verilog.
    #[test]
    fn both_targets_escape_a_word_either_reserves() {
        let named = |n: &str| {
            let mut net = three_regs();
            net.name = n.to_string();
            net
        };
        assert!(named("always").vhdl().contains("entity always_rw is"));
        assert!(named("always").verilog().contains("module always_rw("));
        assert!(named("Signal").vhdl().contains("entity Signal_rw is"));
        assert!(named("buffer")
            .checked()
            .verilog()
            .contains("module buffer_rw("));
        // A name nothing reserves is written as it is.
        assert!(named("regs").verilog().contains("module regs("));
    }

    /// A child's module and its instance are escaped with the parent.
    #[test]
    fn a_child_named_with_a_reserved_word_is_escaped() {
        let mut parent = parent_with_child_named("next");
        parent.instances[0].unit.name = "out".to_string();
        let v = parent.verilog();
        assert!(v.contains("module out_rw("), "the child's module: {v}");
        assert!(v.contains("out_rw next_rw("), "the instance: {v}");
    }

    /// A child in a field whose name a target reserves: the derive
    /// escapes the field and the netlist the instance, and the two are
    /// one name, not a collision.
    #[test]
    fn a_child_in_a_reserved_field_is_one_name() {
        let mut parent = parent_with_child_named("shared");
        parent.fields.push(("shared_rw", None, 0, 0));
        let v = parent.verilog();
        assert!(v.contains("two_slow shared_rw("), "the instance: {v}");
    }

    /// A unit with a port and a register a target reserves, and a wire
    /// reading the port.
    fn reserved_port() -> Lowered {
        let mut net = three_regs();
        net.ports.push(("next".to_string(), Kind::In, 8, "clk"));
        net.ports.push(("out".to_string(), Kind::Out, 8, "clk"));
        net.wires.push((
            "sum".to_string(),
            Expr::bin("+", Expr::name("next"), Expr::name("count")),
        ));
        net.procs.push(Process {
            clock: "clk",
            falling: false,
            body: vec![Stmt::Drive(
                Target::Name("out".to_string()),
                Expr::name("sum"),
            )],
        });
        net
    }

    /// A port is escaped wherever the netlist names it, and the ports
    /// file gives its own name as its trace scope, so the testbench
    /// reads the run's `next` for the netlist's `next_rw`. A mismatch
    /// there would bind the port to nothing and say so late, or not at
    /// all (issue 462).
    #[test]
    fn a_port_is_escaped_and_read_from_the_trace_by_its_own_name() {
        let net = reserved_port();
        let v = net.verilog();
        assert!(v.contains("input [7:0] next_rw"), "the port: {v}");
        assert!(v.contains("output [7:0] out_rw"), "the port: {v}");
        assert!(v.contains("next_rw + count"), "the reference: {v}");
        assert!(!v.contains(" next "), "no bare `next`: {v}");
        let h = net.vhdl();
        assert!(h.contains("next_rw : in"), "the VHDL port: {h}");
        let p = net.ports_file();
        assert!(p.contains("next_rw in 8 next\n"), "the scope: {p}");
        assert!(p.contains("out_rw out 8 out\n"), "the scope: {p}");
    }

    /// `trace_as` takes the port's own name, and its scope wins over
    /// the one the escape would give.
    #[test]
    fn trace_as_names_an_escaped_port_by_its_own_name() {
        let mut net = reserved_port();
        net.trace_as("next", "offered");
        let p = net.ports_file();
        assert!(p.contains("next_rw in 8 offered\n"), "{p}");
    }

    /// An escaped name another name already has is the one case still
    /// refused, naming both.
    #[test]
    #[should_panic(expected = "`next` is a reserved word, and the name the \
                               netlist would give it, `next_rw`, is \
                               already taken in `regs`")]
    fn an_escaped_name_already_taken_is_refused() {
        let mut net = reserved_port();
        net.fields.push(("next_rw", Some(Kind::Reg), 8, 0));
        net.verilog();
    }

    /// A parent's wire joined to a child: what `tie(v)` lowers to.
    fn tied_to(kind: Kind) -> Lowered {
        let mut parent = parent_with_child_named("ticker");
        parent.instances[0].unit.ports.push((
            "step".to_string(),
            kind,
            8,
            "clk",
        ));
        parent.wires.push((
            "ticker_tie0".to_string(),
            lit(crate::types::U::<8>::from(3u8)),
        ));
        parent.instances[0].conns =
            vec![("step".to_string(), "ticker_tie0".to_string())];
        parent
    }

    /// A constant is joined to a child's input (issue 498).
    #[test]
    fn a_constant_is_joined_to_an_input() {
        let v = tied_to(Kind::In).checked().verilog();
        assert!(v.contains(".step(ticker_tie0)"), "the join: {v}");
        assert!(v.contains("assign ticker_tie0 = 8'b00000011;"), "{v}");
    }

    /// And to nothing else: an output would be driven twice.
    #[test]
    #[should_panic(expected = "`ticker_tie0` of `two` holds a constant, and \
                               is joined to the port `step` of `ticker`, \
                               which is not an input")]
    fn a_constant_joined_to_an_output_is_refused() {
        tied_to(Kind::Out).checked();
    }

    /// A channel between children joined to one port only: what a
    /// bundle made whole, `link::<B>()`, would leave if one side were
    /// never passed, which only `lowered` can count (issue 498).
    #[test]
    #[should_panic(expected = "the channel `link_aw` of `two` is joined to 1 \
                               ports")]
    fn a_channel_joined_to_one_port_is_refused() {
        let mut parent = parent_with_child_named("ticker");
        parent
            .nets
            .push(("link_aw".to_string(), Kind::Tx, 8, "clk"));
        parent.instances[0].unit.ports.push((
            "aw".to_string(),
            Kind::Tx,
            8,
            "clk",
        ));
        parent.instances[0].conns =
            vec![("aw".to_string(), "link_aw".to_string())];
        parent.checked();
    }

    /// A one-bit channel between children: its nets are `std_logic`, as
    /// the ports joined to them are, and the channel's `unsigned(0
    /// downto 0)` data ports take the bit by its element, which nvc
    /// otherwise refuses (issue 953). A wider one is joined whole.
    #[test]
    fn a_one_bit_channel_is_joined_by_its_element() {
        let one_bit = |w: usize| {
            let mut parent = parent_with_child_named("ticker");
            parent.nets.push(("tap".to_string(), Kind::Tx, w, "clk"));
            for (p, k) in [("tx", Kind::Tx), ("rx", Kind::Rx)] {
                parent.instances[0].unit.ports.push((
                    p.to_string(),
                    k,
                    w,
                    "clk",
                ));
                parent.instances[0]
                    .conns
                    .push((p.to_string(), "tap".to_string()));
            }
            parent.checked().vhdl()
        };
        let v = one_bit(1);
        assert!(
            v.contains("signal tap_tx_data, tap_rx_data : std_logic;"),
            "{v}"
        );
        assert!(v.contains("tx_data(0) => tap_tx_data,"), "{v}");
        assert!(v.contains("rx_data(0) => tap_rx_data,"), "{v}");
        let v = one_bit(2);
        assert!(v.contains("tx_data => tap_tx_data,"), "{v}");
        assert!(v.contains("rx_data => tap_rx_data,"), "{v}");
    }

    /// A unit that states things of its register: a check under a
    /// condition and a cover point, and a process with nothing else in
    /// it (issue 502).
    fn stating() -> Lowered {
        let mut net = three_regs();
        net.procs.push(Process {
            clock: "clk",
            falling: false,
            body: vec![
                Stmt::Check(
                    Checked::Cover,
                    Expr::bin("==", Expr::name("count"), Expr::Num(9)),
                    "nine".to_string(),
                ),
                Stmt::If(
                    vec![(
                        Expr::name("flag"),
                        vec![Stmt::Check(
                            Checked::Assert,
                            Expr::bin("<=", Expr::name("count"), Expr::Num(9)),
                            "a digit".to_string(),
                        )],
                    )],
                    Vec::new(),
                ),
            ],
        });
        net
    }

    /// In the Verilog, immediate assertions inside the clocked block,
    /// under their conditions, and only for a formal tool.
    #[test]
    fn a_statement_is_an_immediate_assertion_for_a_formal_tool() {
        let v = stating().verilog();
        assert!(v.contains("always @(posedge clk)"), "the block: {v}");
        assert!(v.contains("`ifdef FORMAL"), "only for a formal tool: {v}");
        assert!(v.contains("cover ((count == "), "the cover: {v}");
        let under = v.find("if (flag) begin").expect("the condition");
        let check = v.find("assert ((count <= ").expect("the check");
        assert!(under < check, "the check under its condition: {v}");
        assert!(v.contains("// a digit"), "the message: {v}");
    }

    /// In the VHDL, VHDL's own `assert`, which a simulator checks, and
    /// a cover point's report.
    #[test]
    fn a_statement_is_a_vhdl_assertion() {
        let h = stating().vhdl();
        assert!(h.contains("report \"a digit\" severity failure;"), "{h}");
        assert!(h.contains("report \"cover: nine\" severity note;"), "{h}");
    }

    /// A statement is stated only out of reset, in both languages,
    /// whether the netlist added the reset or the unit declared it
    /// (issue 633). Before, a unit with its own `rst` had its checks
    /// stated in the reset cycles too.
    #[test]
    fn a_statement_is_stated_only_out_of_reset() {
        let added = stating();
        let mut declared = stating();
        declared.ports.push(("rst".to_string(), Kind::In, 1, "clk"));
        assert!(added.adds_reset_port() && !declared.adds_reset_port());
        for net in [added, declared] {
            let v = net.verilog();
            assert!(v.contains("if (!rst) assert ((count <= "), "{v}");
            assert!(v.contains("if (!rst) cover ((count == "), "{v}");
            let h = net.vhdl();
            assert!(h.contains("assert rst = '1' or ("), "{h}");
            assert!(h.contains("if rst = '0' and ("), "{h}");
        }
    }

    /// A helper reached through its own lowering binds a value it
    /// reads twice to a wire of the unit, named for the helper and the
    /// value and numbered, and leaves one read once, or a name, alone
    /// (issue 504).
    #[test]
    fn a_value_a_helper_reads_twice_is_a_wire_of_the_unit() {
        inlined_begin();
        let sum = || Expr::bin("+", Expr::name("a"), Expr::name("b"));
        assert!(matches!(inline_bind("f_x", sum(), 1), Expr::Bin(..)));
        assert!(matches!(
            inline_bind("f_y", Expr::name("a"), 3),
            Expr::Name(n) if n == "a"
        ));
        let w = inline_bind("f_x", sum(), 2);
        assert!(matches!(&w, Expr::Name(n) if n == "f_x_i1"), "{w:?}");
        let wires = inlined_end();
        assert_eq!(wires.len(), 1);
        assert_eq!(wires[0].0, "f_x_i1");
    }

    /// A child lowered inside a parent opens a frame of its own, so the
    /// wires its helpers ask for are its and not the parent's.
    #[test]
    fn a_child_keeps_its_own_helper_wires() {
        let sum = || Expr::bin("+", Expr::name("a"), Expr::name("b"));
        inlined_begin();
        let _ = inline_bind("p", sum(), 2);
        inlined_begin();
        let _ = inline_bind("c", sum(), 2);
        let child = inlined_end();
        let parent = inlined_end();
        assert_eq!(
            child.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["c_i1"]
        );
        assert_eq!(
            parent.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["p_i1"]
        );
    }

    /// The instance and the clock pin would take one name, which
    /// Verilator refuses and nvc does not; the lowering refuses it
    /// first, and says which to rename (issue 367).
    #[test]
    // `expected` is matched as a substring, so it is the message's start,
    // which names the field and its child, and the rest is left out to
    // stay within the listing's frame in //docs:runtime (issue 1061).
    #[should_panic(expected = "field `slow` of `two` holds a child")]
    fn a_child_named_after_its_clock_is_refused() {
        parent_with_child_named("slow").checked();
    }

    /// Under another name the same design lowers, and the clock
    /// reaches the child through the parent's port of its name.
    #[test]
    fn a_child_named_otherwise_takes_the_clock_through_the_parent() {
        let v = parent_with_child_named("ticker").checked().verilog();
        assert!(v.contains("input slow"), "the parent takes the clock: {v}");
        assert!(v.contains("two_slow ticker("), "the instance: {v}");
        assert!(v.contains(".slow(slow)"), "and passes the clock on: {v}");
    }

    /// A register named after a clock is the same clash, and the
    /// message says what the name is.
    #[test]
    #[should_panic(expected = "field `clk` of `regs` is a register")]
    fn a_register_named_after_the_clock_is_refused() {
        let mut net = three_regs();
        net.fields.push(("clk", Some(Kind::Reg), 1, 0));
        net.procs.push(Process {
            clock: "clk",
            falling: false,
            body: Vec::new(),
        });
        net.checked();
    }

    fn chained(arms: &[(&str, &[&str])], els: &[&str]) -> Vec<String> {
        let mut seq = Vec::new();
        let arms = arms
            .iter()
            .map(|(c, b)| {
                (c.to_string(), b.iter().map(|s| s.to_string()).collect())
            })
            .collect();
        let els = els.iter().map(|s| s.to_string()).collect();
        let open = |i: usize, c: &str| {
            if i == 0 {
                format!("if ({c}) begin")
            } else {
                format!("end else if ({c}) begin")
            }
        };
        chain(&mut seq, arms, els, &open, "end else begin", "end");
        seq
    }

    #[test]
    fn a_chain_with_nothing_in_it_is_not_written() {
        assert!(chained(&[("a", &[])], &[]).is_empty());
        assert!(chained(&[("a", &[]), ("b", &[])], &[]).is_empty());
    }

    #[test]
    fn the_empty_arms_at_the_end_go() {
        assert_eq!(
            chained(&[("a", &["x <= 1;"]), ("b", &[])], &[]),
            ["if (a) begin", "x <= 1;", "end"]
        );
    }

    #[test]
    fn an_empty_arm_before_a_kept_one_stays() {
        assert_eq!(
            chained(&[("a", &[]), ("b", &["x <= 1;"])], &[]),
            ["if (a) begin", "end else if (b) begin", "x <= 1;", "end"]
        );
        assert_eq!(
            chained(&[("a", &[])], &["x <= 1;"]),
            ["if (a) begin", "end else begin", "x <= 1;", "end"]
        );
    }

    /// A unit of units named `name` whose children are joined by a
    /// channel of eight bits, `link`: a netlist that needs the channel
    /// module.
    fn joined_by_a_channel(name: &str) -> Lowered {
        let mut net = parent_with_child_named("ticker");
        net.name = name.to_string();
        // A child's module is named for its parent and its field, as
        // the lowering names it.
        net.instances[0].unit.name = format!("{name}_ticker");
        net.nets.push(("link".to_string(), Kind::Tx, 8, "clk"));
        net
    }

    /// The names a netlist defines: `module NAME` in Verilog, `entity
    /// NAME is` in VHDL, each as many times as it is written.
    fn defined(text: &str, verilog: bool) -> Vec<String> {
        text.lines()
            .filter_map(|l| {
                if verilog {
                    l.strip_prefix("module ")
                } else {
                    l.strip_prefix("entity ")?.strip_suffix(" is")
                }
            })
            .map(|l| {
                l.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    /// Two netlists that each join children by a channel, in one
    /// design: the flagship's board and its scanout (issue 979). Every
    /// module and every entity is defined once across the two, so the
    /// two files go into one synthesis, and into one file, as
    /// `write_netlists_from_env` writes several units, without a
    /// definition repeated.
    #[test]
    fn two_netlists_with_channels_define_nothing_twice() {
        let (a, b) =
            (joined_by_a_channel("board"), joined_by_a_channel("scan"));
        for verilog in [true, false] {
            let text =
                |l: &Lowered| if verilog { l.verilog() } else { l.vhdl() };
            let both = format!("{}\n{}", text(&a), text(&b));
            let mut names = defined(&both, verilog);
            let all = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), all, "a name defined twice in:\n{both}");
        }
    }

    /// Each netlist defines the channel module it instantiates, under
    /// a name of its own, and a netlist with no channel defines none.
    #[test]
    fn a_netlist_defines_the_channel_it_instantiates() {
        let net = joined_by_a_channel("board");
        let v = net.verilog();
        assert!(
            defined(&v, true).contains(&"board_txhdl_chan".to_string()),
            "{v}"
        );
        assert!(v.contains("board_txhdl_chan #(.W(8)) link_chan("), "{v}");
        let h = net.vhdl();
        assert!(
            defined(&h, false).contains(&"board_txhdl_chan".to_string()),
            "{h}"
        );
        assert!(
            h.contains("link_chan : entity work.board_txhdl_chan"),
            "{h}"
        );
        assert!(h.contains("architecture rtl of board_txhdl_chan is"), "{h}");
        let plain = three_regs();
        assert!(!plain.verilog().contains("txhdl_chan"));
        assert!(!plain.vhdl().contains("txhdl_chan"));
    }

    /// A child whose module would take the channel module's name, in
    /// any case, since VHDL reads names without it, is refused rather
    /// than defined twice.
    #[test]
    #[should_panic(expected = "names its channel module `board_txhdl_chan`")]
    fn a_unit_named_as_the_channel_is_refused() {
        let mut net = joined_by_a_channel("board");
        net.instances[0].unit.name = "Board_TXHDL_Chan".to_string();
        let _ = net.vhdl();
    }

    /// The same name on a netlist with no channel is no clash, since
    /// that netlist defines no channel module.
    #[test]
    fn a_unit_so_named_without_a_channel_is_accepted() {
        let mut net = parent_with_child_named("ticker");
        net.name = "board".to_string();
        net.instances[0].unit.name = "board_txhdl_chan".to_string();
        let _ = net.verilog();
        let _ = net.vhdl();
    }
}
