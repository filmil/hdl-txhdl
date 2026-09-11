// Probe 13. A design is a program: `fn main()` reaches the top unit
// through nothing but a configuration. Against the library's `Config`
// and `elaborate`.
use txhdl::comp::{elaborate, Config, Module, Reg};
use txhdl::types::U;

pub struct Top<const TAPS: usize> { pub acc: Reg<U<64>> }

impl<const TAPS: usize> Module<(), ()> for Top<TAPS> {
    async fn run(&mut self, _i: (), _o: ()) {
        for i in 0..TAPS { self.acc.set(self.acc.get().wrapping_add(U::new(i as u128 * 2))) }
    }
}

pub struct Fpga;
impl Config for Fpga {
    type Top = Top<8>;
    const NAME: &'static str = "fpga";
    fn top() -> Self::Top { Top { acc: Reg::new(U::new(0)) } }
}

pub struct Asic;
impl Config for Asic {
    type Top = Top<64>;
    const NAME: &'static str = "asic";
    fn top() -> Self::Top { Top { acc: Reg::new(U::new(0)) } }
}

fn report<C: Config>() { let _ = elaborate::<C>(); println!("elaborated config '{}'", C::NAME) }

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("asic") => report::<Asic>(),
        _ => report::<Fpga>(),
    }
}
