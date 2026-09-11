// Probe 11b. A run-time *width*. Expected to fail: the width is the
// type, and a type cannot depend on a value that only exists at run time.
pub struct U<const N: usize>(pub u128);

impl<const N: usize> U<N> {
    pub fn slice_var(&self, _lo: usize, len: usize) -> U<len> {
        U(0)
    }
}
