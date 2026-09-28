// SPDX-License-Identifier: Apache-2.0
//! The configuration flash reached after the bitstream has loaded:
//! the master and the window on one chip, through `STARTUPE2`.
//!
//! `CfgFlash` is the primitive and the pins between the chip and the
//! two parts that reach it. Its model of the primitive says when
//! configuration is over and swallows the first three clocks after,
//! as the part does, so this run shows what the pins do about both.
//!
//! The window is asked for a word at once, before configuration is
//! over: the pins hold it in reset, and the read waits and is answered
//! when the chip is ready. The master then asks the chip who it is,
//! which is the first command the chip sees after the swallowed clocks
//! and must arrive whole. Then the master sends write enable, the one
//! command that lets a flash be changed, and the pins let the chip go
//! before its last bit, so the chip never takes it. The window reads a
//! second word to show the chip is still answering.
//!
//! The chip is `FlashDevice`, stepped between cycles from the loop, and
//! it keeps a list of the commands it took whole.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{
    axi, axi_units, host_end, AxiHost, AxiPer, Host, Link, PerPort, Rd, Resp,
    Wr,
};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge, LitePort};
use txhdl_parts::cfgflash::CfgFlash;
use txhdl_parts::flashwin::FlashWin;
use txhdl_parts::spi::{regs, FlashDevice, Spi, SpiLines};

/// The master's link: its host, and the bridge with the master at
/// `0x1000`.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;
type Bridge = LiteBridge<1, SpiMap, 32, 32, 4, 2>;
type Client = Host<32, 32, 4, 2, 4>;

/// Where the bridge's one peripheral is.
pub struct SpiMap;

impl AddrMap<1> for SpiMap {
    const RANGES: [(usize, usize); 1] = [(0x1000, 0xf000)];
}

const BASE: u32 = 0x1000;
const CTRL: u32 = BASE + regs::ctrl;
const DATA: u32 = BASE + regs::data;
const STATE: u32 = BASE + regs::state;

/// A half of a bit takes four cycles, for both: every output of the
/// pins is a register and the clock passes the primitive too, so the
/// chip's answer needs that long to come back.
const DIV: usize = 3;
const HELD: u32 = DIV as u32 | regs::ctrl_sel.mask();
const FREE: u32 = DIV as u32;

/// The pins refuse write enable.
const GUARD: usize = 1;

/// What the chip says it is: the MT25QL128 on the board.
const ID: [u8; 3] = [0x20, 0xba, 0x18];
/// Where the words are, and what they are.
const AT: u32 = 0x40;
const WORDS: [u32; 2] = [0xdead_beef, 0x0123_4567];

/// One byte each way, as `ex_spi` does it.
async fn swap(host: &Client, byte: u32) -> u8 {
    host.write(Wr::at(DATA), &[U::<32>::from(byte)])
        .await
        .done()
        .await;
    loop {
        let st = host.read(Rd::at(STATE, 1)).await.done().await.data[0];
        if regs::state_busy.get(st.raw() as u32) == 0 {
            break;
        }
    }
    let got = host.read(Rd::at(DATA, 1)).await.done().await.data[0];
    (got.raw() & 0xff) as u8
}

async fn ctrl(host: &Client, v: u32) {
    let r = host
        .write(Wr::at(CTRL), &[U::<32>::from(v)])
        .await
        .done()
        .await;
    assert_eq!(r.resp, Resp::Okay, "the control write was answered");
}

fn main() {
    // The master, behind its bridge.
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
    let lbus: LitePort<32, 32, 4> = lite.per.into();
    // The window, on a link of its own.
    let u = axi_units::<32, 32, 4, 2>();
    let whost = host_end::<32, 32, 4, 2, 4>(u.host_client);
    let wbus: PerPort<32, 32, 4, 2> = u.per_client.into();
    // The wires. `miso` goes from the chip to both.
    let (miso_drive, miso) = signal::<Bit, DefaultClock>();
    let (m_sclk_o, m_sclk) = signal::<Bit, DefaultClock>();
    let (m_mosi_o, m_mosi) = signal::<Bit, DefaultClock>();
    let (m_cs_n_o, m_cs_n) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let (w_sclk_o, w_sclk) = signal::<Bit, DefaultClock>();
    let (w_mosi_o, w_mosi) = signal::<Bit, DefaultClock>();
    let (w_cs_n_o, w_cs_n) = signal::<Bit, DefaultClock>();
    let (w_rst_o, w_rst) = signal::<Bit, DefaultClock>();
    let (cclk_o, cclk) = signal::<Bit, DefaultClock>();
    let (mosi_o, mosi) = signal::<Bit, DefaultClock>();
    let (cs_n_o, cs_n) = signal::<Bit, DefaultClock>();
    let (refused_o, refused) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut spi = Spi::default();
    let mut whost_unit = AxiHost::<32, 32, 4, 2, 4>::default();
    let mut wper_unit = AxiPer::<32, 32, 4, 2>::default();
    let mut win = FlashWin::<DIV, 2>::default();
    let mut flash = CfgFlash::<GUARD>::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("m_sclk", &m_sclk);
        wave.add("m_mosi", &m_mosi);
        wave.add("m_cs_n", &m_cs_n);
        wave.add("w_sclk", &w_sclk);
        wave.add("w_mosi", &w_mosi);
        wave.add("w_cs_n", &w_cs_n);
        wave.add("cclk", &cclk);
        wave.add("mosi", &mosi);
        wave.add("cs_n", &cs_n);
        wave.add("w_rst", &w_rst);
        wave.add("refused", &refused);
        wave.add("flash", &flash);
        wave.start();
    }

    let client = async move {
        // Asked before configuration is over, answered after.
        let first = whost.read(Rd::at(AT, 1)).await.done().await;
        println!("{:4}  window read  {:08x}", now(), first.data[0].raw());
        assert_eq!(first.data[0].raw() as u32, WORDS[0], "the first word");

        ctrl(&host, HELD).await;
        swap(&host, 0x9f).await;
        let mut id = [0u8; 3];
        for slot in id.iter_mut() {
            *slot = swap(&host, 0x00).await;
        }
        ctrl(&host, FREE).await;
        println!("{:4}  master id    {:02x?}", now(), id);
        assert_eq!(id, ID, "the chip said who it is");

        ctrl(&host, HELD).await;
        swap(&host, 0x06).await;
        ctrl(&host, FREE).await;
        println!("{:4}  master sent write enable", now());

        let second = whost.read(Rd::at(AT + 4, 1)).await.done().await;
        println!("{:4}  window read  {:08x}", now(), second.data[0].raw());
        assert_eq!(second.data[0].raw() as u32, WORDS[1], "the second word");
    };

    let mut sim = Running::new(join2(
        join2(
            join2(
                host_unit.run(host_in, host_out),
                bridge.run((aw, ar, w, [lb], [lr]), ([law], [lar], [lw], b, r)),
            ),
            spi.run(
                lbus,
                SpiLines {
                    miso: miso.clone(),
                    sclk: m_sclk_o,
                    mosi: m_mosi_o,
                    cs_n: m_cs_n_o,
                    irq: irq_o,
                },
            ),
        ),
        join2(
            join2(
                join2(
                    whost_unit.run(u.host_in, u.host_out),
                    wper_unit.run(u.per_in, u.per_out),
                ),
                win.run(
                    wbus,
                    (w_rst.clone(), miso, w_sclk_o, w_mosi_o, w_cs_n_o),
                ),
            ),
            join2(
                flash.run(
                    (m_sclk, m_mosi, m_cs_n, w_sclk, w_mosi, w_cs_n),
                    (cclk_o, mosi_o, cs_n_o, w_rst_o, refused_o),
                ),
                client,
            ),
        ),
    ));

    let mut image = vec![0u8; AT as usize + 8];
    for (k, w) in WORDS.iter().enumerate() {
        image[AT as usize + k * 4..AT as usize + k * 4 + 4]
            .copy_from_slice(&w.to_le_bytes());
    }
    let mut chip = FlashDevice::new(image, ID, false, false);

    println!("   t  what the programs saw");
    let mut ready_at = None;
    let mut refusals = 0;
    let mut was = Bit::Zero;
    for t in 0..9000 {
        sim.cycle();
        chip.step(
            cs_n.get().to_bool(),
            cclk.get().to_bool(),
            mosi.get().to_bool(),
        );
        miso_drive.set(Bit::from_bool(chip.miso()));
        if ready_at.is_none() && w_rst.get() == Bit::Zero {
            ready_at = Some(t);
        }
        if refused.get() == Bit::One && was == Bit::Zero {
            refusals += 1;
        }
        was = refused.get();
    }
    stop();
    println!("the chip ready at cycle {}", ready_at.expect("ready"));
    println!("the chip took whole: {:02x?}", chip.commands);
    println!("let go short: {}, refused: {}", chip.partial, refusals);
    assert_eq!(chip.commands, [0x0b, 0x9f, 0x0b], "never write enable");
    assert_eq!(chip.partial, 1, "write enable let go short");
    assert_eq!(refusals, 1, "and the pins said so");
    assert_eq!(irq.get(), Bit::Zero, "no interrupt was asked for");
    let net = CfgFlash::<GUARD>::lowered("cfgflash");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
