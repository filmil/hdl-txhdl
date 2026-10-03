// SPDX-License-Identifier: Apache-2.0
//! An MDIO master on AXI-Lite, and a PHY on the other end of the line.
//!
//! A driver looks for its PHY the way a driver does: it reads the
//! first identifier register at each address until one answers with
//! something other than all ones, which is what a line nobody drives
//! reads through its pull-up. Then it reads both identifier words and
//! the basic status register. The PHY is a model, `MdioPhy`, stepped
//! between cycles from the loop; its identifier and status words are
//! the ones the JL2121 on the AX7A200B answered with (issue 864).
//!
//! The master is lowered, and the build simulates its netlist against
//! this run under nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LitePort, LiteW};
use txhdl_parts::mdio::sim::MdioPhy;
use txhdl_parts::mdio::MdioLines;
use txhdl_parts::mdio::{cmd, reg, Mdio};

type Host = LiteHost<32, 32, 4>;

/// Half of an MDC cycle in this run: two cycles, so a frame of 64 bits
/// takes 256.
const DIV: u32 = 1;

/// The PHY's address.
const ADDR: u8 = 1;

async fn poke(h: &Host, off: u32, word: u32) {
    let (aw, _, w, b, _) = h;
    aw.send(LiteAw {
        addr: U::from(off),
        prot: U::from(0u8),
    });
    w.send(LiteW {
        data: U::from(word),
        strb: U::from(0xfu8),
    });
    loop {
        DefaultClock::rising().await;
        if b.recv().is_some() {
            return;
        }
    }
}

async fn peek(h: &Host, off: u32) -> u32 {
    let (_, ar, _, _, r) = h;
    ar.send(LiteAw {
        addr: U::from(off),
        prot: U::from(0u8),
    });
    loop {
        DefaultClock::rising().await;
        if let Some(got) = r.recv() {
            return got.data.raw() as u32;
        }
    }
}

/// One read frame, and the word it took.
async fn read(h: &Host, phy: u8, r: u8) -> u16 {
    poke(h, reg::CMD, cmd::read(phy, r)).await;
    while peek(h, reg::STATE).await & 1 == 1 {}
    peek(h, reg::DATA).await as u16
}

fn main() {
    let link = axi_lite::<32, 32, 4>();
    let bus: LitePort<32, 32, 4> = link.per.into();
    let host = link.host;
    let (in_o, mdio_in) = signal::<Bit, DefaultClock>();
    let (mdc_o, mdc) = signal::<Bit, DefaultClock>();
    let (out_o, out) = signal::<Bit, DefaultClock>();
    let (oe_o, oe) = signal::<Bit, DefaultClock>();
    let mut master = Mdio::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("mdio_in", &mdio_in);
        wave.add("mdc", &mdc);
        wave.add("mdio_out", &out);
        wave.add("mdio_oe", &oe);
        wave.add("mdio", &master);
        wave.start();
    }

    let client = async move {
        let h = &host;
        poke(h, reg::CTRL, DIV).await;
        // The scan: the first address whose identifier is not all ones.
        let mut found = None;
        for phy in 0..4u8 {
            let id1 = read(h, phy, 2).await;
            println!("t={:>5} phy {phy}: register 2 reads {id1:#06x}", now());
            if id1 != 0xffff {
                found = Some(phy);
                break;
            }
        }
        let phy = found.expect("a PHY answered");
        assert_eq!(phy, ADDR);
        let id2 = read(h, phy, 3).await;
        println!("t={:>5} phy {phy}: register 3 reads {id2:#06x}", now());
        let status = read(h, phy, 1).await;
        println!("t={:>5} phy {phy}: register 1 reads {status:#06x}", now());
        println!("the PHY found at address {phy}, and read");
    };

    let mut sim = Running::new(join2(
        client,
        master.run(
            bus,
            MdioLines {
                mdio_in,
                mdc: mdc_o,
                mdio_out: out_o,
                mdio_oe: oe_o,
            },
        ),
    ));
    let mut regs = [0u16; 32];
    regs[1] = 0x796d;
    regs[2] = 0x937c;
    regs[3] = 0x4032;
    let mut phy = MdioPhy::new(ADDR, regs);
    in_o.set(Bit::One);
    for _ in 0..1600 {
        sim.cycle();
        // The line reads what the master drives, else what the PHY
        // drives, else one through the pull-up.
        let (c, m, e) =
            (mdc.get().to_bool(), out.get().to_bool(), oe.get().to_bool());
        let line = |p: &MdioPhy| if e { m } else { p.drives().unwrap_or(true) };
        let seen = line(&phy);
        phy.step(c, seen);
        in_o.set(Bit::from_bool(line(&phy)));
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&Mdio::lowered("mdio")]);
}
