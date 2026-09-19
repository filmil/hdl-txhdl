// SPDX-License-Identifier: Apache-2.0
//! A read-only window onto an SPI flash, read as ordinary memory.
//!
//! `FlashWin` is the other way of reaching the chip that `ex_spi`
//! drives byte by byte. Here a program issues a read at an address and
//! gets a word, and the nine bytes on the wires, the fast read command,
//! three of address, one thrown away and four of data, happen behind
//! it. Nothing has to be sequenced by software, which is what lets a
//! bootloader, or a core fetching from the bus, run from the flash.
//!
//! The chip is `FlashDevice`, the model that checks `ex_spi`, stepped
//! between cycles from the loop below. The run reads two words, checks
//! them against what the model holds, and then writes, which the
//! window refuses: it is read only, and that is what makes it safe to
//! fetch from.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{
    axi_units, host_end, AxiHost, AxiPer, Rd, Resp, Wr,
};
use txhdl_parts::flashwin::FlashWin;
use txhdl_parts::spi::FlashDevice;

/// The identifier width, and how many a host may have out at once.
const IW: usize = 2;
const NIDS: usize = 4;
/// A half of a bit takes two cycles, so the flash is clocked at a
/// quarter of the system's rate. A board would divide further.
const DIV: usize = 1;
/// The mode the window drives, which is the one a fast read wants.
const CPOL: bool = false;
const CPHA: bool = false;
/// Where the words are, and what they are.
const AT: u32 = 0x40;
const WORDS: [u32; 2] = [0xdead_beef, 0x0123_4567];

fn main() {
    let u = axi_units::<32, 32, 4, IW>();
    let host = host_end::<32, 32, 4, IW, NIDS>(u.host_client);
    let (req, wd, ans, rb) = u.per_client;
    let (miso_drive, miso) = signal::<Bit, DefaultClock>();
    let (sclk_out, sclk) = signal::<Bit, DefaultClock>();
    let (mosi_out, mosi) = signal::<Bit, DefaultClock>();
    let (cs_out, cs_n) = signal::<Bit, DefaultClock>();
    let (rst_out, rst) = signal::<Bit, DefaultClock>();

    let mut host_unit = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut per_unit = AxiPer::<32, 32, 4, IW>::default();
    let mut win = FlashWin::<DIV, IW>::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("req", &req);
        wave.add("wd", &wd);
        wave.add("ans", &ans);
        wave.add("rb", &rb);
        wave.add("rst", &rst);
        wave.add("miso", &miso);
        wave.add("sclk", &sclk);
        wave.add("mosi", &mosi);
        wave.add("cs_n", &cs_n);
        wave.add("win", &win);
        wave.start();
    }

    let client = async move {
        let first = host.read(Rd::at(AT, 1)).await.done().await;
        println!("{:3}  read  {:08x}", now(), first.data[0].raw());
        assert_eq!(first.resp, Resp::Okay, "the first read was answered");
        assert_eq!(first.data[0].raw() as u32, WORDS[0], "the first word");
        let second = host.read(Rd::at(AT + 4, 1)).await.done().await;
        println!("{:3}  read  {:08x}", now(), second.data[0].raw());
        assert_eq!(second.data[0].raw() as u32, WORDS[1], "the second word");
        // The window is read only, and says so rather than pretending.
        let wrote = host
            .write(Wr::at(AT), &[U::<32>::from(0u32)])
            .await
            .done()
            .await;
        println!("{:3}  write {:?}", now(), wrote.resp);
        assert_eq!(wrote.resp, Resp::SlvErr, "a write is refused");
    };

    let mut sim = Running::new(txhdl::comp::join2(
        txhdl::comp::join2(
            host_unit.run(u.host_in, u.host_out),
            per_unit.run(u.per_in, u.per_out),
        ),
        txhdl::comp::join2(
            win.run(
                (rst, req, wd, miso),
                (ans, rb, sclk_out, mosi_out, cs_out),
            ),
            client,
        ),
    ));

    // The chip holds the two words at `AT`, lowest byte first, which
    // is how the window puts them back together.
    let mut image = vec![0u8; AT as usize + 8];
    for (k, w) in WORDS.iter().enumerate() {
        image[AT as usize + k * 4..AT as usize + k * 4 + 4]
            .copy_from_slice(&w.to_le_bytes());
    }
    let mut chip = FlashDevice::new(image, [0x20, 0xba, 0x18], CPOL, CPHA);

    println!("  t  what the program saw");
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    for _ in 0..4000 {
        sim.cycle();
        chip.step(
            cs_n.get().to_bool(),
            sclk.get().to_bool(),
            mosi.get().to_bool(),
        );
        miso_drive.set(Bit::from_bool(chip.miso()));
    }
    stop();
    let net = FlashWin::<DIV, IW>::lowered("flashwin");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
