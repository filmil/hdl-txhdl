// SPDX-License-Identifier: Apache-2.0
//! A host on AXI4 pins, the way a core from elsewhere is one: a PCIe
//! endpoint whose BAR is an AXI master, say. `AxiPins` joins its pins
//! to the link's channels, and a peripheral tracker and a RAM are
//! behind it, as they would be behind a host tracker.
//!
//! The widths are the PCIe endpoint's: 32-bit addresses on this side,
//! 64-bit words, eight lanes, four-bit identifiers. The host writes a
//! burst of three words, reads four back, and reads past the RAM's
//! end, which the peripheral refuses. `AxiPins` is lowered, and the
//! build simulates its netlist against this run under nvc and under
//! Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, Running, Unit};
use txhdl::types::U;
use txhdl_parts::bus::axi::sim::Ram;
use txhdl_parts::bus::axi::{axi, AxiPer, Link};
use txhdl_parts::bus::axi_pins::sim::pins;
use txhdl_parts::bus::axi_pins::AxiPins;

/// The pins' widths: the endpoint's.
type Pins = AxiPins<32, 64, 8, 4>;

fn hex(ws: &[U<64>]) -> String {
    let s: Vec<String> = ws.iter().map(|w| format!("{:#x}", w.raw())).collect();
    s.join(" ")
}

fn main() {
    let Link {
        host_in,
        host_out,
        per,
        per_in,
        per_out,
        ..
    } = axi::<32, 64, 8, 4, 16>();
    let (aw, ar, w, _, _, _) = host_out;
    let (_, _, b, r, _) = host_in;
    let (host, inp, outp) = pins::<32, 64, 8, 4>(aw, ar, w, b, r);
    let ram = Ram::<32, 64, 8, 4>::new(16);
    let mut pinned = Pins::default();
    let mut tracker = AxiPer::<32, 64, 8, 4>::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<txhdl::comp::DefaultClock>();
        // Every port under the name the netlist gives it.
        wave.add("inp_pins_awid", &inp.pins.awid);
        wave.add("inp_pins_awaddr", &inp.pins.awaddr);
        wave.add("inp_pins_awlen", &inp.pins.awlen);
        wave.add("inp_pins_awsize", &inp.pins.awsize);
        wave.add("inp_pins_awburst", &inp.pins.awburst);
        wave.add("inp_pins_awlock", &inp.pins.awlock);
        wave.add("inp_pins_awcache", &inp.pins.awcache);
        wave.add("inp_pins_awprot", &inp.pins.awprot);
        wave.add("inp_pins_awvalid", &inp.pins.awvalid);
        wave.add("inp_pins_wdata", &inp.pins.wdata);
        wave.add("inp_pins_wstrb", &inp.pins.wstrb);
        wave.add("inp_pins_wlast", &inp.pins.wlast);
        wave.add("inp_pins_wvalid", &inp.pins.wvalid);
        wave.add("inp_pins_bready", &inp.pins.bready);
        wave.add("inp_pins_arid", &inp.pins.arid);
        wave.add("inp_pins_araddr", &inp.pins.araddr);
        wave.add("inp_pins_arlen", &inp.pins.arlen);
        wave.add("inp_pins_arsize", &inp.pins.arsize);
        wave.add("inp_pins_arburst", &inp.pins.arburst);
        wave.add("inp_pins_arlock", &inp.pins.arlock);
        wave.add("inp_pins_arcache", &inp.pins.arcache);
        wave.add("inp_pins_arprot", &inp.pins.arprot);
        wave.add("inp_pins_arvalid", &inp.pins.arvalid);
        wave.add("inp_pins_rready", &inp.pins.rready);
        wave.add("inp_b", &inp.b);
        wave.add("inp_r", &inp.r);
        wave.add("outp_aw", &outp.aw);
        wave.add("outp_ar", &outp.ar);
        wave.add("outp_w", &outp.w);
        wave.add("outp_awready", &outp.awready);
        wave.add("outp_wready", &outp.wready);
        wave.add("outp_bid", &outp.bid);
        wave.add("outp_bresp", &outp.bresp);
        wave.add("outp_bvalid", &outp.bvalid);
        wave.add("outp_arready", &outp.arready);
        wave.add("outp_rid", &outp.rid);
        wave.add("outp_rdata", &outp.rdata);
        wave.add("outp_rresp", &outp.rresp);
        wave.add("outp_rlast", &outp.rlast);
        wave.add("outp_rvalid", &outp.rvalid);
        wave.add("pins", &pinned);
        wave.start();
    }

    // The host first: it drives the pins, which are wires, and the unit
    // reads them in the same step.
    let client = async move {
        let words = [
            U::from(0x1111_2222_3333_4444u64),
            U::from(0x5555_6666_7777_8888u64),
            U::from(0x9999_aaaa_bbbb_ccccu64),
        ];
        let got = host.write(5, 0x10, &words).await;
        println!(
            "t={:>3} write 3 at 0x10, id 5 -> {:?}, id {}",
            now(),
            got.resp,
            got.id.raw()
        );
        let got = host.read(6, 0x8, 4).await;
        println!(
            "t={:>3} read 4 at 0x8, id 6 -> {:?}: {}",
            now(),
            got.resp,
            hex(&got.data)
        );
        assert_eq!(got.data[1].raw(), 0x1111_2222_3333_4444);
        let got = host.read(7, 0x100, 1).await;
        println!(
            "t={:>3} read 1 at 0x100, past the end -> {:?}",
            now(),
            got.resp
        );
        println!("every burst answered through the pins");
    };
    let mut sim = Running::new(join2(
        client,
        join2(
            pinned.run(inp, outp),
            join2(tracker.run(per_in, per_out), ram.serve(per, 4)),
        ),
    ));
    for _ in 0..80 {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&Pins::lowered("axi_pins")]);
}
