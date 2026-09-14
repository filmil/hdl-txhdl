// SPDX-License-Identifier: Apache-2.0
//! A memory, lowered: a scratchpad of sixteen bytes with a write port
//! under an enable and a read port that is a wire. `Mem::at` names a
//! word as a drive's target, so `with!` writes it as it writes a
//! register; `read` is a plain read, and lowers to an index. The
//! memory is untraced, so the waveform shows the ports, and the
//! lowering is checked against them.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{signal, Clock, DefaultClock, In, Mem, Out, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

#[derive(Trace, Default)]
pub struct Scratch {
    pub m: Mem<U<8>, 16>,
}

#[lower]
impl Unit for Scratch {
    async fn run(
        &mut self,
        (we, waddr, wdata, raddr): (In<Bit>, In<U<4>>, In<U<8>>, In<U<4>>),
        q: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let (we, wa) = (we.get(), waddr.get());
            let (wd, ra) = (wdata.get(), raddr.get());
            when!(we => self { m.at(wa): wd });
            q.set(self.m.read(ra));
        }
    }
}

fn main() {
    let (we_out, we) = signal::<Bit, DefaultClock>();
    let (waddr_out, waddr) = signal::<U<4>, DefaultClock>();
    let (wdata_out, wdata) = signal::<U<8>, DefaultClock>();
    let (raddr_out, raddr) = signal::<U<4>, DefaultClock>();
    let (q_out, q) = signal::<U<8>, DefaultClock>();
    let mut pad = Scratch::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("we", &we);
        w.add("waddr", &waddr);
        w.add("wdata", &wdata);
        w.add("raddr", &raddr);
        w.add("q", &q);
        w.start();
    }
    let mut sim = Running::new(pad.run((we, waddr, wdata, raddr), q_out));
    // Write four words, read them back, then read one while writing
    // it: the read sees the old word, since the write lands at the edge.
    let steps: [(u8, u8, u8, u8); 10] = [
        (1, 0, 0x11, 0),
        (1, 1, 0x22, 0),
        (1, 2, 0x33, 1),
        (1, 3, 0x44, 2),
        (0, 0, 0, 3),
        (0, 0, 0, 0),
        (0, 0, 0, 1),
        (1, 1, 0x55, 1),
        (0, 0, 0, 1),
        (0, 0, 0, 2),
    ];
    println!("  t we wa   wd ra    q");
    for (en, wa, wd, ra) in steps {
        we_out.set(en == 1);
        waddr_out.set(wa);
        wdata_out.set(wd);
        raddr_out.set(ra);
        sim.cycle();
        println!(
            "{:>3}  {en} {wa:>2} {wd:#04x} {ra:>2} {:#04x}",
            txhdl::comp::now(),
            q.get().raw()
        );
    }
    sim.cycle();
    stop();
    txhdl::netlist::write_vhdl_from_env(&Scratch::lowered("scratch"));
    print!("\n{}", Scratch::verilog("scratch"));
}
