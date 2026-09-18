// SPDX-License-Identifier: Apache-2.0
//! An I2C master on AXI-Lite, and a device on the other end of the two
//! lines.
//!
//! A driver does what a driver does: it addresses the device, writes
//! the register it wants, turns the bus around with a repeated start,
//! and reads the byte back. The device is a model, `I2cDev`, stepped
//! between cycles from the loop: it acknowledges its own address,
//! keeps a byte per register, and holds the clock low for a few cycles
//! after each byte, which is clock stretching and which the master
//! waits for rather than counting through.
//!
//! The master is lowered, and the build simulates its netlist against
//! this run under nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
use txhdl_parts::i2c::sim::I2cDev;
use txhdl_parts::i2c::{cmd, reg, I2c};

type Host = LiteHost<32, 32, 4>;

/// A quarter of a bit in this run: four cycles, so a byte and its
/// acknowledge take about a hundred and fifty.
const DIV: u32 = 3;

/// The device's address, and the registers it starts with.
const ADDR: u8 = 0x50;
const REGS: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

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

/// One command, and the wait for it: the state word once the master is
/// no longer busy.
async fn command(h: &Host, word: u32) -> u32 {
    poke(h, reg::CMD, word).await;
    loop {
        let state = peek(h, reg::STATE).await;
        if state & 1 == 0 {
            return state;
        }
    }
}

fn main() {
    let link = axi_lite::<32, 32, 4>();
    let (aw, ar, w, b, r) = link.per;
    let host = link.host;
    let (scl_in_o, scl_in) = signal::<Bit, DefaultClock>();
    let (sda_in_o, sda_in) = signal::<Bit, DefaultClock>();
    let (scl_low_o, scl_low) = signal::<Bit, DefaultClock>();
    let (sda_low_o, sda_low) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let mut master = I2c::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &aw);
        wave.add("ar", &ar);
        wave.add("w", &w);
        wave.add("b", &b);
        wave.add("r", &r);
        wave.add("scl_in", &scl_in);
        wave.add("sda_in", &sda_in);
        wave.add("scl_low", &scl_low);
        wave.add("sda_low", &sda_low);
        wave.add("irq", &irq);
        wave.add("i2c", &master);
        wave.start();
    }

    let client = async move {
        let h = &host;
        // A quarter of a bit, and the interrupt on.
        poke(h, reg::CTRL, DIV | (1 << 16)).await;
        // The address, with the write bit clear.
        let state =
            command(h, cmd::START | cmd::WRITE | cmd::byte(ADDR << 1)).await;
        println!("t={:>4} addressed {ADDR:#04x}, state {state:#06b}", now());
        // The register to read from.
        command(h, cmd::WRITE | cmd::byte(2)).await;
        println!("t={:>4} pointed at register 2", now());
        // The repeated start turns the bus around, and the last byte a
        // master wants is answered with a NACK.
        command(h, cmd::START | cmd::WRITE | cmd::byte((ADDR << 1) | 1)).await;
        let state = command(h, cmd::READ | cmd::NACK | cmd::STOP).await;
        let byte = peek(h, reg::DATA).await;
        println!(
            "t={:>4} read {byte:#04x} from register 2, state {state:#06b}",
            now()
        );
        assert_eq!(byte as u8, REGS[2]);
        // A device that is not there acknowledges nothing.
        let state =
            command(h, cmd::START | cmd::WRITE | cmd::STOP | cmd::byte(0x40))
                .await;
        println!("t={:>4} nobody at 0x20, state {state:#06b}", now());
        assert_eq!(state & 4, 4, "not acknowledged");
        println!("a register read, and an address nobody answered");
    };

    let mut sim = Running::new(join2(
        client,
        master.run(
            (aw, ar, w, scl_in, sda_in),
            (b, r, scl_low_o, sda_low_o, irq_o),
        ),
    ));
    let mut dev = I2cDev::new(ADDR, &REGS);
    dev.stretch = 3;
    scl_in_o.set(Bit::One);
    sda_in_o.set(Bit::One);
    for _ in 0..3000 {
        sim.cycle();
        // The device sees the master's pulls, and the two lines are
        // open drain: high unless somebody pulls them.
        let (sc, sd) = (scl_low.get().to_bool(), sda_low.get().to_bool());
        dev.step(sc, sd);
        scl_in_o.set(Bit::from_bool(!sc && !dev.pulls_scl()));
        sda_in_o.set(Bit::from_bool(!sd && !dev.pulls_sda()));
    }
    stop();
    txhdl::netlist::write_netlists_from_env(&[&I2c::lowered("i2c")]);
}
