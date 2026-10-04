// SPDX-License-Identifier: Apache-2.0
//! The hart: the core and its memory management unit, joined, with the
//! core's ports and nothing more (issue 1014). Everything outside sees
//! one core; the unit translates the core's addresses for Sv32, and its
//! walker reads the page tables through the core's own port on the
//! bus, so the hart needs no port of its own for them.
//!
//! The unit goes first in the join: its answers are registers, which
//! the core reads in the same step. The core's requests are registers
//! too, and the unit reads them a step late in simulation and on time
//! in hardware, so the core waits two cycles on every request before
//! it believes an answer, which is right in both: the unit holds its
//! answer while the request is held.
use txhdl::comp::{chan, join2, signal, DefaultClock, In, Out, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{Done, Grant, Issue, R, W};
use txhdl_parts::mmu::{Mmu8, Pte};

use crate::core::{Vreteno, Writeback};

/// The core and its unit, eight entries in each of the unit's two
/// translation buffers.
#[derive(Trace, Default)]
pub struct Hart<const IW: usize> {
    pub mmu: Mmu8,
    pub core: Vreteno<IW>,
}

impl<const IW: usize> Hart<IW> {
    /// A hart whose core has its program loaded.
    pub fn with(program: &[u32]) -> Self {
        Hart {
            mmu: Mmu8::default(),
            core: Vreteno::with(program),
        }
    }
}

// begin{run}
#[lower]
impl<const IW: usize> Unit for Hart<IW> {
    async fn run(
        &mut self,
        (
            rst,
            irq,
            tirq,
            sirq,
            rdata,
            done,
            grant,
            haltreq,
            resumereq,
            dbg_regno,
            dbg_wdata,
            dbg_we,
            time,
            seirq,
        ): (
            In<Bit>,
            In<Bit>,
            In<Bit>,
            In<Bit>,
            Rx<R<32, IW>>,
            Rx<Done<IW>>,
            Rx<Grant<IW>>,
            In<Bit>,
            In<Bit>,
            In<U<16>>,
            In<U<32>>,
            In<Bit>,
            In<U<64>>,
            In<Bit>,
        ),
        (halt, instr, wb, issue, wbeat, release, dbg, dbg_rdata): (
            Out<Bit>,
            Out<U<32>>,
            Out<Writeback>,
            Tx<Issue<32>>,
            Tx<W<32, 4>>,
            Tx<Grant<IW>>,
            Out<Bit>,
            Out<U<32>>,
        ),
    ) {
        let rst_mmu = rst.clone();
        // What the core tells the unit.
        let (satp_o, satp_i) = signal::<U<32>, DefaultClock>();
        let (prv_o, prv_i) = signal::<U<2>, DefaultClock>();
        let (sum_o, sum_i) = signal::<Bit, DefaultClock>();
        let (mxr_o, mxr_i) = signal::<Bit, DefaultClock>();
        let (flush_o, flush_i) = signal::<Bit, DefaultClock>();
        let (ireq_o, ireq_i) = signal::<Bit, DefaultClock>();
        let (iva_o, iva_i) = signal::<U<32>, DefaultClock>();
        let (dreq_o, dreq_i) = signal::<Bit, DefaultClock>();
        let (dva_o, dva_i) = signal::<U<32>, DefaultClock>();
        let (dst_o, dst_i) = signal::<Bit, DefaultClock>();
        // What the unit answers.
        let (iok_o, iok_i) = signal::<Bit, DefaultClock>();
        let (ipa_o, ipa_i) = signal::<U<32>, DefaultClock>();
        let (ipf_o, ipf_i) = signal::<Bit, DefaultClock>();
        let (iaf_o, iaf_i) = signal::<Bit, DefaultClock>();
        let (dok_o, dok_i) = signal::<Bit, DefaultClock>();
        let (dpa_o, dpa_i) = signal::<U<32>, DefaultClock>();
        let (dpf_o, dpf_i) = signal::<Bit, DefaultClock>();
        let (daf_o, daf_i) = signal::<Bit, DefaultClock>();
        // The walker's reads, which the core puts on the bus, and what
        // they read.
        let (ptw_tx, ptw_rx) = chan::<U<32>, DefaultClock>();
        let (pte_tx, pte_rx) = chan::<Pte, DefaultClock>();
        join2(
            self.mmu.run(
                (
                    rst_mmu, satp_i, prv_i, sum_i, mxr_i, flush_i, ireq_i,
                    iva_i, dreq_i, dva_i, dst_i, pte_rx,
                ),
                (
                    iok_o, ipa_o, ipf_o, iaf_o, dok_o, dpa_o, dpf_o, daf_o,
                    ptw_tx,
                ),
            ),
            self.core.run(
                (
                    rst, irq, tirq, sirq, rdata, done, grant, haltreq,
                    resumereq, dbg_regno, dbg_wdata, dbg_we, time, seirq,
                    iok_i, ipa_i, ipf_i, iaf_i, dok_i, dpa_i, dpf_i, daf_i,
                    ptw_rx,
                ),
                (
                    halt, instr, wb, issue, wbeat, release, dbg, dbg_rdata,
                    satp_o, prv_o, sum_o, mxr_o, flush_o, ireq_o, iva_o,
                    dreq_o, dva_o, dst_o, pte_tx,
                ),
            ),
        )
        .await;
    }
}
// end{run}
