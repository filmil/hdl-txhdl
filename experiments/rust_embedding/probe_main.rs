// Probe 13. A design is a program: `fn main()` reaches the top unit
// through nothing but a configuration. Against the library's `Config`
// and `elaborate`.
use txhdl::comp::{elaborate, Config, Unit, Reg, rising, DefaultClock};
use txhdl::types::U;

#[derive(Default)]
pub struct Top<const TAPS: usize> { pub acc: Reg<U<64>> }

impl<const TAPS: usize> Unit<(), ()> for Top<TAPS> {
    async fn run(&mut self, _i: (), _o: ()) {
        loop { for i in 0..TAPS { rising::<DefaultClock>().await; let a = self.acc.get(); self.acc.set(a.wrapping_add(U::new(i as u128 * 2))) } }
    }
}

pub struct Fpga;
impl Config for Fpga {
    type Top = Top<8>;
    const NAME: &'static str = "fpga";
}

pub struct Asic;
impl Config for Asic {
    type Top = Top<64>;
    const NAME: &'static str = "asic";
}

fn report<C: Config>() { let _ = elaborate::<C>(); println!("elaborated config '{}'", C::NAME) }

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("asic") => report::<Asic>(),
        _ => report::<Fpga>(),
    }
}
