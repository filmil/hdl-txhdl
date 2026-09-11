// Probe 5. A unit's `run` joins one async fn per process. The processes
// take `&self` and share state through the library's `Reg`.
use txhdl::comp::{join2, Module, Reg};
use txhdl::types::U;

pub struct DualPort { pub cells: Reg<U<32>>, pub hits_a: Reg<U<32>>, pub hits_b: Reg<U<32>> }

impl DualPort {
    async fn port_a(&self) {
        self.hits_a.set(self.hits_a.get().wrapping_add(U::new(1)));
        self.cells.set(self.cells.get().wrapping_add(U::new(1)));
    }
    async fn port_b(&self) { self.hits_b.set(self.hits_b.get().wrapping_add(U::new(1))) }
}

impl Module<(), ()> for DualPort {
    async fn run(&mut self, _i: (), _o: ()) { join2(self.port_a(), self.port_b()).await }
}
