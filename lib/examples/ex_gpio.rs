// SPDX-License-Identifier: Apache-2.0
//! General purpose pins behind AXI-Lite: eight of them, four driven
//! and four read, with an interrupt on one.
//!
//! A host client on an AXI4 link reaches the peripheral through the
//! AXI-Lite bridge and does what a bring-up program does. It makes the
//! low four pins outputs and drives a pattern on them. It reads the
//! high four back as the testbench moves them; the answer is the
//! synchroniser's, two flip-flops behind the pin, which a program
//! never notices because a bus round trip is longer than two cycles.
//! It asks for an interrupt
//! on the rising edge of one pin, watches the line rise when that pin
//! rises and not when it falls, clears the status by writing a one,
//! and watches the line fall. Then it asks for a level instead, and
//! sees the bit set itself again in the cycle after it is cleared,
//! because the level is still there.
//!
//! The peripheral is lowered, and the build simulates its netlist
//! against this run under nvc and Verilator.
use std::cell::RefCell;
use std::rc::Rc;
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, Clock, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::gpio::Gpio;

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the peripheral at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// Eight pins.
type Pins = Gpio<8>;

/// The peripheral's words, as offsets from its base.
const OUT: u32 = 0x1000;
const IN: u32 = 0x1004;
const DIR: u32 = 0x1008;
const IE: u32 = 0x100c;
const KIND: u32 = 0x1010;
const POL: u32 = 0x1014;
const STATUS: u32 = 0x1018;

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
    let (paw, par, pw, pb, pr) = lite.per;
    let (pins_drive, pins) = signal::<U<8>, DefaultClock>();
    let (drive_out, drive) = signal::<U<8>, DefaultClock>();
    let (dirs_out, dirs) = signal::<U<8>, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut gpio = Pins::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        wave.add("pins", &pins);
        wave.add("drive", &drive);
        wave.add("dirs", &dirs);
        wave.add("irq", &irq);
        wave.add("gpio", &gpio);
        wave.start();
    }

    // What the testbench holds on the pins. The client moves it, and
    // the loop below puts it on the wire between one cycle and the
    // next, which is where a switch or another chip would move it: a
    // process that set it during the cycle would race the peripheral
    // reading it, and the netlist, whose port is settled before the
    // edge, would then disagree with the run.
    let held = Rc::new(RefCell::new(0u32));

    let moved = held.clone();
    let seen = irq.clone();
    let out_line = drive.clone();
    let dir_line = dirs.clone();
    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let put = |a: u32| Wr::at(a);
        // Four outputs, four inputs, and a pattern on the outputs.
        let dir = host.write(put(DIR), &word(0x0f)).await.done().await;
        assert_eq!(dir.resp, Resp::Okay, "the write was answered");
        host.write(put(OUT), &word(0x0a)).await.done().await;
        println!(
            "{:3}  drive {:#04x}  dir {:#04x}",
            now(),
            out_line.get().raw(),
            dir_line.get().raw()
        );

        // A pin moves, and the answer is the synchroniser's.
        *moved.borrow_mut() = 0x10;
        let read = host.read(Rd::at(IN, 1)).await.done().await;
        println!("{:3}  in    {:#04x}", now(), read.data[0].raw());
        assert_eq!(read.resp, Resp::Okay, "the read was answered");
        assert_eq!(read.data[0].raw(), 0x10, "the pin reads back");

        // An interrupt on the rising edge of pin 4.
        host.write(put(STATUS), &word(0xff)).await.done().await;
        host.write(put(KIND), &word(0x10)).await.done().await;
        host.write(put(POL), &word(0x10)).await.done().await;
        host.write(put(IE), &word(0x10)).await.done().await;
        *moved.borrow_mut() = 0x00;
        let fell = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  fell  status {:#04x}  irq {}",
            now(),
            fell.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(fell.data[0].raw(), 0, "a fall is not a rise");
        *moved.borrow_mut() = 0x10;
        let rose = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  rose  status {:#04x}  irq {}",
            now(),
            rose.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(rose.data[0].raw(), 0x10, "a rise fires");
        assert_eq!(seen.get().to_bool() as u8, 1, "and raises the line");

        // A one clears it, and the edge has gone.
        host.write(put(STATUS), &word(0x10)).await.done().await;
        let done = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  clear status {:#04x}  irq {}",
            now(),
            done.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(done.data[0].raw(), 0, "the write cleared it");
        assert_eq!(seen.get().to_bool() as u8, 0, "and the line fell");

        // A level instead: the pin is still high, so the bit comes
        // back in the cycle after it is cleared.
        host.write(put(KIND), &word(0x00)).await.done().await;
        host.write(put(STATUS), &word(0x10)).await.done().await;
        let level = host.read(Rd::at(STATUS, 1)).await.done().await;
        println!(
            "{:3}  level status {:#04x}  irq {}",
            now(),
            level.data[0].raw(),
            seen.get().to_bool() as u8
        );
        assert_eq!(level.data[0].raw(), 0x10, "a level does not clear");
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            gpio.run(
                (paw, par, pw, pins),
                (pb, pr, drive_out, dirs_out, irq_out),
            ),
            client,
        ),
    ));
    println!("  t  what the program saw");
    for _ in 0..220 {
        sim.cycle();
        pins_drive.set(U::from(*held.borrow()));
    }
    stop();
    let net = Pins::lowered("gpio");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
