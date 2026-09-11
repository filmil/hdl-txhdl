// Probe 10. `if` on a signal, as a macro.
//
// `if` takes a bool and cannot be overloaded, so control flow on a signal
// needs another spelling.
//
// Not `if_!`. The trailing underscore says the real name was taken, and
// the name it is reaching for is the wrong one anyway: `if` branches, and
// this does not. Both arms exist in the hardware at once and a
// multiplexer picks between them, so a reader who brings Rust's `if`
// semantics to it is misled about the thing that matters most.
//
// `when!` is the name Chisel and SpinalHDL use for exactly this, so the
// people who will read it already know what it means, and it claims
// nothing it does not do.
//
// The question underneath is how far `macro_rules!` gets. An expression
// mux is easy. A statement form that predicates whatever is written
// inside it is what real logic needs, and that is where the limit shows.

use core::cell::Cell;

/// A signal: a node, not a value. It has no value while this program
/// runs, which is the whole reason `if` cannot take it.
#[derive(Clone, Copy)]
pub struct Sig(pub u32);

/// A register, so a predicated write has somewhere to land.
pub struct Reg(Cell<u32>);

impl Reg {
    pub const fn new(v: u32) -> Self { Reg(Cell::new(v)) }
    pub fn get(&self) -> Sig { Sig(self.0.get()) }
    /// A predicated write. The predicate is a signal, so this is a
    /// multiplexer in front of the register's enable, not a branch.
    pub fn set_if(&self, pred: Sig, v: Sig) {
        self.0.set(if pred.0 != 0 { v.0 } else { self.0.get() });
    }
    pub fn set(&self, v: Sig) { self.0.set(v.0) }
}

/// The multiplexer every form below reduces to.
pub fn mux(c: Sig, a: Sig, b: Sig) -> Sig {
    Sig(if c.0 != 0 { a.0 } else { b.0 })
}

pub fn not(a: Sig) -> Sig { Sig((a.0 == 0) as u32) }

// --- the expression form ---------------------------------------------

#[macro_export]
macro_rules! when_expr {
    ($c:expr, $t:expr, $e:expr) => {
        $crate::mux($c, $t, $e)
    };
}

// --- the statement form ----------------------------------------------
//
// Restricted on purpose: it predicates a list of register writes and
// nothing else. That restriction is the finding.

#[macro_export]
macro_rules! when {
    ($c:expr => { $($rt:expr => $vt:expr);* $(;)? }
             else { $($re:expr => $ve:expr);* $(;)? }) => {{
        let __c = $c;
        $( $rt.set_if(__c, $vt); )*
        $( $re.set_if($crate::not(__c), $ve); )*
    }};
}

// --- both, used ------------------------------------------------------

pub fn pick(c: Sig, a: Sig, b: Sig) -> Sig {
    when_expr!(c, a, b)
}

/// Nested, to check the expression form composes. In practice `mux`
/// itself is what a design writes; the macro exists for symmetry.
pub fn pick3(c1: Sig, c2: Sig, a: Sig, b: Sig, d: Sig) -> Sig {
    when_expr!(c1, when_expr!(c2, a, b), d)
}

pub struct Unit {
    pub count: Reg,
    pub flag: Reg,
}

impl Unit {
    pub fn step(&self, enable: Sig, reset: Sig) {
        // Reads as an if, lowers to two predicated writes.
        when!(enable => {
            self.count => Sig(self.count.get().0 + 1);
            self.flag  => Sig(1)
        } else {
            self.count => Sig(0);
            self.flag  => Sig(0)
        });

        // And the expression form in the same body. A plain function
        // reads better than a macro here, because there is nothing to
        // predicate: `mux` is the operation.
        let chosen = mux(reset, Sig(0), self.count.get());
        self.count.set(chosen);
    }
}

pub fn build() -> Unit {
    Unit { count: Reg::new(0), flag: Reg::new(0) }
}
