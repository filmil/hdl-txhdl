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
use txhdl_parts::mmu::{DReq, IReq, Mmu8, Pte, Res};

use crate::core::{Vreteno, VretenoIn, VretenoOut, Writeback};

/// The core and its unit, eight entries in each of the unit's two
/// translation buffers.
#[txhdl::named_type_params(path = crate::hart)]
#[derive(Trace, Default)]
pub struct Hart<
    const ID: usize,
    const LANE_WORDS: usize = 16384,
    const ICACHE: usize = 1,
    const HART: usize = 0,
    const EXCLUSIVE: usize = 0,
> {
    pub mmu: Mmu8,
    pub core: Vreteno![
        ID = ID,
        LANE_WORDS = LANE_WORDS,
        ICACHE = ICACHE,
        HART = HART,
        EXCLUSIVE = EXCLUSIVE,
    ],
}

impl<
        const IW: usize,
        const DW: usize,
        const IC: usize,
        const HID: usize,
        const EXCL: usize,
    > Hart<IW, DW, IC, HID, EXCL>
{
    /// A hart whose core has its program loaded.
    pub fn with(program: &[u32]) -> Self {
        Hart {
            mmu: Mmu8::default(),
            core: Vreteno::with(program),
        }
    }
}

/// What a hart takes in (issue 1800): the core's inputs but for the
/// memory management unit's answers, which the hart makes inside.
/// Taken apart in `run`, so each port keeps its name.
pub struct HartIn<const IW: usize> {
    pub rst: In<Bit>,
    pub irq: In<Bit>,
    pub tirq: In<Bit>,
    pub sirq: In<Bit>,
    pub rdata: Rx<R<32, IW>>,
    pub done: Rx<Done<IW>>,
    pub grant: Rx<Grant<IW>>,
    pub haltreq: In<Bit>,
    pub resumereq: In<Bit>,
    pub dbg_regno: In<U<16>>,
    pub dbg_wdata: In<U<32>>,
    pub dbg_we: In<Bit>,
    pub time: In<U<64>>,
    pub seirq: In<Bit>,
    pub dc_snoop: In<U<9>>,
}

/// What a hart gives out (issue 1800): the core's outputs but for its
/// lines to the memory management unit, which stay inside.
pub struct HartOut<const IW: usize> {
    pub halt: Out<Bit>,
    pub instr: Out<U<32>>,
    pub wb: Out<Writeback>,
    pub issue: Tx<Issue<32>>,
    pub wbeat: Tx<W<32, 4>>,
    pub release: Tx<Grant<IW>>,
    pub dbg: Out<Bit>,
    pub dbg_rdata: Out<U<32>>,
    pub asleep: Out<Bit>,
}

// begin{run}
#[lower]
impl<
        const IW: usize,
        const DW: usize,
        const IC: usize,
        const HID: usize,
        const EXCL: usize,
    > Unit for Hart<IW, DW, IC, HID, EXCL>
{
    async fn run(
        &mut self,
        HartIn {
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
            dc_snoop,
        }: HartIn<IW>,
        HartOut {
            halt,
            instr,
            wb,
            issue,
            wbeat,
            release,
            dbg,
            dbg_rdata,
            asleep,
        }: HartOut<IW>,
    ) {
        let rst_mmu = rst.clone();
        // What the core tells the unit.
        let (satp_o, satp_i) = signal::<U<32>, DefaultClock>();
        let (prv_o, prv_i) = signal::<U<2>, DefaultClock>();
        let (sum_o, sum_i) = signal::<Bit, DefaultClock>();
        let (mxr_o, mxr_i) = signal::<Bit, DefaultClock>();
        let (flush_o, flush_i) = signal::<Bit, DefaultClock>();
        let (ireq_o, ireq_i) = signal::<IReq, DefaultClock>();
        let (dreq_o, dreq_i) = signal::<DReq, DefaultClock>();
        // What the unit answers.
        let (ires_o, ires_i) = signal::<Res, DefaultClock>();
        let (dres_o, dres_i) = signal::<Res, DefaultClock>();
        // The walker's reads, which the core puts on the bus, and what
        // they read.
        let (ptw_tx, ptw_rx) = chan::<U<32>, DefaultClock>();
        let (pte_tx, pte_rx) = chan::<Pte, DefaultClock>();
        join2(
            self.mmu.run(
                (
                    rst_mmu, satp_i, prv_i, sum_i, mxr_i, flush_i, ireq_i,
                    dreq_i, pte_rx,
                ),
                (ires_o, dres_o, ptw_tx),
            ),
            self.core.run(
                VretenoIn {
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
                    dc_snoop,
                    ires: ires_i,
                    dres: dres_i,
                    ptw: ptw_rx,
                },
                VretenoOut {
                    halt,
                    instr,
                    wb,
                    issue,
                    wbeat,
                    release,
                    dbg,
                    dbg_rdata,
                    asleep,
                    mmu_satp: satp_o,
                    mmu_prv: prv_o,
                    mmu_sum: sum_o,
                    mmu_mxr: mxr_o,
                    mmu_flush: flush_o,
                    ireq: ireq_o,
                    dreq: dreq_o,
                    pte: pte_tx,
                },
            ),
        )
        .await;
    }
}
// end{run}
