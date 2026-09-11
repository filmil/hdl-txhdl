// Probe 5. A unit's `run` spins up one async fn per process and joins
// them, so that parallelism is stated rather than implied.
//
// The question underneath is whether two concurrent processes can share
// the unit's state. Hardware processes do: they drive registers. Rust
// forbids two `&mut self` borrows at once. Both shapes are below.

pub mod comp {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};

    /// Run two processes concurrently. A unit does not terminate, so this
    /// completes only if both processes do.
    pub struct Join2<A, B> { a: A, b: B, da: bool, db: bool }

    pub fn join2<A: Future<Output = ()>, B: Future<Output = ()>>(a: A, b: B) -> Join2<A, B> {
        Join2 { a, b, da: false, db: false }
    }

    impl<A: Future<Output = ()>, B: Future<Output = ()>> Future for Join2<A, B> {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            // Safety: no field is moved out, and the future is pinned.
            let this = unsafe { self.get_unchecked_mut() };
            if !this.da {
                let a = unsafe { Pin::new_unchecked(&mut this.a) };
                if a.poll(cx).is_ready() { this.da = true; }
            }
            if !this.db {
                let b = unsafe { Pin::new_unchecked(&mut this.b) };
                if b.poll(cx).is_ready() { this.db = true; }
            }
            if this.da && this.db { Poll::Ready(()) } else { Poll::Pending }
        }
    }

    #[allow(async_fn_in_trait)]
    pub trait Module<In, Out> {
        async fn run(&mut self, inputs: In, outputs: Out);
    }
}

use comp::{join2, Module};

/// A register. Interior mutability, so two processes may both hold a
/// shared reference and still drive it.
pub struct Reg<T: Copy> {
    v: core::cell::Cell<T>,
}

impl<T: Copy> Reg<T> {
    pub const fn new(v: T) -> Self { Self { v: core::cell::Cell::new(v) } }
    pub fn get(&self) -> T { self.v.get() }
    pub fn set(&self, x: T) { self.v.set(x) }
}

/// A unit with two ports and shared state, the dual-port memory that
/// TxHDL could not decide how to describe.
pub struct DualPort {
    pub cells: Reg<u32>,
    pub hits_a: Reg<u32>,
    pub hits_b: Reg<u32>,
}

pub struct Ins;
pub struct Outs;

/// The two processes take `&self`, not `&mut self`. They share the unit
/// through the registers, which is what hardware does, and the borrow
/// checker allows it because the mutation is inside the cell.
impl DualPort {
    async fn port_a(&self) {
        self.hits_a.set(self.hits_a.get() + 1);
        self.cells.set(self.cells.get() + 1);
    }

    async fn port_b(&self) {
        self.hits_b.set(self.hits_b.get() + 1);
    }
}

impl Module<Ins, Outs> for DualPort {
    /// Synthesis calls this. It starts one async fn per process and joins
    /// them, which is what states that the processes run in parallel.
    async fn run(&mut self, _inputs: Ins, _outputs: Outs) {
        join2(self.port_a(), self.port_b()).await;
    }
}

pub fn build() -> DualPort {
    DualPort { cells: Reg::new(0), hits_a: Reg::new(0), hits_b: Reg::new(0) }
}
