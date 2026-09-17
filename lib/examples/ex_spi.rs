// SPDX-License-Identifier: Apache-2.0
//! An SPI master on AXI-Lite, talking to a model of a flash chip.
//!
//! A host client on an AXI4 link reaches the master through the
//! AXI-Lite bridge and does what a bootloader would do before anything
//! else: it asks the chip who it is, and then reads four bytes from an
//! address.
//!
//! Both are the same shape. Hold the chip select, write a byte, wait
//! for the transfer to finish, read the byte that came back the other
//! way, repeat, release the select. SPI has no direction: the eight
//! clocks that send a byte are the eight that receive one, so the
//! answer to a command byte arrives while the next byte is being sent,
//! and a driver counts bytes rather than waiting for a reply.
//!
//! The identify command is `0x9f` and three bytes come back. The fast
//! read is `0x0b`, three bytes of address and one thrown away, and
//! then as many as are clocked. The chip on the other side is
//! `FlashDevice`, a model, stepped between cycles from the loop.
//!
//! The master is lowered, and the build simulates its netlist against
//! this run under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Host, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1};
use txhdl_parts::spi::{FlashDevice, Spi};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the master at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

/// The master's words.
const CTRL: u32 = 0x1000;
const DATA: u32 = 0x1004;
const STATE: u32 = 0x1008;

/// A half of a bit is two cycles, so a byte is thirty-two.
const DIV: u32 = 1;
/// Mode zero: the clock idles low and the leading edge carries the bit.
const CPOL: bool = false;
const CPHA: bool = false;
/// The control word with the chip held, and with it released.
const HELD: u32 = DIV | (1 << 10);
const FREE: u32 = DIV;

/// The client that drives the master.
type Client = Host<32, 32, 4, 2, 4>;

/// One byte out and one byte in, which on SPI is the same eight
/// clocks: write the byte, wait for the transfer to end, and take what
/// came the other way. The caller holds the chip select around a whole
/// command, because a command is several of these.
async fn swap(host: &Client, byte: u32) -> u8 {
    let out = [U::<32>::from(byte)];
    host.write(Wr::at(DATA), &out).await.done().await;
    loop {
        let st = host.read(Rd::at(STATE, 1)).await.done().await.data[0];
        if st.raw() & 1 == 0 {
            break;
        }
    }
    let got = host.read(Rd::at(DATA, 1)).await.done().await.data[0];
    (got.raw() & 0xff) as u8
}

/// What the chip says it is.
const ID: [u8; 3] = [0xef, 0x40, 0x18];
/// Where the program the bootloader would want begins, and what is
/// there.
const AT: u32 = 0x00_1234;
const STORED: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

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
    let (miso_drive, miso) = signal::<Bit, DefaultClock>();
    let (sclk_out, sclk) = signal::<Bit, DefaultClock>();
    let (mosi_out, mosi) = signal::<Bit, DefaultClock>();
    let (cs_out, cs_n) = signal::<Bit, DefaultClock>();
    let (irq_out, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut spi = Spi::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("aw", &paw);
        wave.add("ar", &par);
        wave.add("w", &pw);
        wave.add("b", &pb);
        wave.add("r", &pr);
        wave.add("miso", &miso);
        wave.add("sclk", &sclk);
        wave.add("mosi", &mosi);
        wave.add("cs_n", &cs_n);
        wave.add("irq", &irq);
        wave.add("spi", &spi);
        wave.start();
    }

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let held = host.write(Wr::at(CTRL), &word(HELD)).await.done().await;
        assert_eq!(held.resp, Resp::Okay, "the control write was answered");
        let mut id = [0u8; 3];
        swap(&host, 0x9f).await;
        for slot in id.iter_mut() {
            *slot = swap(&host, 0x00).await;
        }
        host.write(Wr::at(CTRL), &word(FREE)).await.done().await;
        println!("{:3}  id    {:02x?}", now(), id);
        assert_eq!(id, ID, "the chip said who it is");

        host.write(Wr::at(CTRL), &word(HELD)).await.done().await;
        swap(&host, 0x0b).await;
        swap(&host, (AT >> 16) & 0xff).await;
        swap(&host, (AT >> 8) & 0xff).await;
        swap(&host, AT & 0xff).await;
        swap(&host, 0x00).await;
        let mut read = [0u8; 4];
        for slot in read.iter_mut() {
            *slot = swap(&host, 0x00).await;
        }
        host.write(Wr::at(CTRL), &word(FREE)).await.done().await;
        println!("{:3}  read  {:02x?}  from {AT:#08x}", now(), read);
        assert_eq!(read, STORED, "the bytes at that address");
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            spi.run(
                (paw, par, pw, miso),
                (pb, pr, sclk_out, mosi_out, cs_out, irq_out),
            ),
            client,
        ),
    ));

    // The chip holds a stretch of bytes with the four the program
    // wants at `AT`.
    let mut image = vec![0u8; AT as usize + STORED.len()];
    image[AT as usize..].copy_from_slice(&STORED);
    let mut chip = FlashDevice::new(image, ID, CPOL, CPHA);

    println!("  t  what the program read");
    for _ in 0..3600 {
        sim.cycle();
        chip.step(
            cs_n.get().to_bool(),
            sclk.get().to_bool(),
            mosi.get().to_bool(),
        );
        miso_drive.set(Bit::from_bool(chip.miso()));
    }
    stop();
    assert_eq!(irq.get(), Bit::Zero, "no interrupt was asked for");
    let net = Spi::lowered("spi");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
