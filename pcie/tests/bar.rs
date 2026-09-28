// SPDX-License-Identifier: Apache-2.0
//! Everything behind BAR1, driven through the endpoint's master pins as
//! the endpoint drives them when a host reads and writes the BAR, and
//! its netlist.
use pcie::bar::{
    PcieBar, PcieBarIn, PcieBarOut, IDENT, IDENT_AT, LIGHTS_AT, SCRATCH_AT,
    WRITES_AT,
};
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_pins::sim::pin_host;

/// Every word by the offset the documents list it at, so a table that
/// named a word at the wrong offset would fail here (#444).
#[test]
fn a_host_reads_and_writes_the_bar() {
    let (host, h, d) = pin_host::<32, 64, 8, 4>();
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (leds_o, leds) = signal::<U<2>, DefaultClock>();
    let mut bar = PcieBar::default();
    let done = Rc::new(RefCell::new(false));
    let fin = done.clone();
    let lights = leds.clone();
    let client = async move {
        let got = host.read(0, IDENT_AT as _, 1).await;
        assert_eq!(got.resp, Resp::Okay);
        assert_eq!(got.data[0].raw(), IDENT as u128, "the identifier");
        let word = U::from(0x0123_4567_89ab_cdefu64);
        let ok = host.write(1, SCRATCH_AT as _, &[word]).await;
        assert_eq!(ok.resp, Resp::Okay);
        let got = host.read(2, SCRATCH_AT as _, 1).await;
        assert_eq!(got.data[0].raw(), 0x0123_4567_89ab_cdef, "scratch");
        assert_eq!(got.id.raw(), 2, "the read's identifier comes back");
        let ok = host.write(3, LIGHTS_AT as _, &[U::from(2u8)]).await;
        assert_eq!(ok.id.raw(), 3);
        assert_eq!(lights.get().raw(), 2, "the LEDs follow the word");
        // A write to the identifier is served, and changes nothing.
        host.write(4, IDENT_AT as _, &[U::from(0u8)]).await;
        let got = host.read(5, IDENT_AT as _, 1).await;
        assert_eq!(got.data[0].raw(), IDENT as u128, "read only");
        let got = host.read(6, WRITES_AT as _, 1).await;
        assert_eq!(got.data[0].raw(), 3, "three writes served");
        *fin.borrow_mut() = true;
    };
    let mut sim = Running::new(join2(
        client,
        bar.run(
            PcieBarIn {
                rst,
                awid: h.awid,
                awaddr: h.awaddr,
                awlen: h.awlen,
                awsize: h.awsize,
                awburst: h.awburst,
                awlock: h.awlock,
                awcache: h.awcache,
                awprot: h.awprot,
                awvalid: h.awvalid,
                wdata: h.wdata,
                wstrb: h.wstrb,
                wlast: h.wlast,
                wvalid: h.wvalid,
                bready: h.bready,
                arid: h.arid,
                araddr: h.araddr,
                arlen: h.arlen,
                arsize: h.arsize,
                arburst: h.arburst,
                arlock: h.arlock,
                arcache: h.arcache,
                arprot: h.arprot,
                arvalid: h.arvalid,
                rready: h.rready,
            },
            PcieBarOut {
                awready: d.awready,
                wready: d.wready,
                bid: d.bid,
                bresp: d.bresp,
                bvalid: d.bvalid,
                arready: d.arready,
                rid: d.rid,
                rdata: d.rdata,
                rresp: d.rresp,
                rlast: d.rlast,
                rvalid: d.rvalid,
                leds: leds_o,
            },
        ),
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    for _ in 0..400 {
        sim.cycle();
        if *done.borrow() {
            break;
        }
    }
    assert!(*done.borrow(), "every access was answered");
    let _ = leds;
}

/// One module holds the rest, with the pins' names as its ports.
#[test]
fn the_netlist_has_the_endpoint_pins() {
    let v = PcieBar::verilog("pcie_bar");
    assert!(v.contains("module pcie_bar("), "the top");
    for port in ["awaddr", "rdata", "bvalid", "leds"] {
        assert!(v.contains(port), "{port}");
    }
}
