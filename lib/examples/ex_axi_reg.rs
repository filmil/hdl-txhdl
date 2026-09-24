// SPDX-License-Identifier: Apache-2.0
//! The simplest AXI peripheral: one register of one word, written as
//! hardware. It holds no state but the word itself. A read is answered
//! in the cycle it is taken, and a write is taken in the cycle its
//! request and its beat are both there, so nothing waits in between.
//! It serves single-beat bursts of whole words, and looks at no
//! address, no length and no strobe. A host client writes two words
//! and reads each back, printing the time each answer came, and the
//! build checks the register's netlist against the run.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, Clock, DefaultClock, Reg, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{
    axi_to_unit, Answer, AxiHost, AxiPer, HostLink, PerPort, Rd, Resp, Wr, R,
};

// begin{register}
/// One word, readable and writable over AXI.
#[derive(Trace, Default)]
pub struct Register {
    pub word: Reg<U<32>>,
}

#[lower]
impl Unit for Register {
    async fn run(&mut self, bus: PerPort<32, 32, 4, 2>, _out: ()) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let there = bus.req.peek().is_some();
            // A read needs room for its beat; a write needs its beat
            // and room for its response.
            let read = there & q.read & bus.r.ready();
            let write =
                there & !q.read & bus.w.peek().is_some() & bus.ans.ready();
            let _ = bus.req.recv_if(read | write);
            let beat = bus.w.head();
            let _ = bus.w.recv_if(write);
            with!(self <= { write ? word: beat.data });
            if read.to_bool() {
                bus.r.send(R {
                    id: q.id,
                    data: self.word.get(),
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if write.to_bool() {
                bus.ans.send(Answer {
                    id: q.id,
                    resp: Resp::Okay,
                });
            }
        }
    }
}
// end{register}

// begin{main}
fn main() {
    let HostLink {
        host,
        per_client,
        host_in,
        host_out,
        per_in,
        per_out,
    } = axi_to_unit::<32, 32, 4, 2, 4>();
    let bus: PerPort<32, 32, 4, 2> = per_client.into();
    let mut h = AxiHost::<32, 32, 4, 2, 4>::default();
    let mut p = AxiPer::<32, 32, 4, 2>::default();
    let mut reg = Register::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("bus_req", &bus.req);
        w.add("bus_w", &bus.w);
        w.add("bus_ans", &bus.ans);
        w.add("bus_r", &bus.r);
        w.add("reg", &reg);
        w.start();
    }
    let client = async move {
        for v in [0x1234_5678u32, 0xcafe_f00d] {
            let a = host.write(Wr::at(0u32), &[U::from(v)]).await;
            assert_eq!(a.done().await.resp, Resp::Okay);
            println!("{:3}  wrote {v:#010x}", now());
            let r = host.read(Rd::at(0u32, 1)).await.done().await;
            println!("{:3}  read  {:#010x}", now(), r.data[0].raw());
            assert_eq!(r.data[0].raw(), v as u128);
        }
    };
    let mut sim = Running::new(join2(
        join2(h.run(host_in, host_out), p.run(per_in, per_out)),
        join2(reg.run(bus, ()), client),
    ));
    println!("  t  answer");
    for _ in 0..40 {
        sim.cycle();
    }
    stop();
    let net = Register::lowered("axi_reg");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
// end{main}
