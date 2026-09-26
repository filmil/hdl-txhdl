// SPDX-License-Identifier: Apache-2.0
//! Four pulse width modulated outputs behind AXI-Lite: a period, a
//! duty per channel, polarity, and pulses centred or aligned at their
//! left edge.
//!
//! A host client on an AXI4 link reaches the peripheral through the
//! AXI-Lite bridge and does what a program driving LEDs would. It sets
//! a period of ten cycles and three duties, turns the counter on, and
//! watches the pins: the run counts the cycles each channel is high
//! over three periods, which is the duty. It then widens a duty in the
//! middle of a period and counts again, which shows the shadow: the
//! period under way keeps the width it started with, so no pulse is
//! of a width nobody asked for. Last it turns the counter over with a
//! polarity bit and centres the pulses, which makes one run in the
//! middle of a period rather than two at its ends.
//!
//! The peripheral is lowered, and the build simulates its netlist
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::U;
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::pwm::{duty, polarity, Pwm, CTRL_CENTRE, CTRL_ENABLE};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge<1, PwmMap, 32, 32, 4, 2>;

/// Where the bridge's one peripheral is: a nibble of the address
/// space at 0x1000.
pub struct PwmMap;

impl AddrMap<1> for PwmMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

/// The peripheral's words, as the host addresses them.
const BASE: u32 = 0x1000;
const CTRL: u32 = BASE;
const PERIOD: u32 = BASE + 4;

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
    let (pins_out, pins) = signal::<U<4>, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut pwm = Pwm::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("pins", &pins);
        wave.add("pwm", &pwm);
        wave.start();
    }

    // What the pins did, cycle by cycle, as the run recorded it.
    let seen: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let watched = pins.clone();
    let log = seen.clone();

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let put = |a: u32| Wr::at(a);
        let ok = host.write(put(PERIOD), &word(10)).await.done().await;
        assert_eq!(ok.resp, Resp::Okay, "the write was answered");
        host.write(put(BASE + duty(0)), &word(2)).await.done().await;
        host.write(put(BASE + duty(1)), &word(5)).await.done().await;
        host.write(put(BASE + duty(2)), &word(9)).await.done().await;
        let mark = |log: &Rc<RefCell<Vec<u32>>>| log.borrow().len();
        host.write(put(CTRL), &word(CTRL_ENABLE)).await.done().await;
        let from = mark(&log);
        for _ in 0..30 {
            DefaultClock::rising().await;
        }
        let over = |log: &Rc<RefCell<Vec<u32>>>, from: usize, i: u32| {
            log.borrow()[from..]
                .iter()
                .filter(|v| (*v >> i) & 1 == 1)
                .count()
        };
        println!(
            "{:3}  duties 2, 5, 9 of 10: high {}, {}, {} cycles in {}",
            now(),
            over(&log, from, 0),
            over(&log, from, 1),
            over(&log, from, 2),
            log.borrow().len() - from
        );

        // A wider duty, written in the middle of a period.
        host.write(put(BASE + duty(0)), &word(8)).await.done().await;
        let from = mark(&log);
        for _ in 0..30 {
            DefaultClock::rising().await;
        }
        let widths = |log: &Rc<RefCell<Vec<u32>>>, from: usize, i: u32| {
            let mut out: Vec<usize> = Vec::new();
            let mut run = 0;
            for v in log.borrow()[from..].iter() {
                if (*v >> i) & 1 == 1 {
                    run += 1;
                } else if run > 0 {
                    out.push(run);
                    run = 0;
                }
            }
            out
        };
        let runs = widths(&log, from, 0);
        println!("{:3}  after a write of 8: pulses {:?}", now(), runs);
        for w in runs.iter().skip(1) {
            assert!(*w == 2 || *w == 8, "a pulse of a width nobody asked for");
        }

        // Turned over: channel 1 at three of ten, inverted, is high
        // for the other seven.
        host.write(put(BASE + duty(1)), &word(3)).await.done().await;
        host.write(put(CTRL), &word(CTRL_ENABLE | polarity(1)))
            .await
            .done()
            .await;
        let from = mark(&log);
        for _ in 0..20 {
            DefaultClock::rising().await;
        }
        println!(
            "{:3}  channel 1 turned over: high {} cycles in {}",
            now(),
            over(&log, from, 1),
            log.borrow().len() - from
        );
        // Centred: channel 2 at four of ten is one run of eight in a
        // period of twenty, since the counter goes up and back down.
        host.write(put(BASE + duty(2)), &word(4)).await.done().await;
        host.write(put(CTRL), &word(CTRL_ENABLE | CTRL_CENTRE))
            .await
            .done()
            .await;
        let from = mark(&log);
        for _ in 0..60 {
            DefaultClock::rising().await;
        }
        println!(
            "{:3}  centred: channel 2's pulses {:?}",
            now(),
            widths(&log, from, 2)
        );
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
        ),
        join2(pwm.run(bus, pins_out), client),
    ));
    println!("  t  what the program saw");
    for _ in 0..400 {
        sim.cycle();
        seen.borrow_mut().push(watched.get().raw() as u32);
    }
    stop();
    let net = Pwm::lowered("pwm");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
