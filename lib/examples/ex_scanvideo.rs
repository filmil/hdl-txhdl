// SPDX-License-Identifier: Apache-2.0
//! The video peripheral with a scanout beside it, programmed over its
//! slot as a host would program it on the board (issue 151).
//!
//! ```text
//!   Rom -- AxiPer -- AxiHost -- LineFetch ----words---> ScanVideo
//!                                  ^                     |    ^
//!                              ScanFetch <-----req-------+    |
//!                                                  Prog --slot+
//! ```
//!
//! `ScanVideo` is what the board's third slot holds: `Hdmi`, unchanged,
//! below `0x80`, and the scanout's registers from `0x80`, with a second
//! count of the beam for the line pair and a multiplexer on the pins.
//! `Prog` is the host, an AXI-Lite master the testbench tells what to
//! write and read. Both sides run on one clock here; the crossing
//! between the bus clock and the pixel clock is `ex_scanpair`'s, and on
//! the board it is in the top.
//!
//! The run paints one pixel of the framebuffer and shows it, then sets
//! the frame's base and the bit that shows the scanout, and checks
//! every visible pixel of two frames as the word at its place in
//! memory. A frame whose memory
//! stalls for a whole line sets the underflow bit, read over the slot;
//! a write clears it, and a read after a clean frame finds it clear. On
//! every cycle, and across a reset in the middle of a frame, the line
//! pair's count of the beam is `Hdmi`'s.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, set_reset, signal, Clock, DefaultClock, In, Mem, Reg,
    Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{with, Trace};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort, Resp, R};
use txhdl_parts::bus::axi_lite::{
    LiteAr, LiteAw, LiteB, LitePort, LiteR, LiteW,
};
use txhdl_parts::dma::LineFetch;
use txhdl_parts::scanout::{scan, ScanFetch, ScanVideo};

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
const BEATS: usize = 16;
/// The raster: eight visible columns of sixty-four, four visible rows
/// of seven.
const HV: usize = 8;
const HFP: usize = 16;
const HSW: usize = 16;
const HBP: usize = 24;
const VV: usize = 4;
const VFP: usize = 1;
const VSW: usize = 1;
const VBP: usize = 1;
const HT: usize = HV + HFP + HSW + HBP;
const TOTAL: usize = VV + VFP + VSW + VBP;
const AW: usize = 3;
const STRIDE: usize = HV * 4;
/// The frame's first word in memory.
const BASE_WORD: usize = 16;
const N: usize = 256;
/// The memory's bytes, outside which no line is asked for.
const MEM: usize = N * 4;
/// The one pixel the host paints in the framebuffer, and its colour.
const DOT: (usize, usize) = (3, 2);
const FB: u32 = 0xabc;
/// The scanout's registers, at the upper half of the slot.
const SCAN: u32 = 0x80;

type Video = ScanVideo<
    HV,
    HFP,
    HSW,
    HBP,
    VV,
    VFP,
    VSW,
    VBP,
    0,
    AW,
    TOTAL,
    STRIDE,
    0,
    MEM,
>;
type Fetch = ScanFetch<ADDR, 16, HV>;

/// The memory, whose beats a `hold` stalls, as in `ex_scanpair`.
#[derive(Trace, Default)]
pub struct Rom<const A: usize, const I: usize, const M: usize> {
    pub px: Mem<U<32>, M>,
    pub busy: Reg<Bit>,
    pub at: Reg<U<16>>,
    pub left: Reg<U<9>>,
    pub rid: Reg<U<I>>,
}

impl<const A: usize, const I: usize, const M: usize>
    Unit<(PerPort<A, 32, 4, I>, In<Bit>), ()> for Rom<A, I, M>
{
    async fn run(
        &mut self,
        (bus, hold): (PerPort<A, 32, 4, I>, In<Bit>),
        _o: (),
    ) {
        loop {
            DefaultClock::rising().await;
            let q = bus.req.head();
            let offered = Bit::from(bus.req.peek().is_some());
            let busy = self.busy.get();
            let mask = U::<16>::from((M - 1) as u32);
            let qat = (q.addr >> 2u32).resize::<16>() & mask;
            let take = offered & q.read & !busy;
            let _ = bus.req.recv_if(take);
            let beat = busy & !hold.get() & bus.r.ready();
            let last = beat & (self.left.get() == 1);
            with!(self <= {
                take ? {
                    busy: Bit::One,
                    at: qat,
                    left: (q.len.resize::<9>() + 1),
                    rid: q.id,
                },
                beat ? {
                    at: (self.at.get() + 1) & mask,
                    left: self.left.get() - 1,
                },
                last ? busy: Bit::Zero,
            });
            if beat.to_bool() {
                bus.r.send(R {
                    id: self.rid.get(),
                    data: self.px.read(self.at.get().slice::<0, 8>()),
                    resp: Resp::Okay,
                    last,
                });
            }
        }
    }
}

/// The host on the slot: a write when `wgo` is high and a read when
/// `rgo` is, one at a time, and the last word read on `got`.
#[derive(Trace, Default)]
pub struct Prog {
    pub got: Reg<U<32>>,
}

impl
    Unit<
        (
            In<Bit>,
            In<U<32>>,
            In<U<32>>,
            In<Bit>,
            In<U<32>>,
            Rx<LiteB>,
            Rx<LiteR<32>>,
        ),
        (Tx<LiteAw<32>>, Tx<LiteAr<32>>, Tx<LiteW<32, 4>>),
    > for Prog
{
    async fn run(
        &mut self,
        (wgo, waddr, wdata, rgo, raddr, b, r): (
            In<Bit>,
            In<U<32>>,
            In<U<32>>,
            In<Bit>,
            In<U<32>>,
            Rx<LiteB>,
            Rx<LiteR<32>>,
        ),
        (aw, ar, w): (Tx<LiteAw<32>>, Tx<LiteAr<32>>, Tx<LiteW<32, 4>>),
    ) {
        // The write answers are drained, and the last word read is
        // kept in `got`, which the testbench reads.
        loop {
            DefaultClock::rising().await;
            let _ = b.recv_if(true);
            if let Some(word) = r.recv_if(true) {
                self.got.set(word.data);
            }
            if (wgo.get() & aw.ready() & w.ready()).to_bool() {
                aw.send(LiteAw {
                    addr: waddr.get(),
                    prot: U::<3>::from(0u8),
                });
                w.send(LiteW {
                    data: wdata.get(),
                    strb: U::<4>::from(0xfu8),
                });
            }
            if (rgo.get() & ar.ready()).to_bool() {
                ar.send(LiteAr {
                    addr: raddr.get(),
                    prot: U::<3>::from(0u8),
                });
            }
        }
    }
}

fn main() {
    let link = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = link.host_client;
    let bus: PerPort<ADDR, 32, 4, IDB> = link.per_client.into();
    let (hold_o, hold) = signal::<Bit, DefaultClock>();
    let (at_o, at) = signal::<U<ADDR>, DefaultClock>();
    let (count_o, count) = signal::<U<16>, DefaultClock>();
    let (start_o, start) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    let (words_tx, words) = chan::<U<32>, DefaultClock>();
    let (req, lines) = chan::<U<32>, DefaultClock>();
    // The slot.
    let (aw_tx, aw) = chan::<LiteAw<32>, DefaultClock>();
    let (ar_tx, ar) = chan::<LiteAr<32>, DefaultClock>();
    let (w_tx, w) = chan::<LiteW<32, 4>, DefaultClock>();
    let (b, b_rx) = chan::<LiteB, DefaultClock>();
    let (r, r_rx) = chan::<LiteR<32>, DefaultClock>();
    let (rgb_o, rgb) = signal::<U<24>, DefaultClock>();
    let (hs_o, hsync) = signal::<Bit, DefaultClock>();
    let (vs_o, vsync) = signal::<Bit, DefaultClock>();
    let (de_o, de) = signal::<Bit, DefaultClock>();
    // What the testbench tells the host.
    let (wgo_o, wgo) = signal::<Bit, DefaultClock>();
    let (waddr_o, waddr) = signal::<U<32>, DefaultClock>();
    let (wdata_o, wdata) = signal::<U<32>, DefaultClock>();
    let (rgo_o, rgo) = signal::<Bit, DefaultClock>();
    let (raddr_o, raddr) = signal::<U<32>, DefaultClock>();

    let image: Vec<U<32>> = (0..N).map(|i| U::<32>::from(i as u32)).collect();
    let mut rom = Rom::<ADDR, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut fetch = LineFetch::<ADDR, IDB, BEATS, 16>::default();
    let mut sfetch = Fetch::default();
    let mut video = Video::default();
    let mut prog = Prog::default();
    // The two counts of the beam, watched from here.
    let (rhc, rvc) = (video.raster.hc, video.raster.vc);
    let (hhc, hvc) = (video.hdmi.hc, video.hdmi.vc);
    let got_reg = prog.got;

    if let Some(mut wv) = Wave::from_env() {
        wv.clock::<DefaultClock>();
        wv.add("bus_aw", &aw);
        wv.add("bus_ar", &ar);
        wv.add("bus_w", &w);
        wv.add("bus_b", &b);
        wv.add("bus_r", &r);
        wv.add("words", &words);
        wv.add("req", &req);
        wv.add("rgb", &rgb);
        wv.add("hsync", &hsync);
        wv.add("vsync", &vsync);
        wv.add("de", &de);
        wv.add("scanvideo", &video);
        wv.start();
    }

    hold_o.set(Bit::Zero);
    wgo_o.set(Bit::Zero);
    rgo_o.set(Bit::Zero);
    waddr_o.set(U::<32>::from(0u8));
    wdata_o.set(U::<32>::from(0u8));
    raddr_o.set(U::<32>::from(0u8));

    let mut sim = Running::new(join2(
        join2(
            join2(
                host.run(link.host_in, link.host_out),
                per.run(link.per_in, link.per_out),
            ),
            join2(
                rom.run((bus, hold), ()),
                fetch.run(
                    (grant, done, rdata, at, count, start),
                    (issue, release, words_tx, run_o),
                ),
            ),
        ),
        join2(
            join2(
                sfetch.run((lines, running), (at_o, count_o, start_o)),
                video.run(
                    LitePort { aw, ar, w, b, r },
                    (words, req, rgb_o, hs_o, vs_o, de_o),
                ),
            ),
            prog.run(
                (wgo, waddr, wdata, rgo, raddr, b_rx, r_rx),
                (aw_tx, ar_tx, w_tx),
            ),
        ),
    ));
    let _ = wbeat;

    // The beam, as the testbench keeps it: one step a pixel.
    let (mut x, mut y, mut frames) = (0usize, 0usize, 0usize);
    let mut last: Option<(usize, usize)> = None;
    let (mut fb_ok, mut fb_bad) = (0usize, 0usize);
    let (mut scan_ok, mut scan_bad) = (0usize, 0usize);
    let mut status = Vec::new();
    let mut lockstep = 0usize;
    let mut reset_at = None;
    let mut reading = false;
    while frames < 7 {
        let t = now();
        if !DefaultClock::rising_at(t) {
            sim.step();
            continue;
        }
        // What the host does at this pixel, by the frame it is in.
        let at_start = x == 1 && y == 0;
        let at_end = x == 1 && y == TOTAL - 1;
        let (wr, rd): (Option<(u32, u32)>, Option<u32>) = match frames {
            0 if at_start => {
                (Some((SCAN + scan::base, (BASE_WORD * 4) as u32)), None)
            }
            // `Hdmi`'s cursor, then a pixel there, below `0x80`.
            0 if x == 10 && y == 0 => {
                (Some((4, (DOT.0 | DOT.1 << 8) as u32)), None)
            }
            0 if x == 20 && y == 0 => (Some((8, FB)), None),
            // The bit that shows the scanout, a blanking row before the
            // last: the pair asks for nothing until it is shown, and it
            // asks for frame 1's first line as the last row starts.
            0 if x == 1 && y == TOTAL - 2 => {
                (Some((SCAN + scan::ctrl, 1)), None)
            }
            2..=4 if at_end => (None, Some(SCAN + scan::status)),
            4 if at_start => (Some((SCAN + scan::clear, 1)), None),
            _ => (None, None),
        };
        wgo_o.set(Bit::from_bool(wr.is_some()));
        if let Some((a, d)) = wr {
            waddr_o.set(U::<32>::from(a));
            wdata_o.set(U::<32>::from(d));
        }
        rgo_o.set(Bit::from_bool(rd.is_some()));
        if let Some(a) = rd {
            raddr_o.set(U::<32>::from(a));
            reading = true;
        }
        // Frame 3 stalls the memory for the whole of row 1; every
        // frame stalls it briefly in every line.
        hold_o.set(Bit::from_bool(x == 4 || (frames == 3 && y == 1)));
        // A reset for one edge, in the middle of frame 5.
        let rst = frames == 5 && y == 2 && x == 20 && reset_at.is_none();
        set_reset(rst);
        if rst {
            reset_at = Some(t);
        }
        sim.step();
        // The two counts agree, on every cycle.
        assert_eq!(rhc.get().raw(), hhc.get().raw(), "columns at {t}");
        assert_eq!(rvc.get().raw(), hvc.get().raw(), "rows at {t}");
        lockstep += 1;
        // The pixel on the pins is the one the beam was at an edge ago.
        if de.get().to_bool() {
            let (px, py) = last.expect("a pixel follows a column");
            let shown = rgb.get().raw();
            if frames == 0 && py > 0 {
                let want = if (px, py) == DOT { 0xaabbcc } else { 0 };
                if shown == want {
                    fb_ok += 1;
                } else {
                    fb_bad += 1;
                }
            }
            if frames == 1 || frames == 2 {
                if shown == (BASE_WORD + py * HV + px) as u128 {
                    scan_ok += 1;
                } else {
                    scan_bad += 1;
                }
            }
        }
        if reading && x == 40 {
            status.push(got_reg.get().raw() & 1 == 1);
            reading = false;
        }
        if rst {
            // The reset puts both counts back at the start of a frame.
            set_reset(false);
            x = 0;
            y = 0;
            last = None;
            continue;
        }
        last = Some((x, y));
        x += 1;
        if x == HT {
            x = 0;
            y += 1;
            if y == TOTAL {
                y = 0;
                frames += 1;
            }
        }
    }
    stop();
    println!("framebuffer pixels in frame 0: {fb_ok} right, {fb_bad} wrong");
    println!(
        "scanout pixels in frames 1-2:  {scan_ok} right, {scan_bad} wrong"
    );
    println!("underflow read after frames 2, 3 and 4: {status:?}");
    let rt = reset_at.expect("the run reset the design once");
    println!("cycles the two counts agreed:  {lockstep}");
    println!("the reset among them, at tick: {rt}");
    assert_eq!(fb_bad, 0, "the framebuffer shows until the bit is set");
    assert_eq!(fb_ok, (VV - 1) * HV, "frame 0's rows after the first");
    assert_eq!(scan_bad, 0, "every scanout pixel is its word");
    assert_eq!(scan_ok, 2 * VV * HV, "two frames of scanout pixels");
    assert_eq!(status, [false, true, false], "set by the stall, cleared");
    let net = Video::lowered("scanvideo");
    txhdl::netlist::write_netlists_from_env(&[&net]);
    print!(
        "\n{}",
        net.verilog()
            .lines()
            .take(40)
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!();
}
