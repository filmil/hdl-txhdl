// SPDX-License-Identifier: Apache-2.0
//! The Vreteno board as a fast machine, from the command line (issue
//! 1016): a flat image and a device tree blob loaded into the DDR3, the
//! hart started at the image, the serial port's output on standard
//! output as it is sent, and standard input on the port's receive side
//! as it arrives (issue 1127), so a shell on the machine can be typed
//! into or fed from a pipe.
//!
//! ```text
//! bazel run //cpu/vreteno:machine -- --image $PWD/fw_jump.bin \
//!     [--at 0x40000000] [--dtb $PWD/vreteno.dtb] [--dtb-at 0x41008000] \
//!     [--steps 100000000] [--as-loaded BYTES]
//! ```
//!
//! `--eth-peer` puts a station on the Ethernet port's cable that answers
//! ARP and ping at 10.0.0.2 (issue 1203), and says on standard error how
//! many frames went each way when the machine stops.
//!
//! `--timing` charges each step what the core would spend on it, with
//! the board's costs (issue 1392), so `mcycle` reads cycles and the
//! machine says the total when it stops. `--ddr3-store N` charges a
//! store into the DDR3 `N` cycles instead of the board's, to ask what a
//! dearer store would cost a run (issue 1408). `--dcache` charges a load
//! the core's data cache would hold as a hit rather than the DDR3's 40,
//! which without the costs it does not yet model leaves a run well under
//! the board's time (issue 1554).
//!
//! `--rings` says, when the run stops, every word written to Razboj's
//! doorbell and where the scanout showed from as it was written. Razboj
//! is a stand-in here, done at once, and the raster's blanking comes
//! every hundred thousand steps (issue 1551).
//!
//! `--fastboot-peer BYTES` puts a fastboot client on the cable instead
//! (issue 1390), smoltcp's TCP/IP at 192.168.1.1 with a client on top,
//! which sends the fastboot server at 192.168.1.50 a download of `BYTES`
//! bytes. The machine stops when the server has answered `OKAY`, and
//! says on standard error the bytes, the steps the transfer took and a
//! byte's share of them, the client's retransmits and the frames the
//! port dropped.
//!
//! `--as-loaded` starts the serial port as the serial loader leaves it
//! on the board (issue 1136): its receive interrupt enabled, as the
//! hardware resets it, and `BYTES` waiting to be read, so the line is
//! high and the interrupt controller has the request before the image
//! runs.
//!
//! `--until TEXT` stops the run once the console has said `TEXT`, and
//! the count it gives on standard error, of instructions and in the
//! timing mode of cycles, is then how long the run took to say it,
//! within ten thousand instructions (issue 1441). A boot timed to
//! `/init`'s marker is a boot's time to userspace.
//!
//! `--screen FILE` writes what the scanout would show when the run
//! stops, as a PPM: the frame at the scanout's base in the DDR3, in the
//! flagship's mode, 640 by 480, a word a pixel, 4096 bytes a line, or
//! nothing if the scanout is not shown (issue 1440).
//!
//! `--profile FILE` writes what each address the hart ran at cost when
//! the run stops, a line each: the address in hex, its cycles (steps
//! outside the timing mode) and how many times it ran (issue 1434).
//! Against a program's symbols that is a flat profile, and a function's
//! first address counts its calls, which a cable's pacing does not
//! change. Once the second hart has run, its profile is beside it in
//! `FILE.hart1` (issue 1408).
//!
//! `--watch ADDR` says, when the run stops, who reached `ADDR`: each
//! return address in `ra` there, how many times, and the sum of `a2`,
//! which for a copy is its length (issue 1434).
//!
//! It stops when the hart halts, or after `--steps` instructions, and
//! says which, with the program counter, on standard error.
use std::io::{Read, Write};
use std::sync::mpsc;
use vreteno32::machine::Machine;

/// Where the fastboot server stages a download: `fastboot_stage` in
/// `zephyr/fastboot/app/boards/ax7a200b.overlay`.
const STAGE: u32 = 0x4800_0000;

/// Where the image goes and starts, by default: the DDR3's base, which
/// is OpenSBI's `FW_TEXT_START`.
const AT: u32 = 0x4000_0000;
/// Where the blob goes by default: above the Ethernet buffers, clear of
/// OpenSBI's jump address and of where it moves the blob to.
const DTB_AT: u32 = 0x4100_8000;

fn number(s: &str) -> u64 {
    let s = s.replace('_', "");
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16),
        None => s.parse(),
    }
    .unwrap_or_else(|_| panic!("not a number: {s}"))
}

fn main() {
    let mut image = None;
    let mut at = AT;
    let mut dtb = None;
    let mut dtb_at = DTB_AT;
    let mut steps = 100_000_000u64;
    let mut loaded = None;
    let mut peer = false;
    let mut timing = false;
    let mut fastboot = None;
    let mut until: Option<String> = None;
    let mut screen: Option<String> = None;
    let mut profile: Option<String> = None;
    let mut watch: Option<u32> = None;
    let mut ddr3_store: Option<u64> = None;
    let mut dcache = false;
    let mut rings = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val =
            || args.next().unwrap_or_else(|| panic!("{a} wants a value"));
        match a.as_str() {
            "--image" => image = Some(val()),
            "--at" => at = number(&val()) as u32,
            "--dtb" => dtb = Some(val()),
            "--dtb-at" => dtb_at = number(&val()) as u32,
            "--steps" => steps = number(&val()),
            "--as-loaded" => loaded = Some(val()),
            "--eth-peer" => peer = true,
            "--timing" => timing = true,
            "--ddr3-store" => ddr3_store = Some(number(&val())),
            "--dcache" => dcache = true,
            "--rings" => rings = true,
            "--fastboot-peer" => fastboot = Some(number(&val()) as usize),
            // Stop once the console has said this, and say how far the
            // run got, which times a boot to a line of its log (issue
            // 1441).
            "--until" => until = Some(val()),
            // What the scanout would show when the run stops, as a PPM
            // (issue 1440).
            "--screen" => screen = Some(val()),
            // What each address cost, written when the run stops (issue
            // 1434).
            "--profile" => profile = Some(val()),
            // Who reaches an address, and with what length in a2.
            "--watch" => watch = Some(number(&val()) as u32),
            _ => panic!("unknown argument {a}"),
        }
    }
    let image = image.expect("--image is required");
    let profiling = profile.is_some();
    let bytes =
        std::fs::read(&image).unwrap_or_else(|e| panic!("{image}: {e}"));
    let mut m = Machine::new();
    m.load(at, &bytes);
    if let Some(d) = &dtb {
        let blob = std::fs::read(d).unwrap_or_else(|e| panic!("{d}: {e}"));
        m.load(dtb_at, &blob);
    }
    m.boot(at, if dtb.is_some() { dtb_at } else { 0 });
    m.board.0.borrow_mut().eth.peer = peer;
    if timing {
        let mut t = if dcache {
            vreteno32::model::Timing::board_cached()
        } else {
            vreteno32::model::Timing::board()
        };
        if let Some(n) = ddr3_store {
            for r in &mut t.regions {
                if r.0 == 0x4000_0000 {
                    r.3 = n;
                }
            }
        }
        m.model.timing = Some(t);
    }
    if profiling {
        m.profile = Some(Default::default());
        m.profile1 = Some(Default::default());
    }
    if let Some(at) = watch {
        m.watch = Some((at, Default::default()));
    }
    if let Some(n) = fastboot {
        // A pattern rather than zeros, so a byte that lands in the wrong
        // place is a byte that is wrong.
        let image = (0..n).map(|i| (i * 131 + 7) as u8).collect();
        m.board.0.borrow_mut().eth.client =
            Some(vreteno32::machine::fbpeer::FbClient::new(image));
    }
    if let Some(bytes) = &loaded {
        m.board.0.borrow_mut().uart.ie = 2;
        m.type_bytes(bytes.as_bytes());
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    let mut shown = 0;
    let mut ran = 0u64;
    // A slice at a time, so the output streams without a check every
    // instruction.
    // Standard input, read on a thread of its own so that a read that
    // waits for a terminal never stops the machine; what has arrived is
    // handed to the port between slices.
    let (typed_tx, typed) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buf = [0u8; 256];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || typed_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let done = |m: &Machine| {
        m.board.0.borrow().eth.client.as_ref().is_some_and(|c| {
            matches!(
                c.step,
                vreteno32::machine::fbpeer::Step::Done
                    | vreteno32::machine::fbpeer::Step::Failed
            )
        })
    };
    let said = |m: &Machine| {
        until.as_ref().is_some_and(|t| {
            let sent = &m.board.0.borrow().uart.sent;
            sent.windows(t.len()).any(|w| w == t.as_bytes())
        })
    };
    // Finer slices when stopping at a line, so the count is close to
    // where the line was said.
    let slice = if until.is_some() { 10_000 } else { 100_000 };
    while ran < steps && m.model.halted.is_none() && !done(&m) && !said(&m) {
        while let Ok(bytes) = typed.try_recv() {
            m.type_bytes(&bytes);
        }
        ran += m.run((steps - ran).min(slice));
        let sent = &m.board.0.borrow().uart.sent;
        if sent.len() > shown {
            out.write_all(&sent[shown..]).ok();
            out.flush().ok();
            shown = sent.len();
        }
    }
    let how = match m.model.halted {
        Some(h) => format!("halted ({h:?})"),
        None if done(&m) => "stopped when the download ended".to_string(),
        None if said(&m) => {
            format!(
                "stopped once the console said {:?}",
                until.as_ref().unwrap()
            )
        }
        None => "stopped at the step limit".to_string(),
    };
    eprintln!("\n{how} after {ran} instructions, pc {:#010x}", m.model.pc);
    // Each ring of Razboj's doorbell, and where the scanout showed from
    // as it came (issue 1551).
    if rings {
        for (i, (word, shown, rows)) in
            m.board.0.borrow().rings.iter().enumerate()
        {
            let shown =
                shown.map_or("none".to_string(), |b| format!("{b:#010x}"));
            let rows = rows.map_or(String::new(), |(lo, hi)| {
                format!(" rows {lo} to {hi}")
            });
            eprintln!("ring {i}: {word:#010x}{rows} shown {shown}");
        }
    }
    // Where the second hart is, once anything started it (issue 1408).
    if m.model1.minstret != 0 {
        let state = if m.parked1 {
            "waiting to start"
        } else {
            "running"
        };
        eprintln!(
            "hart 1: {state} after {} instructions, pc {:#010x}",
            m.model1.minstret, m.model1.pc
        );
    }
    if let Some(path) = &screen {
        let d = m.board.0.borrow();
        match d.scanout() {
            Some(base) => {
                // The flagship's mode: 640 by 480, a word a pixel, the
                // low 24 bits red, green and blue, 4096 bytes a line.
                let (w, h, stride) = (640u32, 480u32, 4096u32);
                let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                for y in 0..h {
                    for x in 0..w {
                        let p = d.ddr.load(base + y * stride + 4 * x);
                        ppm.extend([(p >> 16) as u8, (p >> 8) as u8, p as u8]);
                    }
                }
                std::fs::write(path, ppm)
                    .unwrap_or_else(|e| panic!("{path}: {e}"));
                eprintln!("screen: the scanout at {base:#010x}, in {path}");
            }
            None => eprintln!("screen: the scanout is not shown"),
        }
    }
    // Hart 0's in FILE, and hart 1's beside it in FILE.hart1 once it ran
    // (issue 1408).
    let hart1 = profile.as_ref().map(|p| format!("{p}.hart1"));
    for (path, p) in [(&profile, &m.profile), (&hart1, &m.profile1)] {
        let (Some(path), Some(p)) = (path, p) else {
            continue;
        };
        if p.is_empty() {
            continue;
        }
        let mut rows: Vec<_> = p.iter().collect();
        rows.sort();
        let text: String = rows
            .iter()
            .map(|(pc, (c, n))| format!("{pc:08x} {c} {n}\n"))
            .collect();
        std::fs::write(path, text).unwrap_or_else(|e| panic!("{path}: {e}"));
        eprintln!("profile: {} addresses, in {path}", rows.len());
    }
    if let Some((at, callers)) = &m.watch {
        let mut rows: Vec<_> = callers.iter().collect();
        rows.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
        for (ra, (n, sum)) in rows {
            eprintln!("watch {at:#010x}: from {ra:#010x}, {n} times, a2 summing {sum}");
        }
    }
    if timing {
        eprintln!("timing: {} cycles", m.model.cycles);
    }
    if fastboot.is_some() {
        let d = m.board.0.borrow();
        let c = d.eth.client.as_ref().expect("the client");
        match (c.began, c.ended) {
            (Some(b), Some(e)) => {
                eprintln!(
                    "fastboot: {} bytes in {} {}, {:.1} a byte; {} \
                     segments, {} retransmits; the port dropped {} frames",
                    c.image.len(),
                    e - b,
                    // The client's clock: steps, or in the timing mode
                    // cycles (issue 1392).
                    if timing { "cycles" } else { "steps" },
                    (e - b) as f64 / c.image.len() as f64,
                    c.segments,
                    c.retransmits,
                    d.eth.dropped
                );
                // What the server staged, read back from where its
                // overlay puts the staging area.
                let staged = d.ddr.get(STAGE, c.image.len() as u32);
                eprintln!(
                    "fastboot: the staged image is {}",
                    if staged == c.image { "intact" } else { "WRONG" }
                );
            }
            _ => eprintln!(
                "fastboot: no OKAY ({:?}); answers {:?}; {} retransmits; \
                 the port dropped {} frames",
                c.step, c.answers, c.retransmits, d.eth.dropped
            ),
        }
    }
    if peer {
        let d = m.board.0.borrow();
        eprintln!(
            "eth: {} frames sent, {} received, {} dropped",
            d.eth.sent.len(),
            d.eth.received,
            d.eth.dropped
        );
    }
}
