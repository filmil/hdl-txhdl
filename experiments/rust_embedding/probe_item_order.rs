// Probe 7. Can an `impl` precede the `struct` it is for?
//
// Items in a Rust module are not order dependent, unlike statements in a
// block. If that holds, `impl Transaction for Foo` may be written above
// `struct Foo`, which would put the kind before the data and read the way
// the old `transaction Foo { .. }` did.

pub trait Transaction {}
pub trait Bus {}

// The impl, before the struct exists in reading order.
impl Transaction for MacRequest {}

pub struct MacRequest {
    pub addr: u32,
    pub count: u8,
}

// The same across a trait with items, not just a marker.
impl Named for MacResponse {
    fn name(&self) -> &'static str { "MacResponse" }
}

pub trait Named {
    fn name(&self) -> &'static str;
}

pub struct MacResponse {
    pub acc: u64,
}

// And where the impl mentions a type declared later still.
impl Bus for MemBus {}

pub struct MemBus {
    pub req: MacRequest,
}

// A generic impl ahead of both the trait bound's type and the struct.
impl<T: Transaction> Holder<T> {
    pub fn new(v: T) -> Self { Holder { v } }
}

pub struct Holder<T: Transaction> {
    pub v: T,
}

pub fn build() -> Holder<MacRequest> {
    Holder::new(MacRequest { addr: 0, count: 1 })
}
