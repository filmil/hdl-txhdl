// SPDX-License-Identifier: Apache-2.0
//! Two cores on one bus, with everything they say compared: the pair
//! runs a program and says nothing, and then one of them is held in
//! reset for a cycle and it says so.
//!
//! The harness is the lockstep test's, with the pair where the core
//! was: a tracker, a router, the data memory, the timer and the serial
//! port. What leaves the pair is the first core's, so the rest of the
//! design is wired as it would be around one core.
use txhdl::comp::{join2, signal, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::bus::router::Router;
use vreteno32::core::{Vreteno, Writeback};
use vreteno32::dmem::Dmem;
use vreteno32::pair::Pair;
use vreteno32::program::random;
use vreteno32::timer::Timer;
use vreteno32::uart::Uart;

const IW: usize = 2;
const NIDS: usize = 4;
/// The address map: the data memory at its page, the timer at
/// `0x0200_0000`, and the serial port at `0x3000`.
struct RunMap;

impl AddrMap<3> for RunMap {
    const RANGES: [(usize, usize); 3] = [
        (0x1000, 0xf000),
        (0x0200_0000, 0xffff_0000),
        (0x3000, 0xf000),
    ];
}

type Rtr = Router<3, RunMap, 32, 32, 4, IW>;
type Serial = LiteBridge<1, SerialMap, 32, 32, 4, IW>;

/// Where the serial port is: a nibble of the address space at 0x3000.
pub struct SerialMap;

impl AddrMap<1> for SerialMap {
    const RANGES: [(usize, usize); 1] = [(0x3000, 0xf000)];
}

/// Runs `program` on a pair for `cycles` cycles, holding the second
/// core in reset during the cycles `fault` names. Answers whether the
/// pair ever said the two disagreed, and whether it was still saying
/// so at the end.
fn pair_says(program: &[u32], cycles: usize, fault: &[usize]) -> (bool, bool) {
    let mut pair = Pair::<IW> {
        one: Vreteno::with(program),
        two: Vreteno::with(program),
        ..Default::default()
    };
    let mut dmem = Dmem::<IW>::default();
    let mut timer = Timer::<IW>::default();
    let mut uart = Uart::<4>::default();
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();
    let (tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (sirq_out, sirq) = signal::<Bit, DefaultClock>();
    let (fault_out, fault_in) = signal::<Bit, DefaultClock>();
    let (tx_out, _tx) = signal::<Bit, DefaultClock>();
    let (rx_out, rx) = signal::<Bit, DefaultClock>();
    let (uirq_out, _uirq) = signal::<Bit, DefaultClock>();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();
    let (differs_out, differs) = signal::<Bit, DefaultClock>();
    let cl = axi_units::<32, 32, 4, IW>();
    let dl = axi_units::<32, 32, 4, IW>();
    let tl = axi_units::<32, 32, 4, IW>();
    let ul = axi_units::<32, 32, 4, IW>();
    let (issue, wbeat, release, grant, cdone, crdata) = cl.host_client;
    let dbus = PerPort::from(dl.per_client);
    let tbus = PerPort::from(tl.per_client);
    let sl = axi_lite::<32, 32, 4>();
    let ubus: LitePort<32, 32, 4> = sl.per.into();
    let (baw, bar, bw, bb, br) = sl.host;
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut dper = AxiPer::<32, 32, 4, IW>::default();
    let mut tper = AxiPer::<32, 32, 4, IW>::default();
    let mut ubridge = Serial::default();
    let mut router = Rtr::default();
    let (rst_t, rst_u) = (rst.clone(), rst.clone());
    let mut sim = Running::new(join2(
        join2(
            join2(
                timer.run(tbus, (rst_t, tirq_out, sirq_out)),
                uart.run(ubus, (rst_u, rx, tx_out, uirq_out)),
            ),
            join2(
                dmem.run(dbus, ()),
                pair.run(
                    (rst, irq, tirq, sirq, fault_in, crdata, cdone, grant),
                    (
                        halt_out,
                        instr_out,
                        wb_out,
                        issue,
                        wbeat,
                        release,
                        differs_out,
                    ),
                ),
            ),
        ),
        join2(
            join2(
                axi_host.run(cl.host_in, cl.host_out),
                router.run(
                    (
                        cl.per_in.0,
                        cl.per_in.1,
                        cl.per_in.2,
                        [dl.host_in.2, tl.host_in.2, ul.host_in.2],
                        [dl.host_in.3, tl.host_in.3, ul.host_in.3],
                    ),
                    (
                        [dl.host_out.0, tl.host_out.0, ul.host_out.0],
                        [dl.host_out.1, tl.host_out.1, ul.host_out.1],
                        [dl.host_out.2, tl.host_out.2, ul.host_out.2],
                        cl.per_out.2,
                        cl.per_out.3,
                    ),
                ),
            ),
            join2(
                join2(
                    dper.run(dl.per_in, dl.per_out),
                    tper.run(tl.per_in, tl.per_out),
                ),
                ubridge.run(
                    (ul.per_in.0, ul.per_in.1, ul.per_in.2, [bb], [br]),
                    ([baw], [bar], [bw], ul.per_out.2, ul.per_out.3),
                ),
            ),
        ),
    ));
    rst_out.set(Bit::One);
    irq_out.set(Bit::Zero);
    rx_out.set(Bit::One);
    fault_out.set(Bit::Zero);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut said = false;
    for c in 0..cycles {
        fault_out.set(Bit::from_bool(fault.contains(&c)));
        sim.cycle();
        said |= differs.get().to_bool();
    }
    (said, differs.get().to_bool())
}

#[test]
fn two_cores_given_the_same_run_agree() {
    for seed in 0..8u64 {
        let program = random(seed, 64);
        assert!(
            !pair_says(&program, 600, &[]).0,
            "the pair said its cores disagreed on seed {seed}"
        );
    }
}

#[test]
fn a_core_held_in_reset_a_cycle_longer_is_caught() {
    let program = random(1, 64);
    assert!(
        pair_says(&program, 600, &[20]).0,
        "the pair missed a core it held in reset for a cycle"
    );
}

#[test]
fn the_line_stays_up_after_a_disagreement() {
    // The fault is one cycle, twenty cycles in; the line is read at
    // the end of six hundred, so it has to have stayed up.
    let program = random(2, 64);
    let (said, at_end) = pair_says(&program, 600, &[20]);
    assert!(said, "the pair missed the fault");
    assert!(at_end, "the line came down before the end");
}
