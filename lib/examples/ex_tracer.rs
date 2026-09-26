// SPDX-License-Identifier: Apache-2.0
//! A trace buffer, filled while a machine runs and read out after it
//! has stopped.
//!
//! The run offers the buffer one entry a cycle, as a core offers the
//! instruction it retired, and the entries here are a made-up program
//! counter and word so that the readout is easy to check. More is
//! offered than the ring holds, so the window is the most recent
//! eight; then a `halt` line rises, as it would when a core stops on
//! a breakpoint, and the buffer freezes itself rather than filling the
//! window with whatever happened next.
//!
//! A host client on an AXI4 link then reads the window out through the
//! AXI-Lite bridge, oldest first, and prints it. The readout says what
//! a post-mortem wants: the last eight things the machine did before
//! it stopped, and nothing after.
//!
//! The peripheral is lowered, and the build simulates its netlist
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::tracer::{regs, word, Tracer, CTRL_FREEZE, CTRL_RUN};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge<1, TracerMap, 32, 32, 4, 2>;

/// Where the bridge's one peripheral is: a nibble of the address
/// space at 0x1000.
pub struct TracerMap;

impl AddrMap<1> for TracerMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

/// A ring of eight entries.
type Ring = Tracer<8>;

/// The peripheral's words, as the host addresses them.
const BASE: u32 = 0x1000;
const CTRL: u32 = BASE + regs::ctrl;
const COUNT: u32 = BASE + regs::count;
const CURSOR: u32 = BASE + regs::cursor;

fn main() {
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<32, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    let lite = axi_lite::<32, 32, 4>();
    let (law, lar, lw, lb, lr) = lite.host;
    let bus: LitePort<32, 32, 4> = lite.per.into();
    let (take_out, take) = signal::<Bit, DefaultClock>();
    let (entry_out, entry) = signal::<U<128>, DefaultClock>();
    let (halt_out, halt) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut tracer = Ring::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("take", &take);
        wave.add("entry", &entry);
        wave.add("halt", &halt);
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("tracer", &tracer);
        wave.start();
    }

    // What the machine being watched does, cycle by cycle: a program
    // counter going up by four and the word it wrote. The testbench
    // holds them and the loop below puts them on the wires between one
    // cycle and the next, where a design would drive them.
    let offered: Rc<RefCell<(bool, u128, bool)>> =
        Rc::new(RefCell::new((false, 0, false)));
    let driven = offered.clone();

    let machine = offered.clone();
    let client = async move {
        let word32 = |v: u32| [U::<32>::from(v)];
        let put = |a: u32| Wr::at(a);
        // Start the buffer, and let it freeze itself on the halt.
        let ok = host
            .write(put(CTRL), &word32(CTRL_RUN | CTRL_FREEZE))
            .await
            .done()
            .await;
        assert_eq!(ok.resp, Resp::Okay, "the write was answered");
        // Twelve retirements into a ring of eight.
        for i in 0..12u32 {
            let pc = 0x100 + 4 * i;
            *machine.borrow_mut() =
                (true, pc as u128 | ((i as u128) << 64), false);
            DefaultClock::rising().await;
        }
        *machine.borrow_mut() = (false, 0, true);
        DefaultClock::rising().await;
        // Four more the buffer must not take, since it has frozen.
        for i in 0..4u32 {
            *machine.borrow_mut() = (true, 0xdead_0000 + i as u128, true);
            DefaultClock::rising().await;
        }
        *machine.borrow_mut() = (false, 0, true);

        let held = host.read(Rd::at(COUNT, 1)).await.done().await;
        let ctrl = host.read(Rd::at(CTRL, 1)).await.done().await;
        println!(
            "{:3}  held {}  ctrl {:#04x}, so the run bit is {}",
            now(),
            held.data[0].raw(),
            ctrl.data[0].raw(),
            ctrl.data[0].raw() & CTRL_RUN as u128
        );
        assert_eq!(held.data[0].raw(), 8, "the ring holds eight");
        assert_eq!(ctrl.data[0].raw() & CTRL_RUN as u128, 0, "and has stopped");

        println!("  i  pc      n");
        for i in 0..8u32 {
            host.write(put(CURSOR), &word32(i)).await.done().await;
            let lo = host.read(Rd::at(BASE + word(0), 1)).await.done().await;
            let hi = host.read(Rd::at(BASE + word(2), 1)).await.done().await;
            println!(
                "{:3}  {:#06x}  {}",
                i,
                lo.data[0].raw(),
                hi.data[0].raw()
            );
            // Twelve were offered and eight are held, so the oldest is
            // the fifth of them.
            assert_eq!(hi.data[0].raw() as u32, 4 + i, "oldest first");
        }
    };

    let mut sim = Running::new(join2(
        join2(
            client,
            join2(
                host_unit.run(host_in, host_out),
                bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
            ),
        ),
        tracer.run(bus, (take, entry, halt)),
    ));
    println!("  t  what the program saw");
    for _ in 0..300 {
        sim.cycle();
        let (t, v, h) = *driven.borrow();
        take_out.set(Bit::from_bool(t));
        entry_out.set(U::<128>::from(v));
        halt_out.set(Bit::from_bool(h));
    }
    stop();
    let net = Ring::lowered("tracer");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
