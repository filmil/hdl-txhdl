// SPDX-License-Identifier: Apache-2.0
//! A platform-level interrupt controller of three sources, driven the
//! way an interrupt handler drives one. Sources 1 and 3 ask while
//! their line is high, and source 2 asks on a rising edge. A host on
//! the AXI-Lite link sets the priorities and the enable bits, raises
//! the lines, and claims and completes the requests: the highest
//! priority first, the lowest number winning a tie, nothing at or
//! below the threshold, a level source still high asking again after
//! its complete, and an edge that came during a service asking once
//! on the complete. The controller is lowered, and the build simulates
//! its netlist against this run under nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
use txhdl_parts::plic::{priority, Plic3, CLAIM, ENABLE, PENDING, THRESHOLD};

/// Three sources; the second asks on an edge.
type Plic = Plic3<0b100>;

type Host = LiteHost<32, 32, 4>;

/// A write of one word, and the wait for its response.
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

/// A read of one word.
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

async fn cycles(n: usize) {
    for _ in 0..n {
        DefaultClock::rising().await;
    }
}

fn main() {
    // What `plic!` wrote for three sources, for the document to show
    // without a hand-typed copy.
    if std::env::args().any(|a| a == "--source") {
        print!("{}", txhdl_parts::plic::plic3::SOURCE);
        return;
    }
    let link = axi_lite::<32, 32, 4>();
    let (aw, ar, w, b, r) = link.per;
    let host = link.host;
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (src1_o, src1) = signal::<Bit, DefaultClock>();
    let (src2_o, src2) = signal::<Bit, DefaultClock>();
    let (src3_o, src3) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let mut plic = Plic::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("rst", &rst);
        wave.add("src1", &src1);
        wave.add("src2", &src2);
        wave.add("src3", &src3);
        wave.add("aw", &aw);
        wave.add("ar", &ar);
        wave.add("w", &w);
        wave.add("b", &b);
        wave.add("r", &r);
        wave.add("irq", &irq);
        wave.add("plic", &plic);
        wave.start();
    }

    let line = move |i: usize, high: bool| {
        let bit = Bit::from_bool(high);
        match i {
            1 => src1_o.set(bit),
            2 => src2_o.set(bit),
            _ => src3_o.set(bit),
        }
    };
    let irq_line = irq.clone();
    let client = async move {
        let h = &host;
        let say = |what: &str, v: u32| {
            let line = irq_line.get().to_bool() as u8;
            println!("t={:>3} irq={line} {what} {v:#x}", now());
        };
        // Priorities 1, 3 and 3, all three enabled, a threshold of 0.
        write(h, priority(1), 1).await;
        write(h, priority(2), 3).await;
        write(h, priority(3), 3).await;
        write(h, ENABLE, 0xe).await;
        // All three ask: the priority decides, then the number.
        for i in 1..=3 {
            line(i, true);
        }
        cycles(3).await;
        say("pending", read(h, PENDING).await);
        let order = [
            read(h, CLAIM).await,
            read(h, CLAIM).await,
            read(h, CLAIM).await,
        ];
        println!("t={:>3} claimed in order {order:?}", now());
        assert_eq!(order, [2, 3, 1]);
        assert_eq!(read(h, CLAIM).await, 0, "all three in service");
        // The completes. Sources 1 and 3 are still high, and ask again;
        // source 2 had one edge, and does not.
        line(3, false);
        for i in 1..=3 {
            write(h, CLAIM, i).await;
        }
        cycles(3).await;
        say("pending after the completes", read(h, PENDING).await);
        assert_eq!(read(h, PENDING).await, 0x2);
        // The threshold at 1 hides source 1's priority of 1.
        write(h, THRESHOLD, 1).await;
        cycles(2).await;
        let got = read(h, CLAIM).await;
        say("claim at threshold 1", got);
        assert_eq!(got, 0);
        write(h, THRESHOLD, 0).await;
        assert_eq!(read(h, CLAIM).await, 1);
        line(1, false);
        write(h, CLAIM, 1).await;
        // A pulse on the edge source, claimed; a second pulse while it
        // is in service is held, and asks on the complete.
        line(2, false);
        cycles(2).await;
        line(2, true);
        cycles(1).await;
        line(2, false);
        cycles(3).await;
        assert_eq!(read(h, CLAIM).await, 2);
        line(2, true);
        cycles(1).await;
        line(2, false);
        cycles(3).await;
        say("pending with an edge held", read(h, PENDING).await);
        write(h, CLAIM, 2).await;
        cycles(3).await;
        say("pending after its complete", read(h, PENDING).await);
        assert_eq!(read(h, CLAIM).await, 2);
        write(h, CLAIM, 2).await;
        cycles(3).await;
        say("pending at the end", read(h, PENDING).await);
        assert!(!irq_line.get().to_bool());
        println!("every request claimed once, in order");
    };

    // The client first: it drives the source lines, which are wires,
    // and the controller reads them in the same step.
    let hardware = plic.run((rst, src1, src2, src3, aw, ar, w), (b, r, irq_o));
    let mut sim = Running::new(join2(client, hardware));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    for _ in 0..220 {
        sim.cycle();
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&Plic::lowered("plic3")]);
}
