// SPDX-License-Identifier: Apache-2.0
//! The board's DDR3 memory as an AXI peripheral, with the controller
//! a module from elsewhere.
//!
//! [`Ddr3`] is UberDDR3's controller behind the thin wrapper in
//! `hdl/ddr3_wb32.v`, which gives it a Wishbone of thirty-two bit
//! words. It is a foreign unit: its netlist is an instance of that
//! module, and its `run` is a model of it, a memory behind the same
//! Wishbone lines that stalls while it calibrates. The model does not
//! drive the memory's pins; nothing in a simulation reads them, and
//! the pins only have to be in the netlist.
//!
//! [`Ddr3Per`] is the peripheral a design puts on its link: the AXI to
//! Wishbone bridge of `txhdl_parts` joined to the controller. It is a
//! unit of units and lowers as one, with the controller's module
//! instantiated in it and the memory's pins among its ports, so a
//! board top has only to join those pins to the package's and give it
//! its clocks.
use txhdl::comp::trace::Kind;
use txhdl::comp::{
    join2, signal, Clock, DefaultClock, In, Out, Pad, Reg, Rx, Tx, Unit,
};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{Answer, PerReq, R, W};
use txhdl_parts::bus::wb::sim::WbMem;
use txhdl_parts::bus::wb::AxiWb;

/// The Wishbone word address: fifteen row bits, ten column bits and
/// three bank bits, the last three the lane of a word in its burst.
pub const AW: usize = 28;

/// The controller's inputs: the memory clock, the reference clock, the
/// memory clock a quarter cycle late, the reset, active low, and the
/// Wishbone host's lines. The controller clock is the design's own and
/// is not a port.
pub type CtlIn = (
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<Bit>,
    In<U<AW>>,
    In<U<32>>,
    In<U<4>>,
);

/// The controller's outputs: stall, acknowledge and the word read;
/// calibration done; the memory's clock pair, reset, clock enable,
/// chip select, the three command strobes, row address, bank, strobe
/// masks and termination; and the memory's data and strobe pads.
pub type CtlOut = (
    Out<Bit>,
    Out<Bit>,
    Out<U<32>>,
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
/// The controller. `MICRON_SIM` shortens its power-on waits for the
/// Micron model, one in simulation and zero on the board; `BIST` is its
/// self test after calibration, which walks the whole memory once and
/// is zero where a simulation cannot wait for that.
#[derive(Trace, Default)]
pub struct Ddr3<const MICRON_SIM: usize, const BIST: usize> {
    /// Whether calibration is done, in the model.
    pub calibrated: Reg<Bit>,
}

impl<const MICRON_SIM: usize, const BIST: usize> Lower
    for Ddr3<MICRON_SIM, BIST>
{
    fn lowered_as(name: &str) -> Lowered {
        foreign(
            name,
            "ddr3_wb32",
            &[
                ("i_ddr3_clk", Kind::In, 1),
                ("i_ref_clk", Kind::In, 1),
                ("i_ddr3_clk_90", Kind::In, 1),
                ("i_rst_n", Kind::In, 1),
                ("i_wb_cyc", Kind::In, 1),
                ("i_wb_stb", Kind::In, 1),
                ("i_wb_we", Kind::In, 1),
                ("i_wb_addr", Kind::In, AW),
                ("i_wb_data", Kind::In, 32),
                ("i_wb_sel", Kind::In, 4),
                ("o_wb_stall", Kind::Out, 1),
                ("o_wb_ack", Kind::Out, 1),
                ("o_wb_data", Kind::Out, 32),
                ("o_calib_complete", Kind::Out, 1),
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
            &[
                ("MICRON_SIM", MICRON_SIM as i128),
                ("BIST_MODE", BIST as i128),
            ],
            &[("i_controller_clk", DefaultClock::NAME)],
        )
    }
}
// end{ctl}

/// Cycles the model stalls for before it has calibrated.
pub const MODEL_WARMUP: u32 = 64;

impl<const MICRON_SIM: usize, const BIST: usize> Unit<CtlIn, CtlOut>
    for Ddr3<MICRON_SIM, BIST>
{
    async fn run(
        &mut self,
        (_ck, _rck, _ck90, _rst_n, cyc, stb, we, adr, dat, sel): CtlIn,
        (stall, ack, rdat, calib, ..): CtlOut,
    ) {
        // The memory: a word a request, answered a cycle after it is
        // taken, after the calibration's stall.
        let mut mem = WbMem::<AW>::new(1, MODEL_WARMUP);
        let mut left = MODEL_WARMUP;
        let calibrated = &self.calibrated;
        join2(
            mem.run((cyc, stb, we, adr, dat, sel), (stall, ack, rdat)),
            async move {
                loop {
                    DefaultClock::rising().await;
                    calib.set(calibrated.get());
                    calibrated.set(Bit::from_bool(left <= 1));
                    left = left.saturating_sub(1);
                }
            },
        )
        .await;
    }
}

// begin{per}
/// The memory as a peripheral: the bridge and the controller, joined by
/// the Wishbone lines. `MICRON_SIM` and `BIST` are the controller's.
#[derive(Trace, Default)]
pub struct Ddr3Per<const MICRON_SIM: usize, const BIST: usize> {
    pub bridge: AxiWb<32, 2, AW>,
    pub ctl: Ddr3<MICRON_SIM, BIST>,
}

#[lower]
impl<const MICRON_SIM: usize, const BIST: usize> Unit
    for Ddr3Per<MICRON_SIM, BIST>
{
    async fn run(
        &mut self,
        (req, wd, ddr3_clk, ref_clk, ddr3_clk_90, rst_n): (
            Rx<PerReq<32, 2>>,
            Rx<W<32, 4>>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
        ),
        (
            ans,
            rb,
            calib,
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
            Tx<Answer<2>>,
            Tx<R<32, 2>>,
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
        let (wb_cyc_o, wb_cyc_i) = signal::<Bit, DefaultClock>();
        let (wb_stb_o, wb_stb_i) = signal::<Bit, DefaultClock>();
        let (wb_we_o, wb_we_i) = signal::<Bit, DefaultClock>();
        let (wb_adr_o, wb_adr_i) = signal::<U<AW>, DefaultClock>();
        let (wb_dat_o, wb_dat_i) = signal::<U<32>, DefaultClock>();
        let (wb_sel_o, wb_sel_i) = signal::<U<4>, DefaultClock>();
        let (wb_stall_o, wb_stall_i) = signal::<Bit, DefaultClock>();
        let (wb_ack_o, wb_ack_i) = signal::<Bit, DefaultClock>();
        let (wb_rdat_o, wb_rdat_i) = signal::<U<32>, DefaultClock>();
        join2(
            self.ctl.run(
                (
                    ddr3_clk,
                    ref_clk,
                    ddr3_clk_90,
                    rst_n,
                    wb_cyc_i,
                    wb_stb_i,
                    wb_we_i,
                    wb_adr_i,
                    wb_dat_i,
                    wb_sel_i,
                ),
                (
                    wb_stall_o, wb_ack_o, wb_rdat_o, calib, ck_p, ck_n,
                    mem_rst_n, cke, cs_n, ras_n, cas_n, we_n, row, bank, dm,
                    odt, dq, dqs, dqs_n,
                ),
            ),
            self.bridge.run(
                (req, wd, wb_stall_i, wb_ack_i, wb_rdat_i),
                (
                    ans, rb, wb_cyc_o, wb_stb_o, wb_we_o, wb_adr_o, wb_dat_o,
                    wb_sel_o,
                ),
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
    use txhdl_parts::bus::axi::{
        axi_to_unit, AxiHost, AxiPer, HostLink, Rd, Resp, Wr,
    };

    /// Words written across the memory and read back, the first ones
    /// issued while the controller is still calibrating.
    #[test]
    fn words_go_in_and_come_back() {
        let HostLink {
            host,
            per_client,
            host_in,
            host_out,
            per_in,
            per_out,
        } = axi_to_unit::<32, 32, 4, 2, 4>();
        let (req, wd, ans, rb) = per_client;
        let (_ck_o, ck) = signal::<Bit, DefaultClock>();
        let (_rck_o, rck) = signal::<Bit, DefaultClock>();
        let (_ck90_o, ck90) = signal::<Bit, DefaultClock>();
        let (_rst_o, rst_n) = signal::<Bit, DefaultClock>();
        let (calib_o, calib) = signal::<Bit, DefaultClock>();
        let bits = || signal::<Bit, DefaultClock>().0;
        let mut h = AxiHost::<32, 32, 4, 2, 4>::default();
        let mut p = AxiPer::<32, 32, 4, 2>::default();
        let mut mem = Ddr3Per::<0, 0>::default();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let out = seen.clone();
        let client = async move {
            let words =
                [(0x4000_0000u32, 1u32), (0x4000_0104, 2), (0x47ff_fffc, 3)];
            for (at, v) in words {
                let a = host.write(Wr::at(at), &[U::from(v)]).await;
                assert_eq!(a.done().await.resp, Resp::Okay);
            }
            for (at, _) in words {
                let r = host.read(Rd::at(at, 1)).await.done().await;
                out.borrow_mut().push(r.data[0].raw() as u32);
            }
        };
        let mut sim = Running::new(join2(
            join2(h.run(host_in, host_out), p.run(per_in, per_out)),
            join2(
                mem.run(
                    (req, wd, ck, rck, ck90, rst_n),
                    (
                        ans,
                        rb,
                        calib_o,
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
    /// with its parameters, its clock pin on the design's clock and its
    /// pads running out to the peripheral's ports, and no module of its
    /// own; the bridge is written, as a lowered child is.
    #[test]
    fn the_controller_is_instantiated_and_not_written() {
        let v = Ddr3Per::<1, 0>::verilog("ddr3_per");
        assert!(v.contains("ddr3_wb32 #("), "{v}");
        assert!(v.contains(".MICRON_SIM(1)"), "{v}");
        assert!(v.contains(".BIST_MODE(0)"), "{v}");
        assert!(v.contains(".i_controller_clk(clk)"), "{v}");
        assert!(v.contains("inout [31:0] dq"), "{v}");
        assert!(v.contains(".io_ddr3_dq(dq)"), "{v}");
        assert!(v.contains("module ddr3_per_bridge("), "{v}");
        assert!(!v.contains("module ddr3_wb32"), "{v}");
        let h = Ddr3Per::<1, 0>::vhdl("ddr3_per");
        assert!(h.contains("component ddr3_wb32"), "{h}");
        assert!(
            h.contains("generic map (MICRON_SIM => 1, BIST_MODE => 0)"),
            "{h}"
        );
        assert!(
            h.contains("dq : inout std_logic_vector(31 downto 0)"),
            "{h}"
        );
        assert!(h.contains("unsigned(o_wb_data) => wb_rdat"), "{h}");
        assert!(!h.contains("entity ddr3_wb32"), "{h}");
    }
}
