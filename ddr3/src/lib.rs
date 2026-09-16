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
use txhdl::comp::trace::Kind;
use txhdl::comp::{join2, Clock, DefaultClock, In, Out, Pad, Reg, Unit};
use txhdl::netlist::{foreign, Lower, Lowered};
use txhdl::types::{Bit, U};
use txhdl::Trace;
use txhdl_parts::bus::wb::sim::WbMem;

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

