// SPDX-License-Identifier: Apache-2.0
//! An AXI link reaching a Wishbone peripheral. A host client writes
//! three words and reads them back; the link's peripheral end is the
//! bridge, `AxiWb`, written as hardware; and behind the bridge's
//! Wishbone lines is a memory that stalls while it calibrates and
//! takes two cycles to answer each request. The run prints the lines
//! each cycle a request is on them, and writes the bridge's trace and
//! netlists, so the build checks the netlist against this run.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{
    axi_to_unit, AxiHost, AxiPer, HostLink, Rd, Resp, Wr,
};
use txhdl_parts::bus::wb::sim::WbMem;
use txhdl_parts::bus::wb::AxiWb;

/// The word address: twenty-eight bits.
type Bridge = AxiWb<32, 2, 28>;

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
    let (cyc_o, cyc) = signal::<Bit, DefaultClock>();
    let (stb_o, stb) = signal::<Bit, DefaultClock>();
    let (we_o, we) = signal::<Bit, DefaultClock>();
    let (adr_o, adr) = signal::<U<28>, DefaultClock>();
    let (dat_o, dat) = signal::<U<32>, DefaultClock>();
    let (sel_o, sel) = signal::<U<4>, DefaultClock>();
    let (stall_o, stall) = signal::<Bit, DefaultClock>();
    let (ack_o, ack) = signal::<Bit, DefaultClock>();
    let (rdat_o, rdat) = signal::<U<32>, DefaultClock>();
    let mut h = AxiHost::<32, 32, 4, 2, 4>::default();
    let mut p = AxiPer::<32, 32, 4, 2>::default();
    let mut bridge = Bridge::default();
    let mem = WbMem::<28>::new(2, 10);
    let mut model = mem.clone();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // Every channel and wire under the name of the bridge's port.
        w.add("req", &req);
        w.add("wd", &wd);
        w.add("ans", &ans);
        w.add("rb", &rb);
        w.add("stall", &stall);
        w.add("ack", &ack);
        w.add("rdat", &rdat);
        w.add("cyc", &cyc);
        w.add("stb", &stb);
        w.add("we", &we);
        w.add("adr", &adr);
        w.add("dat", &dat);
        w.add("sel", &sel);
        w.add("bridge", &bridge);
        w.start();
    }
    let read_back = Rc::new(RefCell::new(Vec::new()));
    let out = read_back.clone();
    let words = [
        (0x4000_0010u32, 0x1111_1111u32),
        (0x4000_0020, 0x2222_2222),
        (0x4000_0014, 0x3333_3333),
    ];
    let client = async move {
        for (at, v) in words {
            let a = host.write(Wr::at(at), &[U::from(v)]).await;
            assert_eq!(a.done().await.resp, Resp::Okay);
        }
        for (at, _) in words {
            let r = host.read(Rd::at(at, 1)).await.done().await;
            out.borrow_mut().push(r.data[0].raw() as u32);
        }
    };
    // The memory before the bridge, so the bridge reads the memory's
    // lines in the cycle they are driven, as its netlist does.
    let (cyc_r, stb_r, we_r, adr_r) =
        (cyc.clone(), stb.clone(), we.clone(), adr.clone());
    let (stall_r, ack_r, rdat_r) = (stall.clone(), ack.clone(), rdat.clone());
    let mut sim = Running::new(join2(
        join2(h.run(host_in, host_out), p.run(per_in, per_out)),
        join2(
            join2(
                model.run(
                    (cyc, stb, we, adr, dat, sel),
                    (stall_o, ack_o, rdat_o),
                ),
                bridge.run(
                    (req, wd, stall, ack, rdat),
                    (ans, rb, cyc_o, stb_o, we_o, adr_o, dat_o, sel_o),
                ),
            ),
            client,
        ),
    ));
    println!(" t cyc stb we  adr        stall ack rdat");
    for t in 0..90 {
        sim.cycle();
        if cyc_r.get().to_bool() || ack_r.get().to_bool() {
            println!(
                "{t:2}  {}   {}   {}  {:#09x}   {}    {}  {:#010x}",
                cyc_r.get().to_bool() as u8,
                stb_r.get().to_bool() as u8,
                we_r.get().to_bool() as u8,
                adr_r.get().raw(),
                stall_r.get().to_bool() as u8,
                ack_r.get().to_bool() as u8,
                rdat_r.get().raw()
            );
        }
    }
    stop();
    let got = read_back.borrow();
    println!("read back: {:x?}", *got);
    assert_eq!(*got, vec![0x1111_1111, 0x2222_2222, 0x3333_3333]);
    // The region's base is above the word address's bits.
    assert_eq!(mem.word(4), 0x1111_1111, "word 4");
    assert_eq!(mem.word(8), 0x2222_2222, "word 8");
    let net = Bridge::lowered("axi_wb");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
