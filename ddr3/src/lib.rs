// SPDX-License-Identifier: Apache-2.0
//! The board's DDR3 memory as an AXI peripheral, with the controller
//! a module from elsewhere.
//!
//! [`Ddr3`] is AMD's MIG 7 Series controller generated with its AXI4
//! port, which the build generates (`//ddr3:ddr3_mig_axi`), behind the
//! thin wrapper in `hdl/ddr3_axi32.v`, which ties off what a design has
//! no reason to drive. It is a foreign unit: its netlist is an instance
//! of that module, and its `run` is a model of it, a memory on the same
//! AXI4 pins that is not ready while it calibrates and answers after the
//! controller's latency. The model does not drive the memory's pins;
//! nothing in a simulation reads them, and the pins only have to be in
//! the netlist. The controller makes the design's clock from the
//! board's, and the model leaves that clock and its reset alone.
//!
//! [`Ddr3Per`] is the peripheral a design puts on its link: the link's
//! peripheral end put onto the controller's pins by `AxiPerPins`, with
//! no tracker in front, since the controller keeps its own. It is a unit
//! of units and lowers as one, with the controller's module instantiated
//! in it and the memory's pins among its ports, so a board top has only
//! to join those pins to the package's, give it the board's clock, and
//! take the design's clock back from it.
pub mod bw;

use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Pad, Reg, Rx, Tx, Unit,
};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{Ar, Aw, B, R, W};
use txhdl_parts::bus::axi_per_pins::sim::{PinRam, PinRamIn, PinRamOut};
use txhdl_parts::bus::axi_per_pins::{
    AxiPerDriven, AxiPerPins, AxiPerPinsIn, AxiPerPinsOut,
};

/// The memory's words as an address: a gigabyte of them is two to the
/// twenty-eighth.
pub const AW: usize = 28;

/// The controller's inputs: the board's 200 MHz clock, which the
/// controller runs the memory from and takes as its delay reference;
/// its reset, active high at power-on and never again; and the AXI4
/// pins a host drives, address phases, write beats and the two readies
/// of the answers, as `AxiPerPins` drives them. The design's clock is
/// the controller's own making, bound to the module's clock pin, and is
/// not a port here.
pub type CtlIn = (
    In<Bit>,
    In<Bit>,
    In<U<5>>,
    In<U<32>>,
    In<U<8>>,
    In<U<3>>,
    In<U<2>>,
    In<Bit>,
    In<U<4>>,
    In<U<3>>,
    In<U<4>>,
    In<Bit>,
    In<U<32>>,
    In<U<4>>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<U<5>>,
    In<U<32>>,
    In<U<8>>,
    In<U<3>>,
    In<U<2>>,
    In<Bit>,
    In<U<4>>,
    In<U<3>>,
    In<U<4>>,
    In<Bit>,
    In<Bit>,
);

/// The controller's outputs: the AXI4 pins it drives, its readies and
/// its answers; calibration done; the design's clock and its reset,
/// which the controller makes; the memory's clock pair, reset, clock
/// enable, chip select, the three command strobes, row address, bank,
/// strobe masks and termination; and the memory's data and strobe pads.
pub type CtlOut = (
    Out<Bit>,
    Out<Bit>,
    Out<U<5>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
    Out<U<5>>,
    Out<U<32>>,
    Out<U<2>>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<Bit>,
    Out<U<15>>,
    Out<U<3>>,
    Out<U<4>>,
    Out<Bit>,
    Pad<U<32>>,
    Pad<U<4>>,
    Pad<U<4>>,
);

// begin{ctl}
/// The controller: AMD's MIG 7 Series with its AXI4 port, behind the
/// wrapper, which calibrates in hardware and, in simulation, is the
/// variant with the fast calibration the build compiles into its
/// library, so it takes no parameter for either.
#[derive(Trace, Default)]
pub struct Ddr3 {
    /// Whether calibration is done, in the model.
    pub calibrated: Reg<Bit>,
}

impl Lower for Ddr3 {
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "ddr3_axi32",
            &[
                ("i_sys_clk", Kind::In, 1),
                ("i_sys_rst", Kind::In, 1),
                ("i_awid", Kind::In, 5),
                ("i_awaddr", Kind::In, 32),
                ("i_awlen", Kind::In, 8),
                ("i_awsize", Kind::In, 3),
                ("i_awburst", Kind::In, 2),
                ("i_awlock", Kind::In, 1),
                ("i_awcache", Kind::In, 4),
                ("i_awprot", Kind::In, 3),
                ("i_awqos", Kind::In, 4),
                ("i_awvalid", Kind::In, 1),
                ("i_wdata", Kind::In, 32),
                ("i_wstrb", Kind::In, 4),
                ("i_wlast", Kind::In, 1),
                ("i_wvalid", Kind::In, 1),
                ("i_bready", Kind::In, 1),
                ("i_arid", Kind::In, 5),
                ("i_araddr", Kind::In, 32),
                ("i_arlen", Kind::In, 8),
                ("i_arsize", Kind::In, 3),
                ("i_arburst", Kind::In, 2),
                ("i_arlock", Kind::In, 1),
                ("i_arcache", Kind::In, 4),
                ("i_arprot", Kind::In, 3),
                ("i_arqos", Kind::In, 4),
                ("i_arvalid", Kind::In, 1),
                ("i_rready", Kind::In, 1),
                ("o_awready", Kind::Out, 1),
                ("o_wready", Kind::Out, 1),
                ("o_bid", Kind::Out, 5),
                ("o_bresp", Kind::Out, 2),
                ("o_bvalid", Kind::Out, 1),
                ("o_arready", Kind::Out, 1),
                ("o_rid", Kind::Out, 5),
                ("o_rdata", Kind::Out, 32),
                ("o_rresp", Kind::Out, 2),
                ("o_rlast", Kind::Out, 1),
                ("o_rvalid", Kind::Out, 1),
                ("o_calib_complete", Kind::Out, 1),
                ("o_ui_clk", Kind::Out, 1),
                ("o_ui_rst", Kind::Out, 1),
                ("o_ddr3_clk_p", Kind::Out, 1),
                ("o_ddr3_clk_n", Kind::Out, 1),
                ("o_ddr3_reset_n", Kind::Out, 1),
                ("o_ddr3_cke", Kind::Out, 1),
                ("o_ddr3_cs_n", Kind::Out, 1),
                ("o_ddr3_ras_n", Kind::Out, 1),
                ("o_ddr3_cas_n", Kind::Out, 1),
                ("o_ddr3_we_n", Kind::Out, 1),
                ("o_ddr3_addr", Kind::Out, 15),
                ("o_ddr3_ba_addr", Kind::Out, 3),
                ("o_ddr3_dm", Kind::Out, 4),
                ("o_ddr3_odt", Kind::Out, 1),
                ("io_ddr3_dq", Kind::Pad, 32),
                ("io_ddr3_dqs", Kind::Pad, 4),
                ("io_ddr3_dqs_n", Kind::Pad, 4),
            ],
            &[],
            &[("i_ui_clk", DefaultClock::NAME)],
        )
    }
}
// end{ctl}

/// Cycles the model is not ready for, before it has calibrated.
pub const MODEL_WARMUP: u32 = 64;

/// Cycles the model takes over a read, from the step it sees the address
/// phase to the step it offers the first beat. `//ddr3/sim:wrapper_test`
/// measured the controller against the Micron models: a read's first
/// beat came 25 cycles after its address phase was taken, a burst's 27,
/// and a read right behind a write 63. The model sees the address a
/// cycle after it is taken, so 24 is the common case.
pub const MODEL_READ_LATENCY: u32 = 24;

/// Cycles the model takes over a write, from the step it sees the last
/// beat to the step it offers the response: the controller answered 3
/// cycles after the last beat was taken, every time, in the same run.
pub const MODEL_WRITE_LATENCY: u32 = 2;

impl Unit<CtlIn, CtlOut> for Ddr3 {
    async fn run(
        &mut self,
        (
            _sys_clk,
            _sys_rst,
            awid,
            awaddr,
            awlen,
            _awsize,
            _awburst,
            _awlock,
            _awcache,
            _awprot,
            _awqos,
            awvalid,
            wdata,
            wstrb,
            wlast,
            wvalid,
            bready,
            arid,
            araddr,
            arlen,
            _arsize,
            _arburst,
            _arlock,
            _arcache,
            _arprot,
            _arqos,
            arvalid,
            rready,
        ): CtlIn,
        (
            awready,
            wready,
            bid,
            bresp,
            bvalid,
            arready,
            rid,
            rdata,
            rresp,
            rlast,
            rvalid,
            calib,
            ..,
        ): CtlOut,
    ) {
        // The memory: the whole gigabyte, its address the link's taken
        // modulo the memory's size as the port's thirty bits take it,
        // not ready until calibrated, and answering after the
        // controller's latency. A read takes its words when its address
        // is taken, so a write that lands while it waits is not in them,
        // as the controller may answer (issue 1424).
        let mem = PinRam::<32, 32, 4, 5>::on(
            PinRamIn {
                awid,
                awaddr,
                awlen,
                awvalid,
                wdata,
                wstrb,
                wlast,
                wvalid,
                bready,
                arid,
                araddr,
                arlen,
                arvalid,
                rready,
            },
            PinRamOut {
                awready,
                wready,
                arready,
                bid,
                bresp,
                bvalid,
                rid,
                rdata,
                rresp,
                rlast,
                rvalid,
            },
            1 << AW,
        )
        .wrapping()
        .reading_at_address()
        .timed(
            MODEL_READ_LATENCY as u64,
            MODEL_WRITE_LATENCY as u64,
            MODEL_WARMUP as u64,
        );
        let mut left = MODEL_WARMUP;
        let calibrated = &self.calibrated;
        join2(mem.serve(), async move {
            loop {
                DefaultClock::rising().await;
                calib.set(calibrated.get());
                calibrated.set(Bit::from_bool(left <= 1));
                left = left.saturating_sub(1);
            }
        })
        .await;
    }
}

// begin{per}
/// The memory as a peripheral: the link's peripheral end on the
/// controller's pins.
#[derive(Trace, Default)]
pub struct Ddr3Per {
    pub pins: AxiPerPins<32, 32, 4, 5>,
    pub ctl: Ddr3,
}

#[lower]
impl Unit for Ddr3Per {
    async fn run(
        &mut self,
        (aw, ar, w, b, r): (
            Rx<Aw<32, 5>>,
            Rx<Ar<32, 5>>,
            Rx<W<32, 4>>,
            Tx<B<5>>,
            Tx<R<32, 5>>,
        ),
        (
            sys_clk,
            sys_rst,
            calib,
            ui_clk,
            ui_rst,
            ck_p,
            ck_n,
            mem_rst_n,
            cke,
            cs_n,
            ras_n,
            cas_n,
            we_n,
            row,
            bank,
            dm,
            odt,
            dq,
            dqs,
            dqs_n,
        ): (
            In<Bit>,
            In<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<U<15>>,
            Out<U<3>>,
            Out<U<4>>,
            Out<Bit>,
            Pad<U<32>>,
            Pad<U<4>>,
            Pad<U<4>>,
        ),
    ) {
        // What the unit drives and the controller reads.
        let (awid_o, awid_i) = signal::<U<5>, DefaultClock>();
        let (awaddr_o, awaddr_i) = signal::<U<32>, DefaultClock>();
        let (awlen_o, awlen_i) = signal::<U<8>, DefaultClock>();
        let (awsize_o, awsize_i) = signal::<U<3>, DefaultClock>();
        let (awburst_o, awburst_i) = signal::<U<2>, DefaultClock>();
        let (awlock_o, awlock_i) = signal::<Bit, DefaultClock>();
        let (awcache_o, awcache_i) = signal::<U<4>, DefaultClock>();
        let (awprot_o, awprot_i) = signal::<U<3>, DefaultClock>();
        let (awqos_o, awqos_i) = signal::<U<4>, DefaultClock>();
        let (awvalid_o, awvalid_i) = signal::<Bit, DefaultClock>();
        let (wdata_o, wdata_i) = signal::<U<32>, DefaultClock>();
        let (wstrb_o, wstrb_i) = signal::<U<4>, DefaultClock>();
        let (wlast_o, wlast_i) = signal::<Bit, DefaultClock>();
        let (wvalid_o, wvalid_i) = signal::<Bit, DefaultClock>();
        let (bready_o, bready_i) = signal::<Bit, DefaultClock>();
        let (arid_o, arid_i) = signal::<U<5>, DefaultClock>();
        let (araddr_o, araddr_i) = signal::<U<32>, DefaultClock>();
        let (arlen_o, arlen_i) = signal::<U<8>, DefaultClock>();
        let (arsize_o, arsize_i) = signal::<U<3>, DefaultClock>();
        let (arburst_o, arburst_i) = signal::<U<2>, DefaultClock>();
        let (arlock_o, arlock_i) = signal::<Bit, DefaultClock>();
        let (arcache_o, arcache_i) = signal::<U<4>, DefaultClock>();
        let (arprot_o, arprot_i) = signal::<U<3>, DefaultClock>();
        let (arqos_o, arqos_i) = signal::<U<4>, DefaultClock>();
        let (arvalid_o, arvalid_i) = signal::<Bit, DefaultClock>();
        let (rready_o, rready_i) = signal::<Bit, DefaultClock>();
        // What the controller drives and the unit reads.
        let (awready_o, awready_i) = signal::<Bit, DefaultClock>();
        let (wready_o, wready_i) = signal::<Bit, DefaultClock>();
        let (bid_o, bid_i) = signal::<U<5>, DefaultClock>();
        let (bresp_o, bresp_i) = signal::<U<2>, DefaultClock>();
        let (bvalid_o, bvalid_i) = signal::<Bit, DefaultClock>();
        let (arready_o, arready_i) = signal::<Bit, DefaultClock>();
        let (rid_o, rid_i) = signal::<U<5>, DefaultClock>();
        let (rdata_o, rdata_i) = signal::<U<32>, DefaultClock>();
        let (rresp_o, rresp_i) = signal::<U<2>, DefaultClock>();
        let (rlast_o, rlast_i) = signal::<Bit, DefaultClock>();
        let (rvalid_o, rvalid_i) = signal::<Bit, DefaultClock>();
        // The controller first: in simulation it is the model, whose
        // pins the unit reads in the same step.
        join2(
            self.ctl.run(
                (
                    sys_clk, sys_rst, awid_i, awaddr_i, awlen_i, awsize_i,
                    awburst_i, awlock_i, awcache_i, awprot_i, awqos_i,
                    awvalid_i, wdata_i, wstrb_i, wlast_i, wvalid_i, bready_i,
                    arid_i, araddr_i, arlen_i, arsize_i, arburst_i, arlock_i,
                    arcache_i, arprot_i, arqos_i, arvalid_i, rready_i,
                ),
                (
                    awready_o, wready_o, bid_o, bresp_o, bvalid_o, arready_o,
                    rid_o, rdata_o, rresp_o, rlast_o, rvalid_o, calib, ui_clk,
                    ui_rst, ck_p, ck_n, mem_rst_n, cke, cs_n, ras_n, cas_n,
                    we_n, row, bank, dm, odt, dq, dqs, dqs_n,
                ),
            ),
            self.pins.run(
                AxiPerPinsIn {
                    pins: AxiPerDriven {
                        awready: awready_i,
                        wready: wready_i,
                        bid: bid_i,
                        bresp: bresp_i,
                        bvalid: bvalid_i,
                        arready: arready_i,
                        rid: rid_i,
                        rdata: rdata_i,
                        rresp: rresp_i,
                        rlast: rlast_i,
                        rvalid: rvalid_i,
                    },
                    aw,
                    ar,
                    w,
                },
                AxiPerPinsOut {
                    b,
                    r,
                    awid: awid_o,
                    awaddr: awaddr_o,
                    awlen: awlen_o,
                    awsize: awsize_o,
                    awburst: awburst_o,
                    awlock: awlock_o,
                    awcache: awcache_o,
                    awprot: awprot_o,
                    awqos: awqos_o,
                    awvalid: awvalid_o,
                    wdata: wdata_o,
                    wstrb: wstrb_o,
                    wlast: wlast_o,
                    wvalid: wvalid_o,
                    bready: bready_o,
                    arid: arid_o,
                    araddr: araddr_o,
                    arlen: arlen_o,
                    arsize: arsize_o,
                    arburst: arburst_o,
                    arlock: arlock_o,
                    arcache: arcache_o,
                    arprot: arprot_o,
                    arqos: arqos_o,
                    arvalid: arvalid_o,
                    rready: rready_o,
                },
            ),
        )
        .await;
    }
}
// end{per}

/// The peripheral on a link, driven from client code, and its netlist.
#[cfg(test)]
mod tests {
    use super::{Ddr3Per, MODEL_WARMUP};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, pad, signal, DefaultClock, Running, Unit};
    use txhdl::types::{Bit, U};
    use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};

    /// Words written across the memory and read back, the first ones
    /// issued while the controller is still calibrating, and a burst
    /// that streams.
    #[test]
    fn words_go_in_and_come_back() {
        let Link {
            host,
            host_in,
            host_out,
            per_in,
            per_out,
            ..
        } = axi::<32, 32, 4, 5, 32>();
        let (aw, ar, w, _, _) = per_in;
        let (_, _, b, r) = per_out;
        let (_sys_clk_o, sys_clk) = signal::<Bit, DefaultClock>();
        let (_sys_rst_o, sys_rst) = signal::<Bit, DefaultClock>();
        let (calib_o, calib) = signal::<Bit, DefaultClock>();
        let bits = || signal::<Bit, DefaultClock>().0;
        let mut h = AxiHost::<32, 32, 4, 5, 32>::default();
        let mut mem = Ddr3Per::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = seen.clone();
        let client = async move {
            let words =
                [(0x4000_0000u32, 1u32), (0x4000_0104, 2), (0x7fff_fffc, 3)];
            for (at, v) in words {
                let a = host.write(Wr::at(at), &[U::from(v)]).await;
                assert_eq!(a.done().await.resp, Resp::Okay);
            }
            for (at, _) in words {
                let r = host.read(Rd::at(at, 1)).await.done().await;
                out.borrow_mut().push(r.data[0].raw() as u32);
            }
            let burst: Vec<U<32>> = (0..16u32).map(U::from).collect();
            let a = host.write(Wr::at(0x4000_1000u32), &burst).await;
            assert_eq!(a.done().await.resp, Resp::Okay);
            let r = host.read(Rd::at(0x4000_1000u32, 16)).await.done().await;
            assert_eq!(r.data, burst, "the burst read back");
        };
        let mut sim = Running::new(join2(
            h.run(host_in, host_out),
            join2(
                mem.run(
                    (aw, ar, w, b, r),
                    (
                        sys_clk,
                        sys_rst,
                        calib_o,
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        bits(),
                        signal::<U<15>, DefaultClock>().0,
                        signal::<U<3>, DefaultClock>().0,
                        signal::<U<4>, DefaultClock>().0,
                        bits(),
                        pad::<U<32>, DefaultClock>(),
                        pad::<U<4>, DefaultClock>(),
                        pad::<U<4>, DefaultClock>(),
                    ),
                ),
                client,
            ),
        ));
        let mut calibrated_at = None;
        for c in 0..2000u32 {
            sim.cycle();
            if calibrated_at.is_none() && calib.get().to_bool() {
                calibrated_at = Some(c);
            }
        }
        assert!(
            calibrated_at.unwrap_or(0) >= MODEL_WARMUP - 1,
            "calibrated at {calibrated_at:?}"
        );
        assert_eq!(*seen.borrow(), vec![1, 2, 3], "the words read back");
    }

    /// The netlist holds the controller as an instance of the wrapper,
    /// its clock pin on the design's clock, the clock it makes and the
    /// board's clock among the peripheral's ports beside the pads, and
    /// no module of its own; the pins part is written, as a lowered
    /// child is.
    #[test]
    fn the_controller_is_instantiated_and_not_written() {
        let v = Ddr3Per::verilog("ddr3_per");
        assert!(v.contains("ddr3_axi32 "), "{v}");
        assert!(v.contains(".i_ui_clk(clk)"), "{v}");
        assert!(v.contains(".i_sys_clk(sys_clk)"), "{v}");
        assert!(v.contains(".o_ui_clk(ui_clk)"), "{v}");
        assert!(v.contains("inout [31:0] dq"), "{v}");
        assert!(v.contains(".io_ddr3_dq(dq)"), "{v}");
        assert!(v.contains("module ddr3_per_pins("), "{v}");
        assert!(!v.contains("module ddr3_axi32"), "{v}");
        let h = Ddr3Per::vhdl("ddr3_per");
        assert!(h.contains("component ddr3_axi32"), "{h}");
        assert!(
            h.contains("dq : inout std_logic_vector(31 downto 0)"),
            "{h}"
        );
        assert!(!h.contains("entity ddr3_axi32"), "{h}");
    }
}
