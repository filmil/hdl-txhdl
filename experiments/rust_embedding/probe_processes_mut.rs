// Probe 5b. The naive shape: two processes that each take `&mut self`,
// joined. Expected to fail. Kept because the failure is the reason the
// unit's state uses interior mutability in probe 5.
pub struct DualPort { pub hits_a: u32, pub hits_b: u32 }

impl DualPort {
    async fn port_a(&mut self) { self.hits_a += 1; }
    async fn port_b(&mut self) { self.hits_b += 1; }

    pub async fn run(&mut self) {
        // Two mutable borrows of `self`, alive at once.
        let a = self.port_a();
        let b = self.port_b();
        let _ = (a, b);
    }
}
