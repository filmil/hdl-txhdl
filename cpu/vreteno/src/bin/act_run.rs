// SPDX-License-Identifier: Apache-2.0
//! Runs one RISC-V architectural test on the core's cycle simulation
//! and says whether it passed (issue 1442).
//!
//! The machine is the one vreteno-conformance simulates as a netlist:
//! the hart with its caches, its link's tracker, and one memory that
//! answers every address the core reaches. The boot memory holds two
//! instructions that jump to the test's entry point, so the whole test
//! runs from that memory, cached or not as its placement makes it.
//!
//! The test speaks HTIF through `tohost`, as it does to Sail: a write
//! to the word after `tohost` completes a command. A command whose
//! upper half is `0x0101` prints the low byte; any other ends the
//! test, passing when the low word is 1.
//!
//! One rule is the runner's own, because the framework does not keep
//! it (issue 1460): an illegal instruction taken outside the test's
//! code, in the framework's boot code, is a failure. The framework
//! records nothing for such a trap when `a0` is zero, which it is after
//! a reset, so a CSR the description promises and the core lacks would
//! otherwise pass unseen.
//!
//! ```text
//! act_run --elf T.elf --name T [--max-cycles N] [--expect pass|fail] [--excl]
//! ```
//!
//! It prints one line, `name=T status=PASS|FAIL|TIMEOUT cycles=...
//! retired=... code=...`, then what the test printed, and exits zero
//! when the status is the one `--expect` names.
//!
//! With `--excl` the hart is the board's, whose `lr.w`, `sc.w` and
//! AMOs on the DDR3 are exclusive pairs, and the exclusive monitor stands
//! between its tracker and the memory as it does on the board, so the
//! A extension's tests run against the pairs the board keeps (issue
//! 1408). With one host nothing else writes, so the arbiter's hold has
//! nothing to hold and is left out.
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use txhdl::comp::{chan, join2, signal, DefaultClock, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::{
    axi_units, per_end, Ar, Aw, AxiHost, AxiPer, Per, Xact, B, W,
};
use txhdl_parts::bus::exmon::ExMon;
use vreteno32::core::Writeback;
use vreteno32::hart::Hart;

/// The link, as the board has it.
const IW: usize = 2;
const NIDS: usize = 4;

/// What an ELF gives the run: its words, where it starts, and the
/// symbols the run watches.
struct Image {
    words: HashMap<u32, u32>,
    entry: u32,
    tohost: u32,
    code: (u32, u32),
}

fn u16_at(b: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u32
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Reads a 32-bit little-endian ELF: its loadable segments, word by
/// word, and three symbols from its symbol table.
fn load(path: &str) -> Image {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(&b[0..4], b"\x7fELF", "{path} is not an ELF");
    assert_eq!(b[4], 1, "{path} is not a 32-bit ELF");
    let entry = u32_at(&b, 24);
    let (phoff, shoff) = (u32_at(&b, 28) as usize, u32_at(&b, 32) as usize);
    let (phentsize, phnum) = (u16_at(&b, 42) as usize, u16_at(&b, 44) as usize);
    let (shentsize, shnum) = (u16_at(&b, 46) as usize, u16_at(&b, 48) as usize);
    let mut words = HashMap::new();
    for i in 0..phnum {
        let p = phoff + i * phentsize;
        if u32_at(&b, p) != 1 {
            continue;
        }
        let (off, paddr) = (u32_at(&b, p + 4) as usize, u32_at(&b, p + 12));
        let (filesz, memsz) = (u32_at(&b, p + 16) as usize, u32_at(&b, p + 20));
        let mut bytes = b[off..off + filesz].to_vec();
        bytes.resize(memsz as usize, 0);
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        for (k, w) in bytes.chunks(4).enumerate() {
            words.insert(paddr + 4 * k as u32, u32_at(w, 0));
        }
    }
    let mut syms = HashMap::new();
    for i in 0..shnum {
        let s = shoff + i * shentsize;
        if u32_at(&b, s + 4) != 2 {
            continue;
        }
        let (off, size) =
            (u32_at(&b, s + 16) as usize, u32_at(&b, s + 20) as usize);
        let strtab = shoff + u32_at(&b, s + 24) as usize * shentsize;
        let stroff = u32_at(&b, strtab + 16) as usize;
        for e in (off..off + size).step_by(16) {
            let name = stroff + u32_at(&b, e) as usize;
            let end = b[name..].iter().position(|&c| c == 0).unwrap();
            let n = String::from_utf8_lossy(&b[name..name + end]).to_string();
            syms.insert(n, u32_at(&b, e + 4));
        }
    }
    let sym = |n: &str| -> u32 {
        *syms.get(n).unwrap_or_else(|| panic!("{path} has no `{n}`"))
    };
    Image {
        words,
        entry,
        tohost: sym("tohost"),
        code: (sym("rvtest_code_begin"), sym("rvtest_code_end")),
    }
}

/// What the test has said through `tohost`.
#[derive(Default)]
struct Htif {
    said: String,
    exit: Option<u32>,
}

/// The memory behind the link: every address, word by word, with the
/// test's image in it. A write to the word after `tohost` is an HTIF
/// command.
async fn memory(
    per: Per<32, 32, 4, IW>,
    words: Rc<RefCell<HashMap<u32, u32>>>,
    tohost: u32,
    htif: Rc<RefCell<Htif>>,
) {
    loop {
        match per.accept().await {
            Xact::Write(w) => {
                let base = w.addr().raw() as u32 & !3;
                for (i, (d, s)) in w.data().iter().zip(w.strb()).enumerate() {
                    let a = base + 4 * i as u32;
                    let mut m = words.borrow_mut();
                    let old = *m.get(&a).unwrap_or(&0);
                    let mut mask = 0u32;
                    for lane in 0..4 {
                        if s.raw() >> lane & 1 == 1 {
                            mask |= 0xff << (8 * lane);
                        }
                    }
                    let new = (old & !mask) | (d.raw() as u32 & mask);
                    m.insert(a, new);
                    if a == tohost + 4 {
                        let lo = *m.get(&tohost).unwrap_or(&0);
                        let mut h = htif.borrow_mut();
                        if new >> 16 == 0x0101 {
                            h.said.push(char::from(lo as u8));
                        } else if lo & 1 == 1 {
                            h.exit = Some(lo);
                        }
                        m.insert(tohost, 0);
                        m.insert(tohost + 4, 0);
                    }
                }
                w.ok().await;
            }
            Xact::Read(r) => {
                let base = r.addr().raw() as u32 & !3;
                let data: Vec<U<32>> = {
                    let m = words.borrow();
                    (0..r.words())
                        .map(|i| {
                            U::from(
                                *m.get(&(base + 4 * i as u32)).unwrap_or(&0),
                            )
                        })
                        .collect()
                };
                r.data(&data).await;
            }
        }
    }
}

/// The run, on the hart `EXCL` builds (issue 1408): what the test said,
/// the cycles, the instructions retired, and the illegal instructions
/// taken outside the test's code.
fn simulate<const EXCL: usize>(
    img: &Image,
    boot: &[u32],
    max_cycles: u64,
) -> (Htif, u64, u128, Vec<u32>) {
    let mut hart = Hart::<2, 16384, 1, 0, EXCL>::with(boot);
    let cpu = &hart.core;
    let (mcause, mepc, minstret, halted) =
        (cpu.mcause, cpu.mepc, cpu.minstret, cpu.halted);
    let words = Rc::new(RefCell::new(img.words.clone()));
    let htif = Rc::new(RefCell::new(Htif::default()));
    let (rst_out, rst) = signal::<Bit, DefaultClock>();
    let (_irq_out, irq) = signal::<Bit, DefaultClock>();
    let (_tirq_out, tirq) = signal::<Bit, DefaultClock>();
    let (_sirq_out, sirq) = signal::<Bit, DefaultClock>();
    let (_seirq_out, seirq) = signal::<Bit, DefaultClock>();
    let (time_out, time) = signal::<U<64>, DefaultClock>();
    // No other host writes the memory, so nothing is snooped.
    let (_dc_snoop_out, dc_snoop) = signal::<U<9>, DefaultClock>();
    // No debugger: its requests stay low and its answers are unread.
    let (_haltreq_o, haltreq) = signal::<Bit, DefaultClock>();
    let (_resumereq_o, resumereq) = signal::<Bit, DefaultClock>();
    let (_dbg_regno_o, dbg_regno) = signal::<U<16>, DefaultClock>();
    let (_dbg_wdata_o, dbg_wdata) = signal::<U<32>, DefaultClock>();
    let (_dbg_we_o, dbg_we) = signal::<Bit, DefaultClock>();
    let (debug_o, _debug) = signal::<Bit, DefaultClock>();
    let (dbg_rdata_o, _dbg_rdata) = signal::<U<32>, DefaultClock>();
    let (halt_out, _halt) = signal::<Bit, DefaultClock>();
    let (instr_out, _instr) = signal::<U<32>, DefaultClock>();
    let (wb_out, _wb) = signal::<Writeback, DefaultClock>();
    let link = axi_units::<32, 32, 4, IW>();
    let (issue, wbeat, release, grant, cdone, crdata) = link.host_client;
    let per = per_end(link.per_client);
    let mut axi_host = AxiHost::<32, 32, 4, IW, NIDS>::default();
    let mut axi_per = AxiPer::<32, 32, 4, IW>::default();
    let mut exmon = ExMon::<IW, IW, 0, 1, 0x4000_0000, 0xc000_0000>::default();
    let mem = memory(per, words, img.tohost, htif.clone());
    // The tracker straight to the memory's unit, or with the monitor
    // between them.
    let hardware: Pin<Box<dyn Future<Output = ()> + '_>> = if EXCL != 0 {
        let (aw_rx, ar_rx, w_rx, ans_rx, rb_rx) = link.per_in;
        let (req_tx, wd_tx, b_tx, r_tx) = link.per_out;
        let (maw_tx, maw_rx) = chan::<Aw<32, IW>, DefaultClock>();
        let (mar_tx, mar_rx) = chan::<Ar<32, IW>, DefaultClock>();
        let (mw_tx, mw_rx) = chan::<W<32, 4>, DefaultClock>();
        let (mb_tx, mb_rx) = chan::<B<IW>, DefaultClock>();
        Box::pin(join2(
            join2(
                axi_host.run(link.host_in, link.host_out),
                join2(
                    exmon.run(
                        (aw_rx, ar_rx, w_rx, mb_rx),
                        (maw_tx, mar_tx, mw_tx, b_tx),
                    ),
                    axi_per.run(
                        (maw_rx, mar_rx, mw_rx, ans_rx, rb_rx),
                        (req_tx, wd_tx, mb_tx, r_tx),
                    ),
                ),
            ),
            mem,
        ))
    } else {
        Box::pin(join2(
            join2(
                axi_host.run(link.host_in, link.host_out),
                axi_per.run(link.per_in, link.per_out),
            ),
            mem,
        ))
    };
    let mut sim = Running::new(join2(
        hart.run(
            (
                rst, irq, tirq, sirq, crdata, cdone, grant, haltreq, resumereq,
                dbg_regno, dbg_wdata, dbg_we, time, seirq, dc_snoop,
            ),
            (
                halt_out,
                instr_out,
                wb_out,
                issue,
                wbeat,
                release,
                debug_o,
                dbg_rdata_o,
            ),
        ),
        hardware,
    ));
    rst_out.set(Bit::One);
    sim.cycle();
    rst_out.set(Bit::Zero);
    let mut cycles = 0u64;
    // The trap last seen, so that a trap is counted when it is taken
    // and not on every cycle after.
    let mut seen = (mcause.get().raw(), mepc.get().raw());
    let mut boot_traps = Vec::new();
    while cycles < max_cycles
        && htif.borrow().exit.is_none()
        && !halted.get().to_bool()
    {
        time_out.set(U::from(cycles / 32));
        sim.cycle();
        cycles += 1;
        let now = (mcause.get().raw(), mepc.get().raw());
        if now != seen {
            seen = now;
            let (cause, at) = (now.0 as u32, now.1 as u32);
            if cause == 2 && !(img.code.0..img.code.1).contains(&at) {
                boot_traps.push(at);
            }
        }
    }
    let h = std::mem::take(&mut *htif.borrow_mut());
    (h, cycles, minstret.get().raw(), boot_traps)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| -> Option<String> {
        args.iter()
            .position(|a| a == k)
            .map(|i| args[i + 1].clone())
    };
    let elf = arg("--elf").expect("--elf T.elf");
    let name = arg("--name").unwrap_or_else(|| elf.clone());
    let max_cycles: u64 =
        arg("--max-cycles").map_or(20_000_000, |s| s.parse().unwrap());
    let expect = arg("--expect").unwrap_or_else(|| "pass".into());
    let img = load(&elf);
    // `lui t0, %hi(entry)` and `jr t0`: the entry is page aligned.
    assert_eq!(img.entry & 0xfff, 0, "the entry point is not page aligned");
    let boot = [img.entry | 0x2b7, 0x0002_8067];
    let (h, cycles, retired, boot_traps) = if args.iter().any(|a| a == "--excl")
    {
        simulate::<1>(&img, &boot, max_cycles)
    } else {
        simulate::<0>(&img, &boot, max_cycles)
    };
    let status = match h.exit {
        None => "TIMEOUT",
        Some(1) if boot_traps.is_empty() => "PASS",
        Some(_) => "FAIL",
    };
    let code = h.exit.map_or(0, |c| c >> 1);
    println!(
        "name={name} status={status} cycles={cycles} retired={} code={code}",
        retired
    );
    if !boot_traps.is_empty() {
        println!(
            "illegal instructions taken outside the test's code, at {:x?} (issue 1460)",
            &boot_traps[..boot_traps.len().min(8)]
        );
    }
    print!("{}", h.said);
    let ok = match expect.as_str() {
        "fail" => status != "PASS",
        _ => status == "PASS",
    };
    std::process::exit(if ok { 0 } else { 1 });
}
