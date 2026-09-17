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
use txhdl::comp::{
    join2, now, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{
    axi_to_unit, Answer, AxiHost, AxiPer, HostLink, PerReq, Rd, Resp, Wr, R, W,
};

// begin{register}
/// One word, readable and writable over AXI.
#[derive(Trace, Default)]
pub struct Register {
    pub word: Reg<U<32>>,
}

#[lower]
impl Unit for Register {
    async fn run(
        &mut self,
        (req, wd): (Rx<PerReq<32, 2>>, Rx<W<32, 4>>),
        (ans, rb): (Tx<Answer<2>>, Tx<R<32, 2>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let q = req.head();
            let there = req.peek().is_some();
            // A read needs room for its beat; a write needs its beat
            // and room for its response.
            let read = there & q.read & rb.ready();
            let write = there & !q.read & wd.peek().is_some() & ans.ready();
            let _ = req.recv_if(read | write);
            let beat = wd.head();
            let _ = wd.recv_if(write);
            with!(self <= { write ? word: beat.data });
            if read.to_bool() {
                rb.send(R {
                    id: q.id,
                    data: self.word.get(),
                    resp: Resp::Okay,
                    last: Bit::One,
                });
            }
            if write.to_bool() {
                ans.send(Answer {
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
    let (req, wd, ans, rb) = per_client;
    let mut h = AxiHost::<32, 32, 4, 2, 4>::default();
    let mut p = AxiPer::<32, 32, 4, 2>::default();
    let mut reg = Register::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("req", &req);
        w.add("wd", &wd);
        w.add("ans", &ans);
        w.add("rb", &rb);
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
        join2(reg.run((req, wd), (ans, rb)), client),
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
