// SPDX-License-Identifier: Apache-2.0
//! The entropy source's adaptive proportion test, tripped (issue 918).
//!
//! The peripheral is fed a source four parts in five one, with no run
//! of the same sample longer than four: biased, and never stuck, so
//! the repetition count test never sees it. The adaptive proportion
//! test of SP 800-90B, section 4.4.2, counts in each window of 1024
//! samples how many equal the window's first; a window that starts on
//! a one counts about 819, over the cutoff of 793, and the test raises
//! its fault. A client on the AXI-Lite link turns the source on, waits
//! for the fault, prints `status`, and clears it.
//!
//! The peripheral is lowered, and the build simulates its netlist
//! against this run under nvc and Verilator, so the netlist's test is
//! checked through the trip, register by register.
use std::cell::Cell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LitePort, LiteW};
use txhdl_parts::trng::{
    Trng, APT_CUTOFF, APT_WINDOW, CTRL, CTRL_CLEAR, CTRL_RUN, RINGS, STATUS,
    STATUS_APTFAULT, STATUS_FAULT,
};

type Host = LiteHost<32, 32, 4>;

async fn write(h: &Host, addr: u32, data: u32) {
    let (aw, _, w, b, _) = h;
    aw.send(LiteAw {
        addr: U::from(addr),
        prot: U::from(0u8),
    });
    w.send(LiteW {
        data: U::from(data),
        strb: U::from(0xfu8),
    });
    loop {
        DefaultClock::rising().await;
        if b.recv().is_some() {
            return;
        }
    }
}

async fn read(h: &Host, addr: u32) -> u32 {
    let (_, ar, _, _, r) = h;
    ar.send(LiteAw {
        addr: U::from(addr),
        prot: U::from(0u8),
    });
    loop {
        DefaultClock::rising().await;
        if let Some(got) = r.recv() {
            return got.data.raw() as u32;
        }
    }
}

fn main() {
    let lite = axi_lite::<32, 32, 4>();
    let host = lite.host;
    let bus: LitePort<32, 32, 4> = lite.per.into();
    let (raw_o, raw) = signal::<U<RINGS>, DefaultClock>();
    let (en_o, en) = signal::<Bit, DefaultClock>();
    let mut trng = Trng::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("raw", &raw);
        wave.add("en", &en);
        wave.add("trng", &trng);
        wave.start();
    }

    // The source: one ring set, so the rings XOR to one, for four
    // cycles in five, and none set the fifth.
    let source = async move {
        let mut t = 0u64;
        loop {
            DefaultClock::rising().await;
            raw_o.set(U::<RINGS>::from(u32::from(t % 5 < 4)));
            t += 1;
        }
    };

    let done = Rc::new(Cell::new(false));
    let fin = done.clone();
    let client = async move {
        write(&host, CTRL, CTRL_RUN).await;
        let mut s = read(&host, STATUS).await;
        while s & STATUS_APTFAULT == 0 {
            s = read(&host, STATUS).await;
        }
        println!(
            "{:5}  status {s:#06x}: the proportion test tripped, the \
             repetition count test did not",
            now()
        );
        assert_eq!(s & STATUS_FAULT, 0, "no run reached the other cutoff");
        write(&host, CTRL, CTRL_RUN | CTRL_CLEAR).await;
        let s = read(&host, STATUS).await;
        println!("{:5}  status {s:#06x}: cleared", now());
        assert_eq!(s & STATUS_APTFAULT, 0, "the clear takes");
        fin.set(true);
    };

    // The source first, so that the peripheral reads the sample set
    // in the same step.
    let mut sim =
        Running::new(join2(join2(source, trng.run(bus, (raw, en_o))), client));
    println!(
        "    t  window {APT_WINDOW} samples, cutoff {APT_CUTOFF} equal to \
         the first"
    );
    for _ in 0..20_000 {
        sim.cycle();
        if done.get() {
            break;
        }
    }
    assert!(done.get(), "the test tripped within the ceiling");
    stop();
    let net = Trng::lowered("trng_apt");
    txhdl::netlist::write_netlists_from_env(&[&net]);
}
