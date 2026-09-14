// SPDX-License-Identifier: Apache-2.0
//! A compound value in a waveform. A packet is a struct of fields, and
//! a transaction; in the trace it is one bit vector, and one signal per
//! field beside it, so a viewer shows `src`, `dst`, `kind` and `len`
//! as themselves. The enum is its variant's index. The file is FST when
//! `TXHDL_FST` names one, as the document build does.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, join2, now, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use txhdl::types::U;
use txhdl::{Trace, Transaction, Value};

#[derive(Value, Clone, Copy, Default, PartialEq, Debug)]
pub enum Kind {
    #[default]
    Data,
    Ack,
    Nack,
}

#[derive(Transaction, Value, Clone, Copy, Default, Debug)]
pub struct Packet {
    pub src: U<4>,
    pub dst: U<4>,
    pub kind: Kind,
    pub len: U<8>,
}

/// Sends a packet every cycle, walking the addresses and the kinds.
#[derive(Default)]
pub struct Talker {
    pub n: Reg<U<8>>,
}

impl Unit<(), Tx<Packet>> for Talker {
    async fn run(&mut self, _i: (), out: Tx<Packet>) {
        loop {
            DefaultClock::rising().await;
            let n = self.n.get();
            if out.ready().to_bool() {
                let k = match n.raw() % 3 {
                    0 => Kind::Data,
                    1 => Kind::Ack,
                    _ => Kind::Nack,
                };
                out.send(Packet {
                    src: U::from((n.raw() % 16) as u8),
                    dst: U::from(((n.raw() + 5) % 16) as u8),
                    kind: k,
                    len: U::from((n.raw() * 3) as u8),
                });
                self.n.set(n + 1);
            }
        }
    }
}

#[derive(Trace, Default)]
pub struct Listener {
    pub last: Reg<Packet>,
}

impl Unit<Rx<Packet>, ()> for Listener {
    async fn run(&mut self, inp: Rx<Packet>, _o: ()) {
        loop {
            let p = inp.wait().await;
            self.last.set(p);
            println!(
                "t={:>2} {:?} from {} to {} len {}",
                now(),
                p.kind,
                p.src.raw(),
                p.dst.raw(),
                p.len.raw()
            );
        }
    }
}

fn main() {
    let (tx, rx) = chan::<Packet, _>();
    let mut talker = Talker::default();
    let mut listener = Listener::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("link", &rx);
        w.add("listener", &listener);
        w.start();
    }
    let mut sim = txhdl::comp::Running::new(join2(
        talker.run((), tx),
        listener.run(rx, ()),
    ));
    for _ in 0..6 {
        sim.cycle();
    }
    stop();
}
