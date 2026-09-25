// SPDX-License-Identifier: Apache-2.0
//! A native-mode SD card host behind AXI-Lite, against a model of a
//! card: the card brought up, a block read on one line, and a block
//! written and read back on four.
//!
//! A host client on an AXI4 link reaches the host through the
//! AXI-Lite bridge and does what a driver does. It gives the card the
//! eighty clocks it wants first, resets it with `CMD0`, asks its
//! voltage with `CMD8`, waits for it to come ready with `ACMD41`,
//! takes its identity with `CMD2` and its address with `CMD3`,
//! selects it with `CMD7`, and puts it on four lines with `ACMD6`.
//! Then it reads block 3 with `CMD17`, fills the buffer and writes
//! block 5 with `CMD24`, and reads block 5 back.
//! The card is the model in `txhdl_parts::sd`, stepped on the wires
//! every cycle, so what is on the command and data lines is the
//! protocol and not a shortcut.
//!
//! The card's clock runs at half the system's here, to keep the run
//! short; a board starts at 400 kHz and moves to 25 MHz once the card
//! is up. The host is lowered, and the build simulates
//! its netlist against this run under nvc and Verilator, with the
//! card's lines as the run recorded them.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{join2, now, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{axi, AxiHost, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{axi_lite, LiteBridge1, LitePort};
use txhdl_parts::sd::{
    regs, Sd, SdCard, CMD_BUSY, CMD_CLOCKS, CMD_LONG, CMD_NOCRC, CMD_READ,
    CMD_SHORT, CMD_WRITE, CTRL_CLEAR, CTRL_WIDE, STATUS_DCRC, STATUS_DONE,
    STATUS_DTIMEOUT, STATUS_RCRC, STATUS_RTIMEOUT, WORDS,
};

/// The link: thirty-two-bit addresses and words, four lanes, two-bit
/// identifiers, four of them.
type HostUnit = AxiHost<32, 32, 4, 2, 4>;

/// The bridge, with the host at `0x1000`.
type Bridge = LiteBridge1<32, 32, 4, 2, 0x1000, 0xf000>;

const BASE: u32 = 0x1000;

/// A half of a card clock in one cycle.
const DIV: u32 = 0;

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
    let (cmd_in_o, cmd_in) = signal::<Bit, DefaultClock>();
    let (dat_in_o, dat_in) = signal::<U<4>, DefaultClock>();
    let (sclk_o, sclk) = signal::<Bit, DefaultClock>();
    let (cmd_out_o, cmd_out) = signal::<Bit, DefaultClock>();
    let (cmd_oe_o, cmd_oe) = signal::<Bit, DefaultClock>();
    let (dat_out_o, dat_out) = signal::<U<4>, DefaultClock>();
    let (dat_oe_o, dat_oe) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut sd = Sd::default();
    let mut card = SdCard::default();
    let block3: Vec<u32> = card.blocks[3 * 512..4 * 512]
        .chunks(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        wave.add("bus_aw", &bus.aw);
        wave.add("bus_ar", &bus.ar);
        wave.add("bus_w", &bus.w);
        wave.add("bus_b", &bus.b);
        wave.add("bus_r", &bus.r);
        wave.add("cmd_in", &cmd_in);
        wave.add("dat_in", &dat_in);
        wave.add("sclk", &sclk);
        wave.add("cmd_out", &cmd_out);
        wave.add("cmd_oe", &cmd_oe);
        wave.add("dat_out", &dat_out);
        wave.add("dat_oe", &dat_oe);
        wave.add("irq", &irq);
        wave.add("sd", &sd);
        wave.start();
    }

    let client = async move {
        let word = |v: u32| [U::<32>::from(v)];
        let at = |off: u32| BASE + off;
        let get = |off: u32| {
            let h = &host;
            async move {
                let got = h.read(Rd::at(at(off), 1)).await.done().await;
                got.data[0].raw() as u32
            }
        };
        let put = |off: u32, v: u32| {
            let h = &host;
            async move {
                let ok = h.write(Wr::at(at(off)), &word(v)).await.done().await;
                assert_eq!(ok.resp, Resp::Okay, "the write was answered");
            }
        };
        // A command: the argument, the word, the wait for done, and
        // the status, which must show no fault.
        let command = |index: u32, arg: u32, flags: u32| async move {
            put(regs::arg, arg).await;
            put(regs::cmd, index | flags).await;
            let s = loop {
                let s = get(regs::status).await;
                if s & STATUS_DONE != 0 {
                    break s;
                }
            };
            put(regs::status, STATUS_DONE).await;
            let faults =
                STATUS_RTIMEOUT | STATUS_RCRC | STATUS_DTIMEOUT | STATUS_DCRC;
            assert_eq!(s & faults, 0, "command {index}: status {s:#x}");
            s
        };
        put(regs::ctrl, DIV).await;
        // Eighty clocks with the line high, which a card wants first.
        command(0, 0, CMD_CLOCKS).await;
        command(0, 0, 0).await;
        command(8, 0x1aa, CMD_SHORT).await;
        println!("{:5}  CMD8 answered {:#x}", now(), get(regs::resp0).await);
        let mut tries = 0;
        loop {
            command(55, 0, CMD_SHORT).await;
            command(41, 0x4030_0000, CMD_SHORT | CMD_NOCRC).await;
            tries += 1;
            let ocr = get(regs::resp0).await;
            if ocr & 0x8000_0000 != 0 {
                println!(
                    "{:5}  ready after {tries} ACMD41: OCR {ocr:#x}",
                    now()
                );
                break;
            }
        }
        command(2, 0, CMD_LONG).await;
        println!(
            "{:5}  CID {:08x} {:08x} {:08x} {:08x}",
            now(),
            get(regs::resp3).await,
            get(regs::resp2).await,
            get(regs::resp1).await,
            get(regs::resp0).await
        );
        command(3, 0, CMD_SHORT).await;
        let rca = get(regs::resp0).await >> 16;
        println!("{:5}  RCA {rca:#x}", now());
        command(7, rca << 16, CMD_SHORT | CMD_BUSY).await;
        command(16, 512, CMD_SHORT).await;
        // Block 3 on one line.
        command(17, 3, CMD_SHORT | CMD_READ).await;
        let mut got = Vec::with_capacity(WORDS);
        for _ in 0..WORDS {
            got.push(get(regs::data).await);
        }
        assert_eq!(got, block3, "block 3 as the card holds it");
        println!(
            "{:5}  block 3 on one line: {:08x} {:08x} .. {:08x}",
            now(),
            got[0],
            got[1],
            got[WORDS - 1]
        );
        // Four lines from here.
        command(55, rca << 16, CMD_SHORT).await;
        command(6, 2, CMD_SHORT).await;
        put(regs::ctrl, DIV | CTRL_WIDE).await;
        // Block 5 written, then read back.
        let sent: Vec<u32> = (0..WORDS as u32)
            .map(|i| i.wrapping_mul(0x9e37_79b9))
            .collect();
        put(regs::ctrl, DIV | CTRL_WIDE | CTRL_CLEAR).await;
        for v in &sent {
            put(regs::data, *v).await;
        }
        let s = command(24, 5, CMD_SHORT | CMD_WRITE).await;
        println!(
            "{:5}  block 5 written on four lines: CRC status {:03b}",
            now(),
            (s >> 6) & 7
        );
        command(17, 5, CMD_SHORT | CMD_READ).await;
        let mut back = Vec::with_capacity(WORDS);
        for _ in 0..WORDS {
            back.push(get(regs::data).await);
        }
        assert_eq!(back, sent, "block 5 as written");
        println!(
            "{:5}  block 5 read back on four lines: {:08x} {:08x} .. {:08x}",
            now(),
            back[0],
            back[1],
            back[WORDS - 1]
        );
    };

    let mut sim = Running::new(join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run((aw, ar, w, lb, lr), (law, lar, lw, b, r)),
        ),
        join2(
            sd.run(
                bus,
                (
                    cmd_in, dat_in, sclk_o, cmd_out_o, cmd_oe_o, dat_out_o,
                    dat_oe_o, irq_o,
                ),
            ),
            client,
        ),
    ));
    println!("    t  what the program saw");
    cmd_in_o.set(Bit::One);
    dat_in_o.set(U::<4>::from(0xfu8));
    for _ in 0..36000 {
        sim.cycle();
        // The wires: whoever drives them, and high when nobody does.
        let host_cmd = cmd_oe.get().to_bool();
        let host_dat = dat_oe.get().to_bool();
        let cmd = cmd_out.get().to_bool();
        let dat = dat_out.get().raw() as u8;
        card.step(sclk.get().to_bool(), host_cmd, cmd, host_dat, dat);
        cmd_in_o.set(Bit::from_bool(if host_cmd {
            cmd
        } else {
            card.cmd_out()
        }));
        dat_in_o.set(U::<4>::from(if host_dat { dat } else { card.dat_out() }));
    }
    stop();
    let net = Sd::lowered("sd");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!("\n{}", net.verilog());
}
