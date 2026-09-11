// Probe 2. Is `async fn` in a trait usable on stable, and may inputs and
// outputs be type parameters so one struct implements `Module` twice?
// Against the library's `Module`.
use txhdl::comp::{Module, Reg, rising, DefaultClock};
use txhdl::types::U;

pub struct MacUnit { pub acc: Reg<U<64>> }
pub struct NarrowIn(pub U<32>);
pub struct WideIn(pub U<64>);
pub struct AccOut;

impl Module<NarrowIn, AccOut> for MacUnit {
    async fn run(&mut self, inputs: NarrowIn, _o: AccOut) {
        // Non-synthesisable, and allowed: it observes and drives nothing.
        assert!(inputs.0.raw() != u32::MAX as u128, "sentinel is not a sample");
        loop {
            rising::<DefaultClock>().await;
            let acc = self.acc.get();   // the wait for the edge
            self.acc.set(acc.wrapping_add(inputs.0.resize::<64>()));
        }
    }
}

// The same struct, a different shape. This is what type parameters buy.
impl Module<WideIn, AccOut> for MacUnit {
    async fn run(&mut self, inputs: WideIn, _o: AccOut) {
        loop {
            rising::<DefaultClock>().await;
            let acc = self.acc.get();
            self.acc.set(acc.wrapping_add(inputs.0));
        }
    }
}
