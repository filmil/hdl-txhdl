// SPDX-License-Identifier: Apache-2.0
// Probe 12. A unit with submodules, against the library. Children are
// disjoint fields, so both may be borrowed mutably at once; the wire
// between them is handed to each as its `run` argument.
use txhdl::comp::{join2, join_all, signal, DefaultClock, In, Unit, Out, Reg, rising};
use txhdl::types::U;

pub struct Producer { pub n: Reg<U<32>> }
pub struct Consumer { pub total: Reg<U<32>> }
pub struct Pe { pub acc: Reg<U<32>> }

impl Unit<(), Out<U<32>>> for Producer {
    async fn run(&mut self, _i: (), out: Out<U<32>>) {
        loop { rising::<DefaultClock>().await; let n = self.n.get(); out.set(n); self.n.set(n.wrapping_add(U::new(1))) }
    }
}
impl Unit<In<U<32>>, ()> for Consumer {
    async fn run(&mut self, inp: In<U<32>>, _o: ()) {
        loop { rising::<DefaultClock>().await; let t = self.total.get(); self.total.set(t.wrapping_add(inp.get())) }
    }
}
impl Unit<U<32>, ()> for Pe {
    async fn run(&mut self, i: U<32>, _o: ()) {
        loop { rising::<DefaultClock>().await; let a = self.acc.get(); self.acc.set(a.wrapping_add(i)) }
    }
}

pub struct Top { pub producer: Producer, pub consumer: Consumer, pub pes: [Pe; 4] }

impl Unit<(), ()> for Top {
    async fn run(&mut self, _i: (), _o: ()) {
        let (tx, rx) = signal::<U<32>, DefaultClock>();
        join2(
            join2(self.producer.run((), tx), self.consumer.run(rx, ())),
            join_all(self.pes.iter_mut().enumerate().map(|(i, pe)| pe.run(U::new(i as u128), ()))),
        ).await;
    }
}
