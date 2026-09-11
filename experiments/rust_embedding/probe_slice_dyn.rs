// Probe 11b. A run-time width. Expected to fail: the width is the type.
use txhdl::types::U;

pub fn slice_var<const N: usize>(w: U<N>, _lo: usize, len: usize) -> U<len> {
    U::new(0)
}
