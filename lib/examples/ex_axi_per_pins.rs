// SPDX-License-Identifier: Apache-2.0
//! A peripheral on AXI4 pins, the way a core from elsewhere is one:
//! AMD's DDR3 controller generated with its AXI4 port, say.
//! `AxiPerPins` puts the link's peripheral end onto its pins, with a
//! host tracker and a client on the link's other end, as they would be
//! in front of a peripheral tracker.
//!
//! The widths are the controller's: 30-bit addresses, 32-bit words,
//! four lanes, five-bit identifiers. The client writes a burst of three
//! words, reads four back, and reads past the memory's end, which the
//! peripheral refuses. `AxiPerPins` is lowered, and the build simulates
//! its netlist against this run under nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, Running, Unit};
use txhdl::types::U;
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Wr};
use txhdl_parts::bus::axi_per_pins::sim::pins;
use txhdl_parts::bus::axi_per_pins::AxiPerPins;

/// The pins' widths: the controller's.
type Pins = AxiPerPins<30, 32, 4, 5>;

fn hex(ws: &[U<32>]) -> String {
    let s: Vec<String> = ws.iter().map(|w| format!("{:#x}", w.raw())).collect();
    s.join(" ")
}

fn main() {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<30, 32, 4, 5, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let (ram, inp, outp) = pins::<30, 32, 4, 5>(aw, ar, w, b, r, 16);
    let mut tracker = AxiHost::<30, 32, 4, 5, 4>::default();
    let mut pinned = Pins::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<txhdl::comp::DefaultClock>();
        // Every port under the name the netlist gives it.
        wave.add("pins_awready", &inp.pins.awready);
        wave.add("pins_wready", &inp.pins.wready);
        wave.add("pins_bid", &inp.pins.bid);
        wave.add("pins_bresp", &inp.pins.bresp);
        wave.add("pins_bvalid", &inp.pins.bvalid);
        wave.add("pins_arready", &inp.pins.arready);
        wave.add("pins_rid", &inp.pins.rid);
        wave.add("pins_rdata", &inp.pins.rdata);
        wave.add("pins_rresp", &inp.pins.rresp);
        wave.add("pins_rlast", &inp.pins.rlast);
        wave.add("pins_rvalid", &inp.pins.rvalid);
        wave.add("aw", &inp.aw);
        wave.add("ar", &inp.ar);
        wave.add("w", &inp.w);
        wave.add("outp_b", &outp.b);
        wave.add("outp_r", &outp.r);
        wave.add("outp_awid", &outp.awid);
        wave.add("outp_awaddr", &outp.awaddr);
        wave.add("outp_awlen", &outp.awlen);
        wave.add("outp_awsize", &outp.awsize);
        wave.add("outp_awburst", &outp.awburst);
        wave.add("outp_awlock", &outp.awlock);
        wave.add("outp_awcache", &outp.awcache);
        wave.add("outp_awprot", &outp.awprot);
        wave.add("outp_awqos", &outp.awqos);
        wave.add("outp_awvalid", &outp.awvalid);
        wave.add("outp_wdata", &outp.wdata);
        wave.add("outp_wstrb", &outp.wstrb);
        wave.add("outp_wlast", &outp.wlast);
        wave.add("outp_wvalid", &outp.wvalid);
        wave.add("outp_bready", &outp.bready);
        wave.add("outp_arid", &outp.arid);
        wave.add("outp_araddr", &outp.araddr);
        wave.add("outp_arlen", &outp.arlen);
        wave.add("outp_arsize", &outp.arsize);
        wave.add("outp_arburst", &outp.arburst);
        wave.add("outp_arlock", &outp.arlock);
        wave.add("outp_arcache", &outp.arcache);
        wave.add("outp_arprot", &outp.arprot);
        wave.add("outp_arqos", &outp.arqos);
        wave.add("outp_arvalid", &outp.arvalid);
        wave.add("outp_rready", &outp.rready);
        wave.add("pins", &pinned);
        wave.start();
    }

    let client = async move {
        let words = [
            U::from(0x1111_2222u32),
            U::from(0x3333_4444u32),
            U::from(0x5555_6666u32),
        ];
        let got = host.write(Wr::at(0x10u32), &words).await.done().await;
        println!("t={:>3} write 3 at 0x10 -> {:?}", now(), got.resp);
        let got = host.read(Rd::at(0xcu32, 4)).await.done().await;
        println!(
            "t={:>3} read 4 at 0xc -> {:?}: {}",
            now(),
            got.resp,
            hex(&got.data)
        );
        assert_eq!(got.data[1].raw(), 0x1111_2222);
        let got = host.read(Rd::at(0x100u32, 1)).await.done().await;
        println!(
            "t={:>3} read 1 at 0x100, past the end -> {:?}",
            now(),
            got.resp
        );
        println!("every burst answered through the pins");
    };
    // The memory first, then the unit: the unit reads the pins the
    // memory drives in the same step.
    let mut sim = Running::new(join2(
        client,
        join2(
            tracker.run(host_in, host_out),
            join2(ram.serve(), pinned.run(inp, outp)),
        ),
    ));
    for _ in 0..80 {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&Pins::lowered("axi_per_pins")]);
}
