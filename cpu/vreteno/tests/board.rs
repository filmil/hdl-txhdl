// SPDX-License-Identifier: Apache-2.0
//! The board's design, as one lowered unit, run on the programs
//! compiled for the core: the greeting, which uses the data memory and
//! the serial port; the DDR3 test, which uses the memory's region
//! through the bridge and the controller's model; and input by
//! interrupt, which takes the serial port's receive interrupt through
//! the interrupt controller; and the remote peripheral, which the core
//! reaches as frames on the Ethernet port with a program in this file
//! answering them. And its netlist, which holds the memory controller
//! as a foreign module.
use std::collections::{HashMap, VecDeque};
use txhdl::comp::{
    chan, join2, pad, set_reset, signal, DefaultClock, In, Out, Running, Rx,
    Tx, Unit,
};
use txhdl::map::AddrMap;
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
use txhdl_parts::bus::axi_pins::AxiHostPins;
use txhdl_parts::dtm::Tck;
use txhdl_parts::eth::EthByte;
use txhdl_parts::hdmi::Raster;
use txhdl_parts::mdio::sim::MdioPhy;
use txhdl_parts::remote::eth::{FRAME_LEN, KIND_ANSWER, KIND_ASK};
use txhdl_parts::scanout::LinePair;
use txhdl_parts::sd::SdCard;
use txhdl_parts::spi::FlashDevice;
use vreteno32::board::{
    Board, BoardIn, BoardMap, BoardOut, SlotMap, REMOTE_DEV,
};
use vreteno32::dmem::Dmem;
use vreteno32::hart::Hart;
use vreteno32::rom::Rom;
use vreteno32::term::Terminal;

// The icosahedron's lists, which `cpu/vreteno/rust/ico_hdmi.rs` sends,
// for the timing of Razboj drawing them (issue 1255); the board
// program's constants this file has no use for go unused.
#[allow(dead_code)]
#[path = "../rust/ico_list.rs"]
mod ico_list;

/// The serial port's divider in these runs: four cycles a bit, as the
/// demonstration has it.
type TestBoard = Board<4>;

/// What a run came to: what the serial line said, and the cycle the
/// core halted on.
struct Ran {
    said: String,
    /// How many bytes of the stream the terminal managed to type, the
    /// cycle the last one went out on, and how long the run was.
    typed: usize,
    typed_at: u64,
    ran_for: u64,
    halted_at: Option<u64>,
    /// Every frame that left the Ethernet port, whether or not a
    /// program was there to answer it.
    sent: Vec<Vec<u8>>,
    /// What the debugger on the JTAG cable read, in order, and how
    /// many steps of its plan it got through.
    got: Vec<u32>,
    steps: usize,
    /// The commands the configuration flash took whole, and those let
    /// go of short.
    flash: Vec<u8>,
    flash_short: u32,
    /// The management frames the PHY took: its address, the
    /// register's, and the word for a write.
    phy_frames: Vec<(u8, u8, Option<u16>)>,
    /// The PHY's page select, register 31, when the run ended.
    phy_page: u16,
    /// The card in the SD slot as the run left it.
    card: SdCard,
    /// The scanout's lines, when the run had one.
    scan: ScanLog,
    /// What each burst of the debugger's plan met, in order.
    bursts: Vec<BurstSeen>,
    /// The cycle a reset asked for began, when it did.
    reset_from: Option<u64>,
}

/// Run `text` with `data` in the data memory, on the board's design,
/// for at most `limit` cycles, with a terminal on the serial line that
/// types `reply` once the core has said a line.
fn run(text: &[u32], data: &[u8], reply: &[u8], limit: u64) -> Ran {
    run_paced(text, data, reply, &[], limit)
}

/// The same, with a program on the other side of the Ethernet port
/// answering the remote peripheral's frames.
fn run_served(text: &[u32], data: &[u8], limit: u64) -> Ran {
    let net = Net {
        serve: true,
        ..Default::default()
    };
    run_all(text, data, b"", &[], limit, net, &[])
}

/// The same, with a debugger on the JTAG cable following `plan`.
fn run_debugged(text: &[u32], data: &[u8], limit: u64, plan: &[Op]) -> Ran {
    run_all(text, data, b"", &[], limit, Net::default(), plan)
}

/// The program at the other end of the wire, for one cycle: it reads
/// the frames the peripheral sends, keeps what it is told to keep,
/// refuses a read of an address nothing has written, and answers with
/// the frame it was sent, four bytes of it changed. `//tools/remote`
/// does the same thing in Go on another machine; this is the same
/// protocol with the wire left out.
fn device(
    from: &Rx<EthByte>,
    to: &Tx<EthByte>,
    frame: &mut Vec<u8>,
    reply: &mut Vec<EthByte>,
    words: &mut HashMap<u32, u32>,
    sent: &mut Vec<Vec<u8>>,
    serve: bool,
) {
    if !reply.is_empty() && to.ready().to_bool() {
        to.send(reply.remove(0));
    }
    let Some(byte) = from.recv() else {
        return;
    };
    frame.push(byte.data.raw() as u8);
    if !byte.last.to_bool() {
        return;
    }
    sent.push(frame.clone());
    if serve && frame.len() >= FRAME_LEN as usize {
        let at = u32::from_be_bytes(frame[18..22].try_into().unwrap());
        let data = u32::from_be_bytes(frame[22..26].try_into().unwrap());
        let (answer, err) = if frame[17] & 1 == 1 {
            words.insert(at, data);
            (0, false)
        } else {
            match words.get(&at) {
                Some(v) => (*v, false),
                None => (0, true),
            }
        };
        let mut out = frame.clone();
        out.truncate(FRAME_LEN as usize);
        out[14] = KIND_ANSWER as u8;
        out[17] = err as u8;
        out[22..26].copy_from_slice(&answer.to_be_bytes());
        let n = out.len();
        for (i, b) in out.into_iter().enumerate() {
            reply.push(EthByte {
                data: U::from(b),
                last: Bit::from_bool(i + 1 == n),
            });
        }
    }
    frame.clear();
}

/// What a debugger on the JTAG cable does during a run, one step at a
/// time, as single-beat transactions on the master's pins: a write of a
/// word, a read of one, whose answer is kept, or a wait. The master
/// model in the run drives the pins as the JTAG-to-AXI core does, one
/// transaction at a time (issue 154).
///
/// A burst is a transaction of its own, for the bus's conformance
/// rather than a debugger's (issue 1196): its address, its beats and
/// the identifier it carries. A write burst's beats carry their own
/// index.
#[derive(Clone, Copy, Debug)]
enum Op {
    Write(u32, u32),
    Read(u32),
    Wait(u64),
    ReadBurst(u32, u32, u8),
    WriteBurst(u32, u32, u8),
}

/// What a burst on the master's pins met: the beats that came back,
/// which of them were marked last, every response, whether every
/// identifier was the burst's, the write responses and whether one came
/// before the last beat went, and the cycles from the address to the
/// end. A burst that is not `done` waited out the bound.
#[derive(Clone, Debug, Default)]
struct BurstSeen {
    beats: u32,
    lasts: Vec<u32>,
    resps: Vec<u8>,
    ids_ok: bool,
    bs: u32,
    b_early: bool,
    cycles: u64,
    done: bool,
}

/// The cycles a burst may take before it counts as a hang. The slowest
/// that answer, sixteen beats of the remote peripheral each a frame on
/// the wire and back, take about 1300.
const BURST_BOUND: u64 = 20_000;

/// The pins the model drives and reads, kept out of the board's port
/// struct so the run can move them.
struct Jtag {
    awaddr: Out<U<32>>,
    awvalid: Out<Bit>,
    wdata: Out<U<32>>,
    wvalid: Out<Bit>,
    bready: Out<Bit>,
    araddr: Out<U<32>>,
    arvalid: Out<Bit>,
    rready: Out<Bit>,
    awid: Out<U<1>>,
    awlen: Out<U<8>>,
    wlast: Out<Bit>,
    arid: Out<U<1>>,
    arlen: Out<U<8>>,
    awready: In<Bit>,
    wready: In<Bit>,
    bvalid: In<Bit>,
    bid: In<U<1>>,
    bresp: In<U<2>>,
    arready: In<Bit>,
    rdata: In<U<32>>,
    rvalid: In<Bit>,
    rid: In<U<1>>,
    rresp: In<U<2>>,
    rlast: In<Bit>,
}

/// The master model's state between cycles: which op, and which of its
/// handshakes are done.
#[derive(Default)]
struct Master {
    at: usize,
    aw_done: bool,
    w_done: bool,
    ar_done: bool,
    /// What was driven valid this cycle, since a ready seen without a
    /// valid is not a handshake.
    awv: bool,
    wv: bool,
    arv: bool,
    waited: u64,
    got: Vec<u32>,
    /// A write burst's beats sent so far, and the burst in hand's
    /// record; every burst's, in order.
    sent: u32,
    seen: BurstSeen,
    bursts: Vec<BurstSeen>,
}

impl Master {
    /// Before a cycle: drive the pins for the op in hand.
    fn drive(&mut self, plan: &[Op], j: &Jtag) {
        let (mut awv, mut wv, mut arv) = (false, false, false);
        // A single beat with identifier zero unless the op says more.
        j.awlen.set(U::from(0u8));
        j.arlen.set(U::from(0u8));
        j.wlast.set(Bit::One);
        j.awid.set(U::from(0u8));
        j.arid.set(U::from(0u8));
        if let Some(op) = plan.get(self.at) {
            match *op {
                Op::ReadBurst(addr, beats, id) => {
                    j.araddr.set(U::from(addr));
                    j.arlen.set(U::from((beats - 1) as u8));
                    j.arid.set(U::from(id));
                    arv = !self.ar_done;
                }
                Op::WriteBurst(addr, beats, id) => {
                    j.awaddr.set(U::from(addr));
                    j.awlen.set(U::from((beats - 1) as u8));
                    j.awid.set(U::from(id));
                    j.wdata.set(U::from(self.sent));
                    j.wlast.set(Bit::from_bool(self.sent + 1 == beats));
                    awv = !self.aw_done;
                    wv = self.aw_done && self.sent < beats;
                }
                Op::Write(addr, data) => {
                    j.awaddr.set(U::from(addr));
                    j.wdata.set(U::from(data));
                    // The data beat after the address has gone, as
                    // the JTAG-to-AXI master sends them.
                    awv = !self.aw_done;
                    wv = self.aw_done && !self.w_done;
                }
                Op::Read(addr) => {
                    j.araddr.set(U::from(addr));
                    arv = !self.ar_done;
                }
                Op::Wait(_) => {}
            }
        }
        j.awvalid.set(Bit::from_bool(awv));
        j.wvalid.set(Bit::from_bool(wv));
        j.arvalid.set(Bit::from_bool(arv));
        self.awv = awv;
        self.wv = wv;
        self.arv = arv;
        j.bready.set(Bit::One);
        j.rready.set(Bit::One);
    }

    /// After the cycle: what the pins say happened at its edge. A
    /// valid the model held meets a ready the pins computed in the
    /// same step, so the beat went; a response present in the step was
    /// taken, since ready is always high on the model's side.
    fn observe(&mut self, plan: &[Op], j: &Jtag) {
        let Some(op) = plan.get(self.at) else {
            return;
        };
        match *op {
            Op::Write(..) => {
                if self.awv && j.awready.get().to_bool() {
                    self.aw_done = true;
                }
                if self.wv && j.wready.get().to_bool() {
                    self.w_done = true;
                }
                if self.aw_done && self.w_done && j.bvalid.get().to_bool() {
                    self.next();
                }
            }
            Op::Read(_) => {
                if self.arv && j.arready.get().to_bool() {
                    self.ar_done = true;
                }
                if self.ar_done && j.rvalid.get().to_bool() {
                    self.got.push(j.rdata.get().raw() as u32);
                    self.next();
                }
            }
            Op::Wait(n) => {
                // A beat or a response after a burst has ended is the
                // last burst's, and counts against it.
                if let Some(b) = self.bursts.last_mut() {
                    if j.rvalid.get().to_bool() {
                        if j.rlast.get().to_bool() {
                            b.lasts.push(b.beats);
                        }
                        b.beats += 1;
                    }
                    if j.bvalid.get().to_bool() {
                        b.bs += 1;
                    }
                }
                self.waited += 1;
                if self.waited >= n {
                    self.next();
                }
            }
            Op::ReadBurst(_, _, id) => {
                self.start();
                if self.arv && j.arready.get().to_bool() {
                    self.ar_done = true;
                }
                if self.ar_done && j.rvalid.get().to_bool() {
                    let k = self.seen.beats;
                    self.seen.beats += 1;
                    self.seen.resps.push(j.rresp.get().raw() as u8);
                    if j.rid.get().raw() as u8 != id {
                        self.seen.ids_ok = false;
                    }
                    if j.rlast.get().to_bool() {
                        self.seen.lasts.push(k);
                        self.seen.done = true;
                    }
                }
                self.end_burst();
            }
            Op::WriteBurst(_, beats, id) => {
                self.start();
                if self.awv && j.awready.get().to_bool() {
                    self.aw_done = true;
                }
                if self.wv && j.wready.get().to_bool() {
                    self.sent += 1;
                }
                if j.bvalid.get().to_bool() {
                    self.seen.bs += 1;
                    self.seen.resps.push(j.bresp.get().raw() as u8);
                    if j.bid.get().raw() as u8 != id {
                        self.seen.ids_ok = false;
                    }
                    if self.sent < beats {
                        self.seen.b_early = true;
                    }
                    self.seen.done = true;
                }
                self.end_burst();
            }
        }
    }

    /// A burst's first cycle starts its record.
    fn start(&mut self) {
        if self.seen.cycles == 0 {
            self.seen.ids_ok = true;
        }
        self.seen.cycles += 1;
    }

    /// A burst ends when its last beat or its response is seen, or
    /// when it has waited out the bound.
    fn end_burst(&mut self) {
        if self.seen.done || self.seen.cycles >= BURST_BOUND {
            let mut s = std::mem::take(&mut self.seen);
            s.beats = s.beats.max(self.sent);
            self.bursts.push(s);
            self.next();
        }
    }

    fn next(&mut self) {
        self.at += 1;
        self.aw_done = false;
        self.w_done = false;
        self.ar_done = false;
        self.waited = 0;
        self.sent = 0;
    }
}

/// The same, with the terminal typing in blocks of `block` bytes and
/// waiting for a byte back between them, which is how a sender talks
/// to the loader.
fn run_paced(
    text: &[u32],
    data: &[u8],
    reply: &[u8],
    blocks: &[usize],
    limit: u64,
) -> Ran {
    run_all(text, data, reply, blocks, limit, Net::default(), &[])
}

/// The run itself. `serve` says whether a program answers the frames
/// the remote peripheral sends; without one its port is a wire with
/// nothing at the other end, which is what every other run here wants.
/// What is at the other end of the Ethernet port during a run.
#[derive(Default)]
struct Net<'a> {
    /// A program answering the remote peripheral's frames.
    serve: bool,
    /// Frames put on the wire from the start, unasked.
    inject: &'a [Vec<u8>],
    /// The PHY's management side, on the board's MDIO line:
    /// `board_phy()` when none is given.
    phy: Option<MdioPhy>,
    /// The card in the SD slot: `SdCard::default()` when none is given.
    card: Option<SdCard>,
    /// A scanout on `scan_req` and `scan_words` (issue 1178).
    scan: Option<Scan>,
    /// Whether the third slot answers, as the flagship's video
    /// peripheral and scanout would, rather than being tied off (issue
    /// 1196): a write is taken and kept, a read gives a kept word back,
    /// or zero, and the video peripheral's status word, at offset zero,
    /// reads its blanking bit high for 400 cycles in every 4000 (issue
    /// 996).
    video: bool,
    /// Whether the run ends once the debugger's plan is done, rather
    /// than when the core halts or the limit is reached.
    until_planned: bool,
    /// A reset in the middle of the run, from the first cycle for the
    /// second, as the serial line held low gives the flagship (issue
    /// 1317): the design's reset and its registers', while the DDR3
    /// model, whose state is the controller's and not a register, keeps
    /// what it was doing, as MIG does, which that reset does not reach.
    reset_at: Option<(u64, u64)>,
}

/// A scanout's pixel side on the board's `scan_req` and `scan_words`:
/// the flagship's `LinePair` and a raster to drive it, on the board's
/// clock, with frames of six rows so that a run sees several. A row is
/// as long in the board's cycles as the flagship's is, [`SCAN_LINE`],
/// so a line has as long to arrive as on the board (issue 1209). From the reset it has no base, as on the board after every
/// reset; at `show_at` a program gives it `base`.
struct Scan {
    base: u32,
    show_at: u64,
}

/// The flagship's line in the board's cycles: 800 columns of the 25.2
/// MHz pixel clock, at 100 MHz.
const SCAN_LINE: u64 = 3175;

/// The raster and the pair: the flagship's line of 640 words, 4096
/// bytes apart, a row of [`SCAN_LINE`] cycles, its back porch long
/// enough to make it so, and four visible rows of six. The 640 words
/// are read here a word a cycle rather than one every four, so the tail
/// of a line is held to a harder deadline than on the board.
type ScanRaster = Raster<640, 16, 96, 2423, 4, 1, 1, 0, 10>;
const _: () = assert!(640 + 16 + 96 + 2423 == SCAN_LINE as usize);
type ScanPair = LinePair<640, 10, 4, 6, 4096, DefaultClock>;

/// What a scanout asked for and when its words came: the address, the
/// cycle the board took the request, and the cycle its last word
/// arrived.
#[derive(Default)]
struct ScanLog {
    lines: Vec<(u32, u64, Option<u64>)>,
    /// The cycle the pair said a line was stuck, and that line's
    /// address (issue 1197).
    stuck: Option<(u64, u32)>,
    /// The first cycle the pair said a column was shown before its word
    /// had arrived (issue 1209).
    starved: Option<u64>,
}

fn run_all(
    text: &[u32],
    data: &[u8],
    reply: &[u8],
    blocks: &[usize],
    limit: u64,
    net: Net,
    plan: &[Op],
) -> Ran {
    let Net {
        serve,
        inject,
        phy,
        card,
        scan,
        video,
        until_planned,
        reset_at,
    } = net;
    // The scanout's two channels to the board, each tapped here so the
    // run sees what was asked for and what came back.
    let (scan_req_tx, scan_req) = chan::<U<32>, DefaultClock>();
    let (scan_words, scan_words_rx) = chan::<U<32>, DefaultClock>();
    let (pair_req, pair_req_rx) = chan::<U<32>, DefaultClock>();
    let (pair_inp_tx, pair_inp) = chan::<U<32>, DefaultClock>();
    let (col_o, col) = signal::<U<10>, DefaultClock>();
    let (vis_o, vis) = signal::<Bit, DefaultClock>();
    let (line_o, line) = signal::<Bit, DefaultClock>();
    let (row_o, row) = signal::<U<12>, DefaultClock>();
    let (frame_o, frame) = signal::<Bit, DefaultClock>();
    let (base_o, base) = signal::<U<32>, DefaultClock>();
    let (clear_o, clear) = signal::<Bit, DefaultClock>();
    let (show_o, show) = signal::<Bit, DefaultClock>();
    let (pix_o, _pix) = signal::<U<32>, DefaultClock>();
    let (starved_o, starved) = signal::<Bit, DefaultClock>();
    let (stuck_o, stuck) = signal::<Bit, DefaultClock>();
    let (stuck_at_o, stuck_at) = signal::<U<32>, DefaultClock>();
    let mut raster = ScanRaster::default();
    let mut pair = ScanPair::default();
    base_o.set(U::<32>::from(0u32));
    clear_o.set(Bit::Zero);
    show_o.set(Bit::Zero);
    let mut scan_log = ScanLog::default();
    let mut scan_got = 0usize;
    let mut scan_words_in = 0u64;
    // The third slot's channels, answered below when `video` says so,
    // and the words written to it.
    let (vaw_tx, vaw_rx) = chan::<LiteAw<32>, DefaultClock>();
    let (var_tx, var_rx) = chan::<LiteAr<32>, DefaultClock>();
    let (vw_tx, vw_rx) = chan::<LiteW<32, 4>, DefaultClock>();
    let (vb_tx, vb_rx) = chan::<LiteB, DefaultClock>();
    let (vr_tx, vr_rx) = chan::<LiteR<32>, DefaultClock>();
    let mut vwords: HashMap<u32, u32> = HashMap::new();
    let mut board = TestBoard {
        cpu: Hart::with(text),
        rom: Rom::with(text),
        dmem: Dmem::with(data),
        ..Default::default()
    };
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (irq_o, irq) = signal::<Bit, DefaultClock>();
    let (rx_o, rx) = signal::<Bit, DefaultClock>();
    let quiet = || signal::<Bit, DefaultClock>().1;
    let bit = || signal::<Bit, DefaultClock>().0;
    let (halt_o, halt) = signal::<Bit, DefaultClock>();
    let (tx_o, tx) = signal::<Bit, DefaultClock>();
    // The Ethernet port, as the board sees it: the frames the remote
    // peripheral sends and the frames that answer them. There is no
    // MAC in these runs, so a byte goes out a cycle and the program
    // below reads them as they arrive.
    let (net_out_tx, net_out_rx) = chan::<EthByte, DefaultClock>();
    let (net_in_tx, net_in_rx) = chan::<EthByte, DefaultClock>();
    // The third slot of the page at `0x3000` is tied off: nothing in
    // these runs writes to `0x3200`, and a run that did would wait on
    // an answer that never comes, which is what the board top's own
    // tie-off does as well.
    // The JTAG master's pins, with no master on them: every valid low
    // and nothing else read.
    let lo = || signal::<Bit, DefaultClock>().1;
    let (awaddr_o, awaddr) = signal::<U<32>, DefaultClock>();
    let (awvalid_o, awvalid) = signal::<Bit, DefaultClock>();
    let (wdata_o, wdata) = signal::<U<32>, DefaultClock>();
    let (wvalid_o, wvalid) = signal::<Bit, DefaultClock>();
    let (bready_o, bready) = signal::<Bit, DefaultClock>();
    let (araddr_o, araddr) = signal::<U<32>, DefaultClock>();
    let (arvalid_o, arvalid) = signal::<Bit, DefaultClock>();
    let (rready_o, rready) = signal::<Bit, DefaultClock>();
    let (awready_o, awready) = signal::<Bit, DefaultClock>();
    let (wready_o, wready) = signal::<Bit, DefaultClock>();
    let (bvalid_o, bvalid) = signal::<Bit, DefaultClock>();
    let (arready_o, arready) = signal::<Bit, DefaultClock>();
    let (rdata_o, rdata) = signal::<U<32>, DefaultClock>();
    let (rvalid_o, rvalid) = signal::<Bit, DefaultClock>();
    // A word wide and incrementing, as the master sends: a single beat
    // with identifier zero, unless a burst of the plan says otherwise.
    let (awid_o, awid) = signal::<U<1>, DefaultClock>();
    let (awlen_o, awlen) = signal::<U<8>, DefaultClock>();
    let (awsize_o, awsize) = signal::<U<3>, DefaultClock>();
    let (awburst_o, awburst) = signal::<U<2>, DefaultClock>();
    let (wstrb_o, wstrb) = signal::<U<4>, DefaultClock>();
    let (wlast_o, wlast) = signal::<Bit, DefaultClock>();
    let (arid_o, arid) = signal::<U<1>, DefaultClock>();
    let (arlen_o, arlen) = signal::<U<8>, DefaultClock>();
    let (arsize_o, arsize) = signal::<U<3>, DefaultClock>();
    let (arburst_o, arburst) = signal::<U<2>, DefaultClock>();
    let (bid_o, bid) = signal::<U<1>, DefaultClock>();
    let (bresp_o, bresp) = signal::<U<2>, DefaultClock>();
    let (rid_o, rid) = signal::<U<1>, DefaultClock>();
    let (rresp_o, rresp) = signal::<U<2>, DefaultClock>();
    let (rlast_o, rlast) = signal::<Bit, DefaultClock>();
    awsize_o.set(U::from(2u8));
    awburst_o.set(U::from(1u8));
    wstrb_o.set(U::from(0xfu8));
    arsize_o.set(U::from(2u8));
    arburst_o.set(U::from(1u8));
    let jtag = Jtag {
        awaddr: awaddr_o,
        awvalid: awvalid_o,
        wdata: wdata_o,
        wvalid: wvalid_o,
        bready: bready_o,
        araddr: araddr_o,
        arvalid: arvalid_o,
        rready: rready_o,
        awid: awid_o,
        awlen: awlen_o,
        wlast: wlast_o,
        arid: arid_o,
        arlen: arlen_o,
        awready,
        wready,
        bvalid,
        bid,
        bresp,
        arready,
        rdata,
        rvalid,
        rid,
        rresp,
        rlast,
    };
    jtag.awlen.set(U::from(0u8));
    jtag.arlen.set(U::from(0u8));
    jtag.wlast.set(Bit::One);
    jtag.awid.set(U::from(0u8));
    jtag.arid.set(U::from(0u8));
    // The configuration flash, as the board has it: the chip's identity,
    // and the start of a bitstream, whose sync word is 32 bytes in.
    let (fl_miso_o, fl_miso) = signal::<Bit, DefaultClock>();
    let (fl_cs_n_o, fl_cs_n) = signal::<Bit, DefaultClock>();
    let (fl_mosi_o, fl_mosi) = signal::<Bit, DefaultClock>();
    let (fl_cclk_o, fl_cclk) = signal::<Bit, DefaultClock>();
    let mut head = vec![0xffu8; 16];
    head.extend([0x00, 0x00, 0x00, 0xbb, 0x11, 0x22, 0x00, 0x44]);
    head.extend([0xff; 8]);
    head.extend([0xaa, 0x99, 0x55, 0x66]);
    head.resize(256, 0xff);
    let mut chip = FlashDevice::new(head, [0x20, 0xba, 0x18], false, false);
    // The Ethernet PHY's management interface, with the model PHY the
    // caller gives in `net`: `board_phy()` unless a test wants another.
    let (phy_mdio_in_o, phy_mdio_in) = signal::<Bit, DefaultClock>();
    let (phy_mdc_o, phy_mdc) = signal::<Bit, DefaultClock>();
    let (phy_out_o, phy_out) = signal::<Bit, DefaultClock>();
    let (phy_oe_o, phy_oe) = signal::<Bit, DefaultClock>();
    let mut phy = phy.unwrap_or_else(board_phy);
    phy_mdio_in_o.set(Bit::One);
    // The card in the SD slot, on the host's lines; a line nobody
    // drives reads high through its pull-up.
    let (sd_cmd_in_o, sd_cmd_in) = signal::<Bit, DefaultClock>();
    let (sd_dat_in_o, sd_dat_in) = signal::<U<4>, DefaultClock>();
    let (sd_clk_o, sd_clk) = signal::<Bit, DefaultClock>();
    let (sd_cmd_out_o, sd_cmd_out) = signal::<Bit, DefaultClock>();
    let (sd_cmd_oe_o, sd_cmd_oe) = signal::<Bit, DefaultClock>();
    let (sd_dat_out_o, sd_dat_out) = signal::<U<4>, DefaultClock>();
    let (sd_dat_oe_o, sd_dat_oe) = signal::<Bit, DefaultClock>();
    let mut card = card.unwrap_or_default();
    sd_cmd_in_o.set(Bit::One);
    sd_dat_in_o.set(U::<4>::from(0xfu8));
    let board = board.run(
        BoardIn {
            rst,
            irq,
            rx,
            sys_clk: quiet(),
            sys_rst: quiet(),
            vb: vb_rx,
            vr: vr_rx,
            net_rx: net_in_rx,
            fl_miso,
            phy_mdio_in,
            sd_cmd_in,
            sd_dat_in,
            bscan_sel: signal::<Bit, Tck>().1,
            bscan_shift: signal::<Bit, Tck>().1,
            bscan_capture: signal::<Bit, Tck>().1,
            bscan_update: signal::<Bit, Tck>().1,
            bscan_tdi: signal::<Bit, Tck>().1,
            bscan_reset: signal::<Bit, Tck>().1,
            scan_req,
            jtag: AxiHostPins {
                awid,
                awaddr,
                awlen,
                awsize,
                awburst,
                awlock: lo(),
                awcache: signal::<U<4>, DefaultClock>().1,
                awprot: signal::<U<3>, DefaultClock>().1,
                awvalid,
                wdata,
                wstrb,
                wlast,
                wvalid,
                bready,
                arid,
                araddr,
                arlen,
                arsize,
                arburst,
                arlock: lo(),
                arcache: signal::<U<4>, DefaultClock>().1,
                arprot: signal::<U<3>, DefaultClock>().1,
                arvalid,
                rready,
            },
        },
        BoardOut {
            halt: halt_o,
            tx: tx_o,
            pwm_pins: signal::<U<4>, DefaultClock>().0,
            calib: bit(),
            ui_clk: bit(),
            ui_rst: bit(),
            ck_p: bit(),
            ck_n: bit(),
            mem_rst_n: bit(),
            cke: bit(),
            cs_n: bit(),
            ras_n: bit(),
            cas_n: bit(),
            we_n: bit(),
            row: signal::<U<15>, DefaultClock>().0,
            bank: signal::<U<3>, DefaultClock>().0,
            dm: signal::<U<4>, DefaultClock>().0,
            odt: bit(),
            dq: pad::<U<32>, DefaultClock>(),
            dqs: pad::<U<4>, DefaultClock>(),
            dqs_n: pad::<U<4>, DefaultClock>(),
            vaw: vaw_tx,
            var: var_tx,
            vw: vw_tx,
            net_tx: net_out_tx,
            jtag_awready: awready_o,
            jtag_wready: wready_o,
            jtag_bid: bid_o,
            jtag_bresp: bresp_o,
            jtag_bvalid: bvalid_o,
            jtag_arready: arready_o,
            jtag_rid: rid_o,
            jtag_rdata: rdata_o,
            jtag_rresp: rresp_o,
            jtag_rlast: rlast_o,
            jtag_rvalid: rvalid_o,
            fl_cs_n: fl_cs_n_o,
            fl_mosi: fl_mosi_o,
            fl_cclk: fl_cclk_o,
            fl_refused: bit(),
            phy_mdc: phy_mdc_o,
            phy_mdio_out: phy_out_o,
            phy_mdio_oe: phy_oe_o,
            sd_clk: sd_clk_o,
            sd_cmd_out: sd_cmd_out_o,
            sd_cmd_oe: sd_cmd_oe_o,
            sd_dat_out: sd_dat_out_o,
            sd_dat_oe: sd_dat_oe_o,
            bscan_tdo: signal::<Bit, Tck>().0,
            // A scanout runs only when the run asks for one.
            scan_words,
        },
    );
    let mut sim = Running::new(join2(
        board,
        join2(
            raster.run((), (col_o, vis_o, line_o, row_o, frame_o)),
            pair.run(
                (pair_inp, col, vis, line, row, frame, base, clear, show),
                (pix_o, pair_req, starved_o, stuck_o, stuck_at_o),
            ),
        ),
    ));
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    irq_o.set(Bit::Zero);
    rx_o.set(Bit::One);
    let mut term = if blocks.is_empty() {
        Terminal::new(reply)
    } else {
        Terminal::paced(reply, blocks)
    };
    let mut halted_at = None;
    let mut typed_was = 0;
    let mut typed_at = 0;
    let mut ran_for = 0;
    // The program at the other end of the Ethernet port, and what it
    // has been told to remember.
    let mut frame: Vec<u8> = Vec::new();
    let mut reply: Vec<EthByte> = Vec::new();
    // Frames put on the wire from the start, before anything is asked
    // of the program at the other end. They leave a byte a cycle, as
    // the answers do.
    for f in inject {
        for (i, b) in f.iter().enumerate() {
            reply.push(EthByte {
                data: U::from(*b),
                last: Bit::from_bool(i + 1 == f.len()),
            });
        }
    }
    let mut words: HashMap<u32, u32> = HashMap::new();
    let mut sent: Vec<Vec<u8>> = Vec::new();
    // The crossing's FIFOs, for a run with a reset in it.
    let mut req_cdc: VecDeque<U<32>> = VecDeque::new();
    let mut words_cdc: VecDeque<U<32>> = VecDeque::new();
    let mut master = Master::default();
    // When the reset began: the first cycle from the one asked for at
    // which a scanout's words are in the crossing, so that it lands
    // with a line on its way, as it can on the board.
    let mut reset_from: Option<u64> = None;
    for cycle in 0..limit {
        if let Some((from, cycles)) = reset_at {
            let words = scan.is_none() || words_cdc.len() >= 64;
            if reset_from.is_none() && cycle >= from && words {
                reset_from = Some(cycle);
                set_reset(true);
                rst_o.set(Bit::One);
            }
            if reset_from.map(|f| f + cycles) == Some(cycle) {
                set_reset(false);
                rst_o.set(Bit::Zero);
            }
        }
        master.drive(plan, &jtag);
        device(
            &net_out_rx,
            &net_in_tx,
            &mut frame,
            &mut reply,
            &mut words,
            &mut sent,
            serve,
        );
        sim.cycle();
        master.observe(plan, &jtag);
        chip.step(
            fl_cs_n.get().to_bool(),
            fl_cclk.get().to_bool(),
            fl_mosi.get().to_bool(),
        );
        fl_miso_o.set(Bit::from_bool(chip.miso()));
        // The management line reads what the board drives, else what
        // the PHY drives, else one through the pull-up.
        let (oe, out) = (phy_oe.get().to_bool(), phy_out.get().to_bool());
        let line =
            |p: &MdioPhy| if oe { out } else { p.drives().unwrap_or(true) };
        let seen = line(&phy);
        phy.step(phy_mdc.get().to_bool(), seen);
        phy_mdio_in_o.set(Bit::from_bool(line(&phy)));
        // The SD lines: whoever drives each, else high.
        let (host_cmd, host_dat) =
            (sd_cmd_oe.get().to_bool(), sd_dat_oe.get().to_bool());
        let (cmd, dat) =
            (sd_cmd_out.get().to_bool(), sd_dat_out.get().raw() as u8);
        card.step(sd_clk.get().to_bool(), host_cmd, cmd, host_dat, dat);
        sd_cmd_in_o.set(Bit::from_bool(if host_cmd {
            cmd
        } else {
            card.cmd_out()
        }));
        sd_dat_in_o.set(U::<4>::from(if host_dat {
            dat
        } else {
            card.dat_out()
        }));
        term.see(tx.get().to_bool());
        rx_o.set(Bit::from_bool(term.level()));
        ran_for = cycle;
        if term.typed() != typed_was {
            typed_was = term.typed();
            typed_at = cycle;
        }
        if video {
            // The video slot as the flagship has it, as far as a
            // program needs: a write kept, a read of it given back, and
            // the video peripheral's status, at offset zero, blanking
            // 400 cycles in every 4000.
            if vaw_rx.peek().is_some()
                && vw_rx.peek().is_some()
                && vb_tx.ready().to_bool()
            {
                let aw = vaw_rx.recv_if(true).unwrap();
                let w = vw_rx.recv_if(true).unwrap();
                vwords.insert(aw.addr.raw() as u32 & 0xff, w.data.raw() as u32);
                vb_tx.send(LiteB { resp: Resp::Okay });
            }
            if var_rx.peek().is_some() && vr_tx.ready().to_bool() {
                let ar = var_rx.recv_if(true).unwrap();
                let at = ar.addr.raw() as u32 & 0xff;
                let data = if at == 0 {
                    (cycle % 4000 < 400) as u32
                } else {
                    vwords.get(&at).copied().unwrap_or(0)
                };
                vr_tx.send(LiteR {
                    data: U::from(data),
                    resp: Resp::Okay,
                });
            }
        }
        if let Some(s) = &scan {
            if starved.get().to_bool() && scan_log.starved.is_none() {
                scan_log.starved = Some(cycle);
            }
            if stuck.get().to_bool() && scan_log.stuck.is_none() {
                scan_log.stuck = Some((cycle, stuck_at.get().raw() as u32));
            }
            // The base and the bit that shows it are the video
            // peripheral's registers, which a reset clears and a program
            // writes again, a frame after the reset here (issue 1317).
            let shown_again = reset_from
                .zip(reset_at)
                .map(|(f, (_, n))| f + n + 6 * SCAN_LINE);
            if cycle == s.show_at || Some(cycle) == shown_again {
                base_o.set(U::<32>::from(s.base));
                show_o.set(Bit::One);
            }
            if reset_from == Some(cycle) {
                base_o.set(U::<32>::from(0u32));
                show_o.set(Bit::Zero);
            }
            // The pair's requests to the board, and the board's words
            // to the pair, a cycle each way through the taps. A run with
            // a reset in it puts the flagship's crossing between them as
            // well: `chan_cdc`'s FIFOs, of 4 requests and 1024 words,
            // which nothing resets (issue 1317).
            if reset_at.is_some() {
                if req_cdc.len() < 4 {
                    if let Some(at) = pair_req_rx.recv_if(true) {
                        req_cdc.push_back(at);
                    }
                }
                if words_cdc.len() < 1024 {
                    if let Some(w) = scan_words_rx.recv_if(true) {
                        words_cdc.push_back(w);
                    }
                }
            }
            let req_ready = scan_req_tx.ready().to_bool();
            if reset_at.is_some() {
                if req_ready {
                    if let Some(at) = req_cdc.pop_front() {
                        scan_req_tx.send(at);
                        scan_log.lines.push((at.raw() as u32, cycle, None));
                    }
                }
            } else if pair_req_rx.peek().is_some() && req_ready {
                let at = pair_req_rx.recv_if(true).unwrap();
                scan_req_tx.send(at);
                scan_log.lines.push((at.raw() as u32, cycle, None));
            }
            // The pixel side takes a word a pixel, one in four of the
            // board's cycles, so the words' FIFO fills under a burst.
            let word = if reset_at.is_some() {
                if cycle % 4 == 0 && pair_inp_tx.ready().to_bool() {
                    words_cdc.pop_front()
                } else {
                    None
                }
            } else if scan_words_rx.peek().is_some()
                && pair_inp_tx.ready().to_bool()
            {
                scan_words_rx.recv_if(true)
            } else {
                None
            };
            if let Some(w) = word {
                pair_inp_tx.send(w);
                scan_words_in += 1;
                if scan_words_in.is_multiple_of(640) {
                    if let Some(l) = scan_log.lines.get_mut(scan_got) {
                        l.2 = Some(cycle);
                    }
                    scan_got += 1;
                }
            }
        }
        if until_planned {
            if master.at >= plan.len() {
                break;
            }
        } else if scan.is_none() && halt.get().to_bool() {
            halted_at = Some(cycle);
            break;
        }
    }
    set_reset(false);
    rst_o.set(Bit::Zero);
    // The port is still sending what its queue holds when the core
    // halts (issue 1011): up to eight bytes and the one going out, each
    // a frame of ten bits of four cycles, and a cycle between frames;
    // ten frames and a margin cover it.
    for _ in 0..(10 * 10 * 4 + 64) {
        sim.cycle();
        term.see(tx.get().to_bool());
    }
    Ran {
        said: term.said.clone(),
        typed: term.typed(),
        typed_at,
        ran_for,
        halted_at,
        sent,
        got: master.got,
        steps: master.at,
        bursts: master.bursts,
        reset_from,
        flash: chip.commands.clone(),
        flash_short: chip.partial,
        phy_frames: phy.frames.clone(),
        phy_page: phy.regs[31],
        card,
        scan: scan_log,
    }
}

/// The scanout after a reset (issue 1178). The flagship's scanout
/// comes out of every reset with no base, and a program gives it one
/// some time later, as `scanprobe` and `scanat` do. Once it has one, its
/// lines must come back from that base, whatever it did before.
#[test]
fn the_scanout_shows_a_base_given_after_the_reset() {
    let frame = 6 * SCAN_LINE;
    let net = Net {
        scan: Some(Scan {
            base: 0x4100_1000,
            show_at: 3 * frame,
        }),
        ..Net::default()
    };
    let ran = run_all(
        hello_program::TEXT,
        hello_program::DATA,
        b"",
        &[],
        6 * frame,
        net,
        &[],
    );
    let lines = &ran.scan.lines;
    // Nothing is asked for before the base: not the boot memory at
    // zero, whose one-beat answer to a line's burst hung the fetch.
    assert!(
        lines.iter().all(|(at, _, _)| *at >= 0x4100_1000),
        "asked for {:x?}",
        lines.iter().take(8).collect::<Vec<_>>()
    );
    let shown = lines
        .iter()
        .filter(|(at, _, got)| *at >= 0x4100_1000 && got.is_some())
        .count();
    assert!(
        shown >= 3,
        "{shown} lines of the base came back; asked for {:x?}",
        lines.iter().take(8).collect::<Vec<_>>()
    );
    // Lines that come, if late, are never stuck (issue 1197).
    assert_eq!(ran.scan.stuck, None, "a scanout that keeps going");
}

/// The base and the show given together after a reset, at every point
/// of a frame (issue 1317): the first line asked for is the base's, and
/// nothing below it is asked for. On the board, a show written after a
/// frame's vertical sync but before its last row began asked the first
/// line from the base the pair had taken at the sync, which a reset
/// leaves at zero, and the fetch stuck at row 3 of zero, `0x3000`.
#[test]
fn the_first_line_is_the_bases_wherever_in_the_frame_it_is_shown() {
    let frame = 6 * SCAN_LINE;
    // Every other half line of a frame: the board's failure was at
    // eight, between the sync and the last row.
    for k in (0..12).step_by(2) {
        let show_at = 3 * frame + k * SCAN_LINE / 2;
        let net = Net {
            scan: Some(Scan {
                base: 0x4100_1000,
                show_at,
            }),
            ..Net::default()
        };
        let ran = run_all(
            hello_program::TEXT,
            hello_program::DATA,
            b"",
            &[],
            show_at + 2 * frame,
            net,
            &[],
        );
        let lines = &ran.scan.lines;
        assert!(
            lines.iter().all(|(at, _, _)| *at >= 0x4100_1000),
            "shown at {show_at} ({k} half lines into a frame): asked for {:x?}",
            lines.iter().take(8).collect::<Vec<_>>()
        );
        assert_eq!(ran.scan.stuck, None, "shown at {show_at}");
    }
}

/// The configuration flash on the board (issue 312): the core reads the
/// chip's identity over the master, sends write enable, which the pins
/// refuse, so the chip's latch reads clear, and finds the bitstream's
/// sync word through the window. The chip is the model; the same
/// program on the board reads the real one.
#[test]
fn the_configuration_flash_answers_on_the_board() {
    let ran = run(flashid_program::TEXT, flashid_program::DATA, b"", 40000);
    assert_eq!(
        ran.said,
        "flash id 0020ba18\nflash wel 0\nflash sync at 32\n"
    );
    // The chip took the identity, the status read and nine window
    // reads, up to the word the sync is in; write enable was let go of
    // on its seventh bit, and never reached it.
    let mut want = vec![0x9f, 0x05];
    want.extend([0x0b; 9]);
    assert_eq!(ran.flash, want);
    assert_eq!(ran.flash_short, 1, "write enable, cut short");
    assert!(ran.halted_at.is_some(), "and halted");
}

/// A load from a hole in the board's map, refused by the router, as
/// the HAL's fault report says it (#1214): a load access fault, cause
/// 5, at an instruction in the boot memory, with the address in
/// `mtval` and in `a0`, where the program put it.
#[test]
fn a_refused_load_says_where_it_was_and_what_it_read() {
    let ran = run(faultsay_program::TEXT, faultsay_program::DATA, b"", 20000);
    let line = ran
        .said
        .strip_prefix("fault say\n")
        .unwrap_or_else(|| panic!("the greeting first: {:?}", ran.said));
    let words: Vec<&str> = line.trim_end().split(' ').collect();
    assert_eq!(words.len(), 10, "one line of five fields: {line:?}");
    let field = |name: &str| {
        let at = words.iter().position(|w| *w == name).unwrap();
        u32::from_str_radix(words[at + 1], 16).unwrap()
    };
    assert_eq!(field("trap"), 5, "a load access fault");
    assert!(field("at") < 0x1000, "at an instruction in the boot memory");
    assert_eq!(field("mtval"), 0x3000_0000, "the address refused");
    assert_eq!(field("a0"), 0x3000_0000, "a0 as the load had it");
    assert!(ran.halted_at.is_some(), "and halted, not resumed");
    assert!(!ran.said.contains("not refused"));
}

/// The registers of the model PHY on the board's management line: the
/// JL2121's, page 0, as `phyregs` read them from the board on October
/// 3, 2026, the second of two runs (#864). The first differed only in
/// two bits that clause 22 clears on a read: link status in register 1
/// and page received in register 6.
fn phy_regs() -> [u16; 32] {
    [
        0x1140, 0x796d, 0x937c, 0x4032, 0x01e1, 0xcde1, 0x000d, 0x2001, 0x0000,
        0x0200, 0x3800, 0x0000, 0x0000, 0x0000, 0x0000, 0x2000, 0x0040, 0x0000,
        0x0000, 0x0000, 0x4080, 0x7c12, 0x489b, 0x2800, 0x8000, 0x0000, 0x0000,
        0x002f, 0x0000, 0x1208, 0x8000, 0x0000,
    ]
}

/// The model PHY on the board's management line: page 0 is what the
/// JL2121 answered with on the board (`phy_regs`), and it pages its
/// vendor registers by register 31 as JLSemi's do. Register 17 of page
/// 3336, where JLSemi's driver keeps the RGMII delay bits, holds
/// `0200`: the receive delay set and the transmit delay clear. That word
/// is the model's own, chosen so the two bits read differently; the
/// JL2121's is what the board run of `phydelay` will say (#869).
fn board_phy() -> MdioPhy {
    let mut phy = MdioPhy::new(0, phy_regs());
    phy.pages.insert((3336, 17), 0x0200);
    phy
}

/// A run with `phy` on the management line in place of `board_phy()`.
fn run_phy(text: &[u32], data: &[u8], limit: u64, phy: MdioPhy) -> Ran {
    let net = Net {
        phy: Some(phy),
        ..Net::default()
    };
    run_all(text, data, b"", &[], limit, net, &[])
}

/// The Ethernet PHY's management registers on the board (issue 864):
/// the core finds the PHY at the first address whose register 2 is
/// not all ones and prints its 32 registers, four to a line. The PHY
/// is the model, holding what the JL2121 answered on the board, so the
/// run prints what the board printed. The program only reads: every
/// frame the PHY took is a read.
#[test]
fn the_phy_registers_read_on_the_board() {
    let ran = run(phyregs_program::TEXT, phyregs_program::DATA, b"", 200_000);
    let regs = phy_regs();
    let mut want = String::from("phy at 0\n");
    for row in 0..8 {
        want += &format!("r{:02}", row * 4);
        for w in &regs[row * 4..row * 4 + 4] {
            want += &format!(" {w:04x}");
        }
        want.push('\n');
    }
    assert_eq!(ran.said, want);
    // Address 0's identifier, which answers, then the 32 registers in
    // order; no write.
    let mut frames = vec![(0, 2, None)];
    frames.extend((0..32).map(|r| (0, r, None)));
    assert_eq!(ran.phy_frames, frames);
    assert!(ran.halted_at.is_some(), "and halted");
}

/// The RGMII delay bits on the board (issue 869): the core finds the
/// PHY, reads the page select, selects page 3336, reads register 17,
/// writes the page it found back, and reads the page select again. The
/// frames are exactly those, read, write, read, write, read, after the
/// scan, and the page select is left as it was found.
#[test]
fn the_phy_delay_bits_read_through_the_page() {
    let ran = run_phy(
        phydelay_program::TEXT,
        phydelay_program::DATA,
        60_000,
        board_phy(),
    );
    assert_eq!(
        ran.said,
        "phy at 0\npage was 0000\np3336 r17 0200\ntx delay 0\nrx delay 1\n\
         page now 0000\n"
    );
    assert_eq!(
        ran.phy_frames,
        vec![
            (0, 2, None),
            (0, 31, None),
            (0, 31, Some(3336)),
            (0, 17, None),
            (0, 31, Some(0)),
            (0, 31, None),
        ]
    );
    assert_eq!(ran.phy_page, 0, "the page select restored");
    assert!(ran.halted_at.is_some(), "and halted");
}

/// A page select that reads all ones, which is a line nobody drove: the
/// program says so and writes nothing.
#[test]
fn a_failed_page_read_writes_nothing() {
    let mut phy = board_phy();
    phy.regs[31] = 0xffff;
    let ran =
        run_phy(phydelay_program::TEXT, phydelay_program::DATA, 60_000, phy);
    assert_eq!(ran.said, "phy at 0\npage read failed, nothing written\n");
    assert_eq!(ran.phy_frames, vec![(0, 2, None), (0, 31, None)]);
    assert!(ran.phy_frames.iter().all(|f| f.2.is_none()), "no write");
    assert!(ran.halted_at.is_some(), "and halted");
}

/// The card in the SD slot on the board (issue 153): the core brings
/// it up at 400 kHz, asking with HCS set, prints its identity and
/// address, and reads block 0 on one line and again on four. The card
/// is the model, high capacity, its block 0 ending in the boot
/// signature, and after the run it holds what it held before: the
/// program writes nothing.
#[test]
fn the_sd_card_is_read_on_the_board() {
    let mut card = SdCard::default();
    card.blocks[510] = 0x55;
    card.blocks[511] = 0xaa;
    let before = card.blocks.clone();
    let word = |c: &[u8]| u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
    let cid: Vec<String> = card
        .cid
        .chunks(4)
        .map(|c| format!("{:08x}", word(c)))
        .collect();
    let mut want = format!(
        "cmd8 000001aa\nocr c0ff8000 sdhc\ncid {}\nrca {:08x}\nblk0\n",
        cid.join(" "),
        card.rca
    );
    for line in before[..512].chunks(16) {
        for b in line {
            want += &format!("{b:02x}");
        }
        want.push('\n');
    }
    want += "wide same\nboot 55aa\ndma same\n";
    let net = Net {
        card: Some(card),
        ..Net::default()
    };
    let ran = run_all(
        sdprobe_program::TEXT,
        sdprobe_program::DATA,
        b"",
        &[],
        1_000_000,
        net,
        &[],
    );
    assert_eq!(ran.said, want);
    assert_eq!(ran.card.blocks, before, "the card holds what it held");
    assert_eq!(ran.card.bad_commands, 0, "every command's CRC was right");
    assert!(ran.halted_at.is_some(), "and halted");
}

#[test]
fn the_greeting_runs_on_the_board() {
    let ran = run(hello_program::TEXT, hello_program::DATA, b"", 8000);
    assert_eq!(ran.said, "hello from rust\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

#[test]
fn the_memory_test_runs_on_the_board() {
    let ran = run(ddr3_program::TEXT, ddr3_program::DATA, b"", 40000);
    assert_eq!(ran.said, "ddr3 ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// A frame arrives on the wire, and the Ethernet port's engines store
/// it in DDR3 without the core touching a byte of it. The core then
/// reads it back out of memory as the Zephyr driver will, and says what
/// it read (issue 151).
///
/// The bytes the core says are the only evidence this test takes, and
/// they come out of the real memory through the real bus, so a pass
/// covers the whole receive path: the split by EtherType, the length
/// found again after the crossing, the packing into words, the store
/// engine's bursts through the arbiter, the router and the bridge, and
/// the register block's arrival. That is what the engines' own
/// examples could not show, since each ran against a memory it defined
/// itself.
#[test]
fn a_frame_received_lands_in_memory_and_the_core_reads_it_back() {
    // Not the remote peripheral's type, so the sharing unit sends it to
    // the Ethernet port rather than to the remote peripheral. Twenty one
    // bytes, so the last word holds one real byte and the store engine
    // strobes away the other three, which is a write the memory must
    // honour lane by lane.
    let mut frame: Vec<u8> = (0..21u8).map(|i| 0x40 + i).collect();
    frame[12] = 0x08;
    frame[13] = 0x00;
    let bytes: String = frame.iter().map(|b| format!("{b:02x}")).collect();
    let want = format!("rx {:04x} {bytes}\n", frame.len());
    let frames = [frame];
    let net = Net {
        inject: &frames,
        ..Default::default()
    };
    let ran = run_all(
        ethrx_program::TEXT,
        ethrx_program::DATA,
        b"",
        &[],
        40000,
        net,
        &[],
    );
    assert_eq!(ran.said, want, "what the core read back out of DDR3");
    assert!(ran.halted_at.is_some(), "the core acknowledged and halted");
}

/// A driver too slow for the frames, as Zephyr's is on the board: it
/// copies each frame out of DDR3 inside its interrupt handler, a
/// thousand cycles and more, while full frames arrive every 1514 bytes'
/// time (issue 1313). Eight such frames back to back, each with its
/// number at byte 14 and again at byte 1500, against a driver that
/// polls, reads the slot and the number, spends five thousand loop
/// turns on it, reads the second copy, and acknowledges.
///
/// Every frame must be either announced to the driver, once and intact,
/// or dropped and counted in `rx_errors`; none may land on a slot the
/// driver has not released, and none may vanish unannounced. Before the
/// fix the driver was told of two frames, 0 and 5, and `rx_errors` read
/// zero: the other six were overwritten or never announced.
#[test]
fn frames_beyond_the_slots_are_dropped_and_counted_not_lost() {
    let frames: Vec<Vec<u8>> = (0..8u8)
        .map(|seq| {
            let mut f: Vec<u8> = (0..1514u32).map(|i| i as u8).collect();
            f[12] = 0x08;
            f[13] = 0x00;
            f[14] = seq;
            f[1500] = seq;
            f
        })
        .collect();
    let net = Net {
        inject: &frames,
        ..Default::default()
    };
    let ran = run_all(&slow_driver(5000), &[], b"", &[], 3_000_000, net, &[]);
    assert!(ran.halted_at.is_some(), "the driver gave up and halted");
    let word = |name: &str| -> u32 {
        let at = ran.said.find(name).expect(name) + name.len() + 1;
        u32::from_str_radix(&ran.said[at..at + 8], 16).expect("hex")
    };
    let (told, errors, torn, seen) =
        (word("told"), word("errors"), word("torn"), word("seen"));
    assert_eq!(
        torn, 0,
        "no frame was overwritten under the driver: {}",
        ran.said
    );
    assert_eq!(
        seen.count_ones(),
        told,
        "no frame was told twice: {}",
        ran.said
    );
    assert_eq!(
        told + errors,
        8,
        "every frame told or counted: {}",
        ran.said
    );
    assert!(told >= 2, "both slots were used: {}", ran.said);
}

/// The driver of the test above: `delay` loop turns a frame between
/// reading its number and acknowledging it. Says how many frames it was
/// told of, `rx_errors`, how many frames' two numbers disagreed, and the
/// mask of the numbers it saw.
fn slow_driver(delay: u32) -> Vec<u32> {
    use vreteno32::isa::{
        add, addi, andi, beq, bne, halt, jal, lbu, lui, lw, or, sll, slli, sw,
        UART_BASE,
    };
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    li(&mut a, 10, 0x3400); // the port's registers
    li(&mut a, 9, 0x4100_0000); // the receive slots
    a.emit(addi(11, 0, 0)); // frames told of
    a.emit(addi(15, 0, 0)); // the numbers seen, a bit each
    a.emit(addi(8, 0, 0)); // frames torn
    a.emit(addi(12, 0, 0)); // polls since the last frame
    let top = a.label();
    let idle = a.label();
    a.place(top);
    a.emit(lw(14, 10, 0x10)); // rx_ev_pending
    a.emit(andi(14, 14, 1));
    a.to(idle, |off| beq(14, 0, off));
    a.emit(addi(12, 0, 0));
    a.emit(addi(11, 11, 1));
    a.emit(lw(7, 10, 0)); // rx_slot
    a.emit(slli(7, 7, 11));
    a.emit(add(7, 7, 9)); // x7 = the slot's address
    a.emit(lbu(6, 7, 14)); // x6 = the frame's number
    a.emit(addi(13, 0, 1));
    a.emit(sll(13, 13, 6));
    a.emit(or(15, 15, 13));
    li(&mut a, 13, delay);
    let spin = a.label();
    a.place(spin);
    a.emit(addi(13, 13, -1));
    a.to(spin, |off| bne(13, 0, off));
    a.emit(lbu(5, 7, 1500)); // its second copy, after the copy's time
    let whole = a.label();
    a.to(whole, |off| beq(5, 6, off));
    a.emit(addi(8, 8, 1));
    a.place(whole);
    a.emit(addi(14, 0, 1));
    a.emit(sw(14, 10, 0x10)); // acknowledge
    a.to(top, |off| jal(0, off));
    a.place(idle);
    a.emit(addi(12, 12, 1));
    li(&mut a, 14, 30000);
    a.to(top, |off| bne(12, 14, off));
    a.emit(lw(5, 10, 0x08)); // rx_errors
    say(&mut a, b"told ");
    say_hex(&mut a, 11);
    say(&mut a, b" errors ");
    say_hex(&mut a, 5);
    say(&mut a, b" torn ");
    say_hex(&mut a, 8);
    say(&mut a, b" seen ");
    say_hex(&mut a, 15);
    say(&mut a, b"\n");
    a.emit(halt());
    a.words()
}

/// The core writes two frames into the two transmit slots in DDR3 and
/// sends them back to back, in the Zephyr driver's shape: wait until the
/// port is ready, give it the slot, the length and the start, return.
/// The port's engines fetch each out of memory and put it on the wire
/// without the core (issue 151).
///
/// What left the port is the only evidence taken, compared byte for
/// byte and in order with what the core wrote, so a pass covers the
/// whole sending path: the fetch engine's bursts through the arbiter,
/// the router and the bridge, the word count `FrameOut` works out, its
/// bytes with each frame's last one marked, and the merge onto the wire.
///
/// TWO frames, for two reasons. A byte side that read past a frame's
/// count leaves the extra bytes in front of the next frame, which one
/// frame cannot show. And the second send's wait for ready comes as
/// soon after the first start as it can, with nothing between, which is
/// where a stale ready from a posted start would bite: the driver's own
/// shape copies the next frame between sends and so has more slack than
/// this, so this passing is the tighter case passing.
#[test]
fn two_frames_written_to_memory_leave_the_port_as_written_in_order() {
    let frame = |f: u32, len: u32| -> Vec<u8> {
        (0..len)
            .map(|i| match i {
                12 => 0x08,
                13 => {
                    if f == 0 {
                        0x00
                    } else {
                        0x06
                    }
                }
                _ => (if f == 0 { 0x60 } else { 0x90 } + i) as u8,
            })
            .collect()
    };
    let a = frame(0, 23);
    let b = frame(1, 18);
    let ran = run(ethtx_program::TEXT, ethtx_program::DATA, b"", 40000);
    assert!(
        ran.halted_at.is_some(),
        "both frames went and the core halted"
    );
    assert_eq!(ran.sent.len(), 2, "exactly two frames left the port");
    assert_eq!(ran.sent[0], a, "the first, as the core wrote it");
    assert_eq!(ran.sent[1], b, "then the second, as the core wrote it");
}

/// The entropy source on its slot: the program turns it on, takes four
/// words, and says they came and differed. The rings are the model in
/// this run, so this is the slot, the bridge and the peripheral proven
/// and nothing about randomness (issue 458).
///
/// Then the run of raw samples the program joins from overlapping
/// windows (issue 805): the model's samples are known, so the run must
/// be found in them whole, which says the joining neither repeats a
/// sample nor skips one.
#[test]
fn the_entropy_source_answers_on_the_board() {
    const CYCLES: u64 = 120000;
    let ran = run(trng_program::TEXT, trng_program::DATA, b"", CYCLES);
    assert!(ran.halted_at.is_some(), "and halted: {}", ran.said);
    let mut lines = ran.said.lines();
    assert_eq!(lines.next(), Some("trng ok"), "{}", ran.said);
    // In simulation the loop is steady, so every pair fits at its
    // cycle count, which is exact since issue 807.
    assert_eq!(
        lines.next(),
        Some("rawrun pairs 63 fit 63 moved 0 gap 0 bad 0"),
        "{}",
        ran.said
    );
    assert_eq!(lines.next(), Some("rawrun"), "{}", ran.said);
    let words: Vec<u32> = lines
        .map(|l| u32::from_str_radix(l, 16).expect(l))
        .collect();
    // 64 windows, the first whole and each after it adding samples.
    assert!(words.len() > 2, "{}", ran.said);
    let run: Vec<u8> = words
        .iter()
        .flat_map(|w| (0..32).rev().map(move |i| (w >> i & 1) as u8))
        .collect();
    let stream = model_samples(CYCLES as usize);
    assert!(
        stream.windows(run.len()).any(|w| w == run.as_slice()),
        "the run of {} samples is not a stretch of the model's samples",
        run.len()
    );
}

/// The samples the model rings give, folded as the peripheral folds
/// them, from the seeds on: each ring a shift register stepped once a
/// cycle, its top bit the sample, and the eight XORed. Every ring
/// starts at its seed and steps, as the model now does; it used to be
/// ring 7 alone, when the model kept its rings in a memory that takes
/// one write a step (issues 808 and 809).
fn model_samples(n: usize) -> Vec<u8> {
    use txhdl_parts::trng::SEEDS;
    let mut lfsr = SEEDS;
    (0..n)
        .map(|_| {
            let mut bit = 0u8;
            for s in lfsr.iter_mut() {
                bit ^= (*s >> 31) as u8;
                let fb = (*s >> 31) ^ (*s >> 21) ^ (*s >> 1) ^ *s;
                *s = (*s << 1) | (fb & 1);
            }
            bit
        })
        .collect()
}

/// The terminal types four bytes, and the program takes each through
/// the interrupt controller, one interrupt a byte, never polling the
/// serial port for input.
#[test]
fn input_comes_by_interrupt_one_byte_each() {
    let ran = run(irq_program::TEXT, irq_program::DATA, b"ping", 20000);
    assert_eq!(ran.said, "ready\ngot ping in 4 interrupts\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// A line pasted at the shell (issue 1153): the terminal types
/// forty-eight bytes back to back, the line's full speed, while the
/// program is busy elsewhere, and only then does it read them. Every
/// byte must still be there. With a receive queue of eight, SiFive's
/// depth, all but eight were dropped, which is what typing at Linux's
/// shell on the board did.
#[test]
fn a_line_typed_back_to_back_waits_whole_in_the_receive_queue() {
    use vreteno32::isa::{addi, blt, bne, halt, lui, lw, sw, UART_BASE};
    let line: &[u8] = b"uname -a; cat /proc/cpuinfo; free; ls / # 48 by\n";
    assert_eq!(line.len(), 48);
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12));
    say(&mut a, b"go\n");
    // Busy for longer than the line takes to arrive: forty cycles a
    // byte at this divider, about two thousand for the line.
    a.emit(lui(5, 2)); // x5 = 8192 rounds
    let spin = a.label();
    a.place(spin);
    a.emit(addi(5, 5, -1));
    a.to(spin, |off| bne(5, 0, off));
    // Then every byte waiting, echoed, until `rxdata` says empty.
    let next = a.label();
    let done = a.label();
    a.place(next);
    a.emit(lw(4, 1, 4)); // x4 = rxdata, bit 31 empty
    a.to(done, |off| blt(4, 0, off));
    let wait = a.label();
    a.place(wait);
    a.emit(lw(2, 1, 0));
    a.to(wait, |off| blt(2, 0, off));
    a.emit(sw(4, 1, 0));
    a.to(next, |off| bne(0, 1, off));
    a.place(done);
    a.emit(halt());
    let ran = run(&a.words(), &[], line, 120_000);
    assert!(ran.halted_at.is_some(), "the program halted: {}", ran.said);
    assert_eq!(ran.typed, line.len(), "the terminal typed the line");
    assert_eq!(
        ran.said,
        format!("go\n{}", String::from_utf8_lossy(line)),
        "every byte of the line came back"
    );
}

/// A byte at a time onto the serial port, waiting while it is busy.
/// `x1` holds the page the port is on.
fn say(a: &mut vreteno32::program::Asm, text: &[u8]) {
    use vreteno32::isa::{addi, blt, lw, sw};
    for byte in text {
        // Wait while the port's queue is full, which a load of
        // `txdata` says in its sign bit (issue 1011).
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 0)); // x2 = txdata, bit 31 full
        a.to(wait, |off| blt(2, 0, off));
        a.emit(addi(3, 0, *byte as i32));
        a.emit(sw(3, 1, 0)); // the byte goes out
    }
}

/// A program that uses the remote peripheral: it writes a word to
/// `0x3300`, reads it back, and says on the serial port whether what
/// came back is what went out. Everything between the store and the
/// load is a frame leaving the Ethernet port, a program reading it,
/// and a frame coming back.
fn remote_program() -> Vec<u32> {
    use vreteno32::isa::{addi, beq, halt, jal, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    // The serial port and the peripheral are on one page, so one
    // register addresses both.
    a.emit(lui(1, UART_BASE >> 12));
    a.emit(lui(4, 0xdead0));
    a.emit(addi(4, 4, 0x123)); // x4 = 0xdead0123
    a.emit(sw(4, 1, 0x300)); // the peripheral, at 0x3300
    a.emit(lw(5, 1, 0x300)); // and back from it
    let same = a.label();
    let done = a.label();
    a.to(same, |off| beq(5, 4, off));
    say(&mut a, b"remote bad\n");
    a.to(done, |off| jal(0, off));
    a.place(same);
    say(&mut a, b"remote ok\n");
    a.place(done);
    a.emit(halt());
    a.words()
}

/// A program that reads its own first word out of the boot memory over
/// the bus, tries to overwrite it, reads it again, and says whether the
/// memory kept it. The memory refuses the store, and the core raises
/// the store access fault for it (issue 417), so the program has a
/// handler that says so and returns to where the fault was taken.
fn rom_program() -> Vec<u32> {
    use vreteno32::isa::{
        addi, beq, csrrw, halt, jal, lui, lw, mret, sw, CSR_MTVEC, UART_BASE,
    };
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    let handler = a.label();
    a.abs(handler, |h| addi(7, 0, h as i32)); // x7 = the handler
    a.emit(csrrw(0, CSR_MTVEC, 7));
    a.emit(lui(6, 0)); // x6 = 0, the boot memory
    a.emit(lw(9, 6, 0)); // x9 = the program's first word, in a
                         // register the handler's say leaves alone
    a.emit(lui(4, 0xdead0));
    a.emit(addi(4, 4, 0x123)); // x4 = 0xdead0123
    a.emit(sw(4, 6, 0)); // refused, and answered so
    a.emit(lw(5, 6, 0)); // x5 = the word again
    let kept = a.label();
    let done = a.label();
    a.to(kept, |off| beq(5, 9, off));
    say(&mut a, b"rom changed\n");
    a.to(done, |off| jal(0, off));
    a.place(kept);
    say(&mut a, b"rom kept\n");
    a.place(done);
    a.emit(halt());
    // The store's fault: taken before whichever instruction was next
    // when the answer came back, so the handler returns to it as is.
    a.place(handler);
    say(&mut a, b"refused\n");
    a.emit(mret());
    a.words()
}

/// The boot memory is on the bus at zero: a load reads the program
/// that is there, and a store does not change it. The memory answers
/// the store with a refusal, which the core raises as a store access
/// fault (issue 417), so the program says `refused` from its handler
/// and then that the word is what it was; that the first load returned
/// the program and not zero is checked too, since a hole answers zero
/// as readily.
#[test]
fn the_boot_memory_is_readable_and_not_writable() {
    let text = rom_program();
    let ran = run(&text, b"", b"", 4000);
    assert_eq!(ran.said, "refused\nrom kept\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    assert_ne!(text[0], 0, "the word the program reads is not zero");
}

/// The core writes a word to a device that is a program on the other
/// side of the Ethernet port, reads it back, and gets what it wrote.
/// Nothing on the bus knows the device is software: the transaction
/// leaves the board as a frame and the answer arrives as one.
#[test]
fn the_core_reaches_a_program_across_the_ethernet_port() {
    let ran = run_served(&remote_program(), b"", 8000);
    assert_eq!(ran.said, "remote ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
    // Two transactions, two frames: the store and the load.
    assert_eq!(ran.sent.len(), 2, "a frame each");
}

/// What leaves the board is the protocol's frame, read here off the
/// port rather than out of the peripheral: the store the program above
/// makes, addressed to the peripheral's own address, carrying the word
/// and all four lanes, from this board's device number.
#[test]
fn a_transaction_leaves_the_board_as_a_frame() {
    let ran = run(&remote_program(), b"", b"", 2000);
    let frame = &ran.sent[0];
    assert_eq!(frame.len(), FRAME_LEN as usize, "one frame, whole");
    assert_eq!(frame[12..14], [0x88, 0xb5], "the type");
    assert_eq!(frame[14], KIND_ASK as u8, "an ask");
    assert_eq!(frame[15], REMOTE_DEV as u8, "this board");
    assert_eq!(frame[17] & 1, 1, "a write");
    assert_eq!(&frame[18..22], &0x3300u32.to_be_bytes(), "the address");
    assert_eq!(&frame[22..26], &0xdead_0123u32.to_be_bytes(), "the word");
    assert_eq!(frame[26], 0xf, "every lane");
    // With nothing answering, the core waits on the peripheral, which
    // waits `REMOTE_WAIT` cycles before it answers the bus itself.
    // That is a second on the board, longer than this run.
    assert_eq!(ran.said, "", "the core is still waiting");
    assert!(ran.halted_at.is_none(), "and has not halted");
}

/// One module holds the rest, and the controller is an instance of its
/// wrapper that the netlist does not write, with the memory's pads
/// running out to the board's own ports.
#[test]
fn the_netlist_holds_the_controller() {
    let v = TestBoard::verilog("board");
    assert!(v.contains("module board("), "the top");
    assert!(v.contains("module board_cpu("), "the core");
    assert!(v.contains("module board_ddr3_pins("), "the pins");
    assert!(v.contains("ddr3_axi32 "), "the controller");
    assert!(!v.contains("module ddr3_axi32"), "not written");
    assert!(v.contains("ring_osc "), "the rings");
    assert!(!v.contains("module ring_osc"), "not written either");
    assert!(v.contains("inout [31:0] dq"), "the data pads");
}

/// A program for the loader to load: it says `hi` on the serial port
/// and halts. It is assembled here rather than compiled, because a
/// compiled image is linked for the boot memory and this one runs
/// wherever the stream says, which is what a loader is for.
fn payload() -> Vec<u32> {
    use vreteno32::isa::{addi, blt, halt, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port
    for byte in b"hi\n" {
        // Wait while the port's queue is full, then write the byte.
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 0)); // x2 = txdata, bit 31 full
        a.to(wait, |off| blt(2, 0, off));
        a.emit(addi(3, 0, *byte as i32));
        a.emit(sw(3, 1, 0)); // the byte goes out
    }
    a.emit(halt());
    a.words()
}

/// Where the sender pauses for the loader's acknowledgement: the
/// header, then every word, then the checksum on its own. One is
/// `BLOCK_WORDS` in the loader, and the two have to agree.
fn blocks(words: usize) -> Vec<usize> {
    let mut out = vec![12];
    let mut left = words;
    while left > 0 {
        let take = if left > 1 { 1 } else { left };
        out.push(take * 4);
        left -= take;
    }
    out.push(4);
    out
}

/// The stream the loader takes: the magic word, the address, the
/// length, the words and their sum, every number least significant
/// byte first.
fn stream(addr: u32, words: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut word = |w: u32| out.extend_from_slice(&w.to_le_bytes());
    word(0x444c_5854);
    word(addr);
    word(words.len() as u32 * 4);
    let mut sum: u32 = 0;
    for w in words {
        word(*w);
        sum = sum.wrapping_add(*w);
    }
    word(sum);
    out
}

/// The loader takes a program off the serial port, writes it into the
/// memory's region and jumps to it, and the program runs.
#[test]
fn the_loader_loads_a_program_and_runs_it() {
    let addr = 0x4000_0000;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(addr, &payload()),
        &blocks(payload().len()),
        400_000,
    );
    // The loader says `boot`, then `load` when the header arrives, then
    // one `K` for the header and one for each word, then `ok` with the
    // address it jumps to; the program it loaded says `hi`.
    let acks = "K".repeat(payload().len() + 1);
    assert_eq!(
        ran.said,
        format!("boot\nload\n{acks}ok 40000000\nhi\n"),
        "typed {} of {} bytes, the last at cycle {} of {}",
        ran.typed,
        stream(addr, &payload()).len(),
        ran.typed_at,
        ran.ran_for
    );
    assert!(ran.halted_at.is_some(), "the loaded program halted");
}

/// A stream whose checksum does not match is refused, and the loader
/// waits for another rather than jumping into whatever arrived.
#[test]
fn the_loader_refuses_a_stream_whose_sum_is_wrong() {
    let addr = 0x4000_0000;
    let mut bytes = stream(addr, &payload());
    let last = bytes.len() - 4;
    bytes[last] ^= 0xff;
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &bytes,
        &blocks(payload().len()),
        400_000,
    );
    assert!(ran.said.contains("bad sum "), "{}", ran.said);
    // The second pass of the loop says so, which is what tells a
    // return from a loaded program apart from a reset (issue 413).
    assert!(
        ran.said.ends_with("boot again\n"),
        "it waits for another: {}",
        ran.said
    );
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}

/// An address outside the memory's region is refused, so a stream
/// cannot write over the peripherals.
#[test]
fn the_loader_refuses_an_address_outside_the_memory() {
    let ran = run_paced(
        boot_program::TEXT,
        boot_program::DATA,
        &stream(0x3000, &payload()),
        &blocks(payload().len()),
        200_000,
    );
    assert!(ran.said.starts_with("boot\nload\nbad len "), "{}", ran.said);
    assert!(ran.halted_at.is_none(), "nothing was jumped into");
}

/// A program that counts in `x5` to three thousand, says `done` and
/// halts: long enough to be caught in the middle by a debugger.
fn count_program() -> Vec<u32> {
    use vreteno32::isa::{addi, blt, halt, lui, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    a.emit(addi(5, 0, 0));
    a.emit(addi(6, 0, 1500));
    a.emit(addi(6, 6, 1500)); // x6 = 3000
    let again = a.label();
    a.place(again);
    a.emit(addi(5, 5, 1));
    a.to(again, |off| blt(5, 6, off));
    say(&mut a, b"done\n");
    a.emit(halt());
    a.words()
}

/// The debug module from the cable (issue 154): a debugger on the
/// JTAG pins halts the counting program, sees it halted, reads the
/// count and `dpc` by the abstract command, writes the count to its
/// end, resumes, and sees the core running with the resume
/// acknowledged. The program then finishes at once, which is the write
/// having landed in the register file: three thousand iterations take
/// nine thousand cycles, and the run ends well before that.
#[test]
fn the_debug_module_halts_reads_writes_and_resumes_the_core() {
    use vreteno32::debug::{
        access, at, ABSTRACTCS, ALLHALTED, ALLRESUMEACK, ALLRUNNING, COMMAND,
        DATA0, DMACTIVE, DMCONTROL, DMSTATUS, HALTREQ, HALTSUM0, REGNO_GPR,
        RESUMEREQ,
    };
    let plan = [
        Op::Wait(300),
        Op::Write(at(DMCONTROL), DMACTIVE),
        Op::Write(at(DMCONTROL), HALTREQ | DMACTIVE),
        Op::Wait(20),
        Op::Read(at(DMSTATUS)), // 0
        Op::Read(at(HALTSUM0)), // 1
        Op::Write(at(COMMAND), access(REGNO_GPR + 5, false)),
        Op::Wait(6),
        Op::Read(at(DATA0)), // 2: the count
        Op::Write(at(COMMAND), access(0x7b1, false)),
        Op::Wait(6),
        Op::Read(at(DATA0)),      // 3: dpc
        Op::Read(at(ABSTRACTCS)), // 4
        Op::Write(at(DATA0), 2999),
        Op::Write(at(COMMAND), access(REGNO_GPR + 5, true)),
        Op::Wait(6),
        Op::Read(at(ABSTRACTCS)), // 5
        Op::Write(at(DMCONTROL), DMACTIVE),
        Op::Write(at(DMCONTROL), RESUMEREQ | DMACTIVE),
        Op::Wait(20),
        Op::Read(at(DMSTATUS)), // 6
    ];
    let ran = run_debugged(&count_program(), &[], 12000, &plan);
    assert_eq!(
        ran.steps,
        plan.len(),
        "the plan ran through: {:x?}",
        ran.got
    );
    let got = &ran.got;
    assert_eq!(got[0] & ALLHALTED, ALLHALTED, "halted: {:#x}", got[0]);
    assert_eq!(got[0] & ALLRUNNING, 0);
    assert_eq!(got[1], 1, "haltsum0");
    assert!(got[2] > 0 && got[2] < 3000, "the count so far: {}", got[2]);
    assert!((16..24).contains(&got[3]), "dpc in the loop: {:#x}", got[3]);
    assert_eq!(got[4] >> 8 & 7, 0, "no command error");
    assert_eq!(got[5] >> 8 & 7, 0, "no command error on the write");
    assert_eq!(got[6] & ALLRUNNING, ALLRUNNING, "running: {:#x}", got[6]);
    assert_eq!(got[6] & ALLRESUMEACK, ALLRESUMEACK, "acknowledged");
    assert_eq!(ran.said, "done\n");
    let halted_at = ran.halted_at.expect("the program halted itself");
    assert!(
        halted_at < 3000,
        "the write to x5 ended the loop: {halted_at}"
    );
}

/// The cycle counter read twice back to back, from the data memory, and
/// the difference said eight times (issue 848). The routine runs from
/// the data memory through the instruction cache (issue 1021), whose
/// lines it fills on the first pass, so every difference is the same:
/// the cycles from one read to the next, which a read that lost a
/// count, as every read did before issue 807's fix, would make one
/// fewer.
#[test]
fn mcycle_steps_steadily_between_two_reads() {
    let ran = run(steps_program::TEXT, steps_program::DATA, b"", 40000);
    assert!(ran.halted_at.is_some(), "and halted: {}", ran.said);
    let steps: Vec<u32> = ran
        .said
        .lines()
        .map(|l| {
            l.strip_prefix("mcycle step ")
                .unwrap_or_else(|| panic!("not a step: {l:?}"))
                .parse()
                .expect("a number")
        })
        .collect();
    assert_eq!(steps.len(), 8, "eight differences: {}", ran.said);
    assert!(
        steps.iter().all(|&s| s == steps[0]),
        "every difference the same: {steps:?}"
    );
    // Two cycles from one read to the next, the second read's word
    // in the buffer with the first's (issue 1279); three with the cache
    // alone. Fetched over the bus a word at a time, before the cache,
    // it was thirteen; with the core of before issue 807's fix a steady
    // twelve, each read costing the counter the one count it wrote back
    // over.
    assert_eq!(steps[0], 2, "cycles between the two reads");
}

/// The path into DDR3 timed by the core (issue 1023): sixteen loads,
/// then sixteen stores and a fence, from the data memory, against the
/// DDR3 and against the data memory, four times each. Every run of a
/// kind takes the same cycles.
///
/// A load waits for its word, so the loads' difference is what a word
/// costs the DDR3's path over the block RAM's: the controller's read
/// latency, which the model has. It was that less two cycles, since the
/// pins part and the controller's port took two cycles fewer than the
/// data memory's tracker and block RAM; the tracker hands the request
/// and the read beat on in the cycle (issue 1291), and the two paths
/// take the same. On the board it is the controller's own latency,
/// which is what the board run is for.
///
/// The stores into the DDR3 take seven cycles more than into the data
/// memory, over sixteen. Before the instruction cache (issue 1021) the
/// two were equal: the core issued stores no faster than it fetched the
/// routine over the bus, so they showed that the core could not fill
/// the path. From the cache it issues them faster than the DDR3's path
/// takes them, and the difference is the path's. It was eight with the
/// cache alone; the fetch window (issue 1187) moves where the stores
/// fall against the path's cycles, and it is seven.
#[test]
fn the_ddr3_path_is_timed_by_the_core() {
    let ran = run(ddr3bw_program::TEXT, ddr3bw_program::DATA, b"", 80000);
    assert!(ran.halted_at.is_some(), "and halted: {}", ran.said);
    let of = |what: &str| -> Vec<u32> {
        ran.said
            .lines()
            .filter_map(|l| l.strip_prefix(&format!("bw {what} ")))
            .map(|n| n.parse().expect("a number"))
            .collect()
    };
    let (ld, lm) = (of("load ddr3"), of("load dmem"));
    let (sd, sm) = (of("store ddr3"), of("store dmem"));
    for (what, v) in [
        ("load ddr3", &ld),
        ("load dmem", &lm),
        ("store ddr3", &sd),
        ("store dmem", &sm),
    ] {
        assert_eq!(v.len(), 4, "four runs of {what}: {}", ran.said);
        // The first run of each fills the instruction cache with the
        // routine's lines (issue 1021), so the runs after it are the
        // steady ones.
        assert!(v[1..].iter().all(|&c| c == v[1]), "{what} steady: {v:?}");
    }
    let (ld, lm, sd, sm) =
        (ld[1] as i64, lm[1] as i64, sd[1] as i64, sm[1] as i64);
    let latency = ddr3::MODEL_READ_LATENCY as i64;
    assert_eq!(
        ld - lm,
        16 * latency,
        "a load costs the DDR3 the controller's latency"
    );
    assert_eq!(sd - sm, 7, "the stores into the DDR3 wait on its path");
}

/// The DDR3's writes, reads and strobes, by the program the loader
/// sends to the flagship (issue 1174), run here from the boot memory
/// through the controller's model: a block of words, words up to the
/// top of the gigabyte, bytes and halfwords written under their
/// strobes, and bytes and halfwords read back at every offset.
#[test]
fn the_loaded_memory_test_passes_through_the_model() {
    let ran = run(ddr3ram_program::TEXT, ddr3ram_program::DATA, b"", 200000);
    assert_eq!(
        ran.said,
        "ddr3ram words ok\nddr3ram high ok\nddr3ram strobes ok\n\
         ddr3ram lanes ok\nddr3ram ok\n"
    );
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// `rd` = `v`, in two instructions, the upper part rounded for the
/// sign of the lower.
fn li(a: &mut vreteno32::program::Asm, rd: u32, v: u32) {
    use vreteno32::isa::{addi, lui};
    let lo = ((v & 0xfff) as i32) << 20 >> 20;
    a.emit(lui(rd, (v.wrapping_sub(lo as u32)) >> 12));
    a.emit(addi(rd, rd, lo));
}

/// A program that draws with Razboj (issue 985): it writes a display
/// list of one rectangle into the DDR3, a word that the rectangle will
/// not cover beside it, and the count into the doorbell; waits for the
/// doorbell to read zero and for the rasteriser to say it is idle; and
/// says whether the rectangle's corners took its colour and the word
/// beside it is as it was.
fn razboj_program() -> Vec<u32> {
    use razboj::dl::encode;
    use razboj::op::{Insn, Kind};
    use vreteno32::board::{RAZBOJ_DL, RAZBOJ_DOORBELL, RAZBOJ_FB};
    use vreteno32::isa::{beq, bne, halt, jal, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
                                     // x15 = the doorbell, which is past an immediate's reach from x1.
    li(&mut a, 15, RAZBOJ_DOORBELL as u32);
    let pixel = |x: u32, y: u32| RAZBOJ_FB as u32 + (y * 1024 + x) * 4;
    // Columns 3 to 6 and rows 2 to 4, both ends included.
    let words = encode(&Insn {
        kind: Kind::Rect,
        colour: U::from(0x12_3456u32),
        alpha: U::from(0xffu8),
        x0: U::from(3u32),
        y0: U::from(2u32),
        x1: U::from(6u32),
        y1: U::from(4u32),
        ..Insn::default()
    });
    li(&mut a, 10, RAZBOJ_DL as u32);
    for (i, w) in words.iter().enumerate() {
        li(&mut a, 4, *w);
        a.emit(sw(4, 10, 4 * i as i32));
    }
    li(&mut a, 11, pixel(7, 4)); // past the rectangle's last column
    li(&mut a, 12, 0x0bad_f00d);
    a.emit(sw(12, 11, 0));
    // Ring: one entry. Then wait for the count to come back to zero,
    // and for the status to say idle.
    li(&mut a, 4, 1);
    a.emit(sw(4, 15, 0)); // count
    let drawn = a.label();
    a.place(drawn);
    a.emit(lw(5, 15, 0)); // count
    a.to(drawn, |off| bne(5, 0, off));
    let idle = a.label();
    a.place(idle);
    a.emit(lw(5, 15, 4)); // status
    a.to(idle, |off| beq(5, 0, off));
    li(&mut a, 13, 0xff12_3456); // the word a pixel takes
    let bad = a.label();
    let done = a.label();
    for (x, y) in [(3, 2), (6, 2), (3, 4), (6, 4)] {
        li(&mut a, 14, pixel(x, y));
        a.emit(lw(5, 14, 0));
        a.to(bad, |off| bne(5, 13, off));
    }
    a.emit(lw(5, 11, 0));
    a.to(bad, |off| bne(5, 12, off));
    say(&mut a, b"razboj ok\n");
    a.to(done, |off| jal(0, off));
    a.place(bad);
    say(&mut a, b"razboj bad\n");
    a.place(done);
    a.emit(halt());
    a.words()
}

/// A program that has Razboj fill the screen the scanout shows, 640 by
/// 480, with one rectangle, waits for the doorbell to read zero and the
/// rasteriser to say it is idle, checks two opposite corners, and halts:
/// the cycle it halts at is the draw's time, give or take the few
/// hundred the program takes around it (issue 987).
fn razboj_fill_program() -> Vec<u32> {
    use razboj::dl::encode;
    use razboj::op::{Insn, Kind};
    use vreteno32::board::{RAZBOJ_DL, RAZBOJ_DOORBELL, RAZBOJ_FB};
    use vreteno32::isa::{beq, bne, halt, jal, lui, lw, sw, UART_BASE};
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1 = the serial port, for say
    li(&mut a, 15, RAZBOJ_DOORBELL as u32);
    let words = encode(&Insn {
        kind: Kind::Rect,
        colour: U::from(0x12_3456u32),
        alpha: U::from(0xffu8),
        x0: U::from(0u32),
        y0: U::from(0u32),
        x1: U::from(639u32),
        y1: U::from(479u32),
        ..Insn::default()
    });
    li(&mut a, 10, RAZBOJ_DL as u32);
    for (i, w) in words.iter().enumerate() {
        li(&mut a, 4, *w);
        a.emit(sw(4, 10, 4 * i as i32));
    }
    li(&mut a, 4, 1);
    a.emit(sw(4, 15, 0)); // ring for the one entry
    let drawn = a.label();
    a.place(drawn);
    a.emit(lw(5, 15, 0));
    a.to(drawn, |off| bne(5, 0, off));
    let idle = a.label();
    a.place(idle);
    a.emit(lw(5, 15, 4));
    a.to(idle, |off| beq(5, 0, off));
    li(&mut a, 13, 0xff12_3456);
    let bad = a.label();
    let done = a.label();
    for at in [0, (479 * 1024 + 639) * 4] {
        li(&mut a, 14, RAZBOJ_FB as u32 + at);
        a.emit(lw(5, 14, 0));
        a.to(bad, |off| bne(5, 13, off));
    }
    say(&mut a, b"razboj full\n");
    a.to(done, |off| jal(0, off));
    a.place(bad);
    say(&mut a, b"razboj bad\n");
    a.place(done);
    a.emit(halt());
    a.words()
}

/// Razboj fills the screen on the board's model, through the arbiter
/// and the DDR3's controller, with nothing else on the bus (issue 987).
/// A row's pixels go out as bursts of sixteen, so the core halts at
/// cycle 327426, about one a pixel of the 307200; written a pixel a
/// burst, as before, it halted at 1229346, four a pixel.
#[test]
fn razboj_fills_the_screen_in_bursts() {
    let ran = run(&razboj_fill_program(), &[], b"", 4_000_000);
    assert_eq!(ran.said, "razboj full\n");
    let at = ran.halted_at.expect("the core halted itself");
    eprintln!("razboj fill: halted at cycle {at}");
    assert!(at < 360_000, "the fill took until cycle {at}");
}

/// The word in `rs` as eight hexadecimal digits on the serial port, `x1`
/// holding the port's page; `x2`, `x3` and `x4` are used.
fn say_hex(a: &mut vreteno32::program::Asm, rs: u32) {
    use vreteno32::isa::{addi, andi, blt, lw, srli, sw};
    for k in (0..8).rev() {
        a.emit(srli(4, rs, 4 * k));
        a.emit(andi(4, 4, 15));
        a.emit(addi(3, 4, b'0' as i32));
        a.emit(addi(2, 0, 10));
        let digit = a.label();
        a.to(digit, |off| blt(4, 2, off));
        a.emit(addi(3, 4, b'a' as i32 - 10));
        a.place(digit);
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 0));
        a.to(wait, |off| blt(2, 0, off));
        a.emit(sw(3, 1, 0));
    }
}

/// A program that waits while Razboj draws a list the debugger's plan
/// wrote and rang (issue 1255): the count read until it is not zero,
/// then until it is zero again with the status idle, the cycle counter
/// read at each end. It says `razboj cycles` and the cycles between,
/// then reads each of `checks`, a byte address and the word it must
/// hold, and says `razboj ok` or `razboj bad`, and halts.
fn razboj_wait_program(checks: &[(u32, u32)]) -> Vec<u32> {
    use vreteno32::board::RAZBOJ_DOORBELL;
    use vreteno32::isa::{
        beq, bne, csrrs, halt, jal, lui, lw, sub, CSR_MCYCLE, UART_BASE,
    };
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12));
    li(&mut a, 15, RAZBOJ_DOORBELL as u32);
    let rung = a.label();
    a.place(rung);
    a.emit(lw(5, 15, 0));
    a.to(rung, |off| beq(5, 0, off));
    a.emit(csrrs(20, CSR_MCYCLE, 0));
    let drawn = a.label();
    a.place(drawn);
    a.emit(lw(5, 15, 0));
    a.to(drawn, |off| bne(5, 0, off));
    let idle = a.label();
    a.place(idle);
    a.emit(lw(5, 15, 4));
    a.to(idle, |off| beq(5, 0, off));
    a.emit(csrrs(21, CSR_MCYCLE, 0));
    a.emit(sub(22, 21, 20));
    say(&mut a, b"razboj cycles ");
    say_hex(&mut a, 22);
    say(&mut a, b"\n");
    let bad = a.label();
    let done = a.label();
    for &(at, want) in checks {
        li(&mut a, 14, at);
        a.emit(lw(5, 14, 0));
        li(&mut a, 13, want);
        a.to(bad, |off| bne(5, 13, off));
    }
    say(&mut a, b"razboj ok\n");
    a.to(done, |off| jal(0, off));
    a.place(bad);
    say(&mut a, b"razboj bad\n");
    a.place(done);
    a.emit(halt());
    a.words()
}

/// The debugger's plan for a list on the board: `words` written at the
/// list's address, only those that are not zero since the DDR3 starts
/// at zero, then `marks`, each a byte address and a word, and last the
/// count rung on the doorbell.
fn razboj_plan(words: &[u32], marks: &[(u32, u32)], count: u32) -> Vec<Op> {
    use vreteno32::board::{RAZBOJ_DL, RAZBOJ_DOORBELL};
    let mut plan: Vec<Op> = words
        .iter()
        .enumerate()
        .filter(|(_, &w)| w != 0)
        .map(|(i, &w)| Op::Write(RAZBOJ_DL as u32 + 4 * i as u32, w))
        .collect();
    plan.extend(marks.iter().map(|&(at, w)| Op::Write(at, w)));
    plan.push(Op::Write(RAZBOJ_DOORBELL as u32, count));
    plan
}

/// The cycles a `razboj_wait_program` run said Razboj took.
fn razboj_cycles(said: &str) -> u64 {
    let hex = said
        .lines()
        .find_map(|l| l.strip_prefix("razboj cycles "))
        .unwrap_or_else(|| panic!("no cycles in {said:?}"));
    u64::from_str_radix(hex, 16).unwrap()
}

/// Razboj draws a list in tiles on the board (issue 1255): a rectangle
/// and a shaded triangle across the edges of tiles, binned into a tile
/// table, drawn through the tile buffer and written out through the
/// arbiter into the DDR3. Every pixel checked is the model's, and a word
/// inside a tile the list touches but at a pixel no entry covers keeps
/// what the debugger wrote there, since the write-out's strobes are off
/// where nothing was drawn.
#[test]
fn razboj_draws_a_list_in_tiles() {
    use razboj::model::render;
    use razboj::op::{assemble, Op as Draw};
    use vreteno32::board::RAZBOJ_FB;
    let (sw, sh) = (1024usize, 480usize);
    let list = assemble(
        &[
            Draw::Rect {
                colour: 0xff12_3456,
                x: 40,
                y: 50,
                w: 40,
                h: 30,
            },
            Draw::Gouraud {
                a: (100 * 16, 20 * 16 + 8),
                b: (150 * 16 + 4, 90 * 16),
                c: (60 * 16, 130 * 16 + 12),
                colours: [0xffff_0000, 0x00_ff00, 0x00_00ff],
            },
        ],
        sw,
        sh,
    );
    let (words, count) = razboj::tiles::image(&list, sw, sh);
    assert!(count & 0xffff >= 4, "tiles: {}", count & 0xffff);
    let want = render(&list, sw, sh);
    let at = |x: usize, y: usize| RAZBOJ_FB as u32 + 4 * (y * sw + x) as u32;
    // A pixel of tile (0, 0), which the rectangle touches, that nothing
    // covers.
    let mark = (at(10, 10), 0x0bad_f00du32);
    assert_eq!(want[10 * sw + 10], 0, "the marked pixel is not drawn");
    let mut checks = vec![mark];
    for &(x, y) in
        &[(40, 50), (79, 79), (63, 60), (64, 60), (100, 40), (90, 100)]
    {
        checks.push((at(x, y), want[y * sw + x]));
    }
    assert!(
        checks.iter().skip(1).all(|c| c.1 != 0),
        "every check is drawn"
    );
    let plan = razboj_plan(&words, &[mark], count);
    let ran = run_all(
        &razboj_wait_program(&checks),
        &[],
        b"",
        &[],
        600_000,
        Net::default(),
        &plan,
    );
    assert!(ran.said.ends_with("razboj ok\n"), "{}", ran.said);
    eprintln!("razboj tiles: {} cycles", razboj_cycles(&ran.said));
}

/// Razboj tests depth on the board (issue 992): two triangles that cross
/// in depth, each nearer than the other over part of it, under
/// `GL_LESS` with depth written over a clear of depth, in a tile table
/// the debugger writes into the DDR3 and rings. The pixels checked, on
/// both sides of where the two cross, are the model's, so the nearer
/// triangle is the one seen at each.
#[test]
fn razboj_tests_depth_in_tiles() {
    use razboj::model::render;
    use razboj::op::{assemble, DepthMode, Op as Draw, ALWAYS, LESS};
    use vreteno32::board::RAZBOJ_FB;
    let (sw, sh) = (1024usize, 480usize);
    let list = assemble(
        &[
            Draw::Depth(Some(DepthMode {
                func: ALWAYS,
                write: true,
            })),
            Draw::RectZ {
                colour: 0xff20_2020,
                x: 0,
                y: 0,
                w: 128,
                h: 96,
                z: 0xffff,
            },
            Draw::Depth(Some(DepthMode {
                func: LESS,
                write: true,
            })),
            Draw::TriZ {
                colour: 0xffc0_4000,
                a: (8 * 16, 10 * 16),
                b: (120 * 16, 14 * 16),
                c: (30 * 16, 90 * 16),
                z: [0x1000, 0xf000, 0x8000],
            },
            Draw::GouraudZ {
                a: (110 * 16, 10 * 16),
                b: (100 * 16, 92 * 16),
                c: (10 * 16, 40 * 16),
                colours: [0xffff_ff00, 0xff00_ffff, 0xffff_00ff],
                z: [0x2000, 0x3000, 0xe000],
            },
        ],
        sw,
        sh,
    );
    let (words, count) = razboj::tiles::image(&list, sw, sh);
    let want = render(&list, sw, sh);
    let at = |x: usize, y: usize| RAZBOJ_FB as u32 + 4 * (y * sw + x) as u32;
    // Where the flat triangle is nearer, where the shaded one is, and the
    // clear where neither reaches.
    let flat = 0xffc0_4000u32;
    let near_flat = (0..96)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .find(|&(x, y)| want[y * sw + x] == flat)
        .expect("the flat triangle is nearer somewhere");
    let near_shaded = (0..96)
        .flat_map(|y| (0..128).map(move |x| (x, y)))
        .find(|&(x, y)| {
            let p = want[y * sw + x];
            p != flat && p != 0xff20_2020 && p != 0
        })
        .expect("the shaded triangle is nearer somewhere");
    let checks: Vec<(u32, u32)> = [near_flat, near_shaded, (126, 94), (64, 50)]
        .iter()
        .map(|&(x, y)| (at(x, y), want[y * sw + x]))
        .collect();
    let plan = razboj_plan(&words, &[], count);
    let ran = run_all(
        &razboj_wait_program(&checks),
        &[],
        b"",
        &[],
        1_000_000,
        Net::default(),
        &plan,
    );
    assert!(ran.said.ends_with("razboj ok\n"), "{}", ran.said);
    eprintln!("razboj depth: {} cycles", razboj_cycles(&ran.said));
}

/// Razboj's draw time on the board's model, flat and in tiles (issue
/// 1255): the screen filled, 640 by 480, and the icosahedron's first
/// frame, which clears the screen and draws the faces. Each list is
/// written and rung by the debugger, and the core says the cycles
/// between the ring and the drawing done. Ignored by default, since the
/// runs take many minutes; run it with `--test_arg=--ignored`. In tiles
/// a plain fill is slower, by design, until the second bank lets a tile
/// be drawn while the last is written out, and depth gives the tile
/// buffer work that memory could not do cheaply.
#[test]
#[ignore = "minutes of simulation: run with --test_arg=--ignored"]
fn razboj_draws_flat_and_in_tiles_timed() {
    use razboj::dl::decode;
    use razboj::model::render;
    use razboj::op::{assemble, Insn, Op as Draw};
    use vreteno32::board::RAZBOJ_FB;
    let (sw, sh) = (1024usize, 480usize);
    let fill = assemble(
        &[Draw::Rect {
            colour: 0xff12_3456,
            x: 0,
            y: 0,
            w: 640,
            h: 480,
        }],
        sw,
        sh,
    );
    let solid = ico_list::Solid::new();
    let mut out = [[0u32; ico_list::WORDS]; ico_list::MOST];
    let (n, _) =
        ico_list::frame(&solid, 0, 0, 0, ico_list::Box::SCREEN, &mut out);
    let ico: Vec<Insn> = out[..n].iter().map(|w| decode(w)).collect();
    for (name, list) in [("fill", fill), ("icosahedron", ico)] {
        let want = render(&list, sw, sh);
        let at = |x: usize, y: usize| {
            (RAZBOJ_FB as u32 + 4 * (y * sw + x) as u32, want[y * sw + x])
        };
        let checks = [at(5, 5), at(320, 240), at(639, 479)];
        let flat = razboj::dl::image(&list);
        let (tiled, count) = razboj::tiles::image(&list, sw, sh);
        for (how, words, count) in
            [("flat", flat, list.len() as u32), ("tiled", tiled, count)]
        {
            let plan = razboj_plan(&words, &[], count);
            let ran = run_all(
                &razboj_wait_program(&checks),
                &[],
                b"",
                &[],
                8_000_000,
                Net::default(),
                &plan,
            );
            assert!(
                ran.said.ends_with("razboj ok\n"),
                "{name} {how}: {}",
                ran.said
            );
            eprintln!(
                "razboj {name} {how}: {} entries, {} tiles, {} cycles",
                list.len(),
                if how == "flat" { 0 } else { count & 0xffff },
                razboj_cycles(&ran.said)
            );
        }
    }
}

/// Razboj on the board (issue 985): a program writes a display list
/// into the DDR3 and rings the doorbell, the rasteriser draws it into
/// the frame the scanout shows, through the arbiter's seventh port,
/// and the program reads the pixels back. The only evidence is what
/// the core reads through the real bus, so a pass covers the doorbell
/// on the tenth slot, the rasteriser's poll of it, its fetch of the
/// list and its bursts into the frame at the scanout's stride.
#[test]
fn razboj_draws_a_list_rung_on_the_doorbell() {
    let text = razboj_program();
    let ran = run(&text, &[], b"", 60000);
    assert_eq!(ran.said, "razboj ok\n");
    assert!(ran.halted_at.is_some(), "the core halted itself");
}

/// Every slave on the board's map answers every legal burst (issue
/// 1196): for each range of `BoardMap`, each slot of the peripheral
/// page's `SlotMap` and a hole, a read and a write of 1, 2, 4, 8 and 16
/// beats, incrementing and a word wide, from the debugger's port.
///
/// A read gets exactly as many beats as it asked for, the last of them
/// and only the last marked last; a write gets one response, and not
/// before its last beat has gone; every beat and response carries the
/// burst's identifier; nothing waits out `BURST_BOUND`; and the hole
/// answers `DecErr`. The core spins in place meanwhile, the remote
/// peripheral has a program on the wire, and the third slot answers as
/// the flagship's video peripheral would, so that every slave is there
/// to answer.
///
/// The core makes single beats only, so no test sent a burst to most
/// of these before, and the boot memory answered sixteen beats with
/// one, which hung the scanout (#1178).
#[test]
fn every_slave_answers_every_burst() {
    let mut slaves: Vec<(u32, &str)> = BoardMap::RANGES
        .iter()
        .zip(BoardMap::NAMES)
        .filter(|((base, _), _)| *base != 0x3000)
        .map(|((base, _), name)| (*base as u32, name))
        .collect();
    slaves.extend(
        SlotMap::RANGES
            .iter()
            .zip(SlotMap::NAMES)
            .map(|((base, _), name)| (*base as u32, name)),
    );
    let hole = 0x2000u32;
    slaves.push((hole, "no slave: the hole above the data memory"));
    // Each slave on a board of its own, so that one that wedges the bus
    // does not take the next one's bursts with it.
    let mut seen = Vec::new();
    let mut asked = Vec::new();
    for &(base, name) in &slaves {
        let mut plan = Vec::new();
        let mut mine = Vec::new();
        for beats in [1u32, 2, 4, 8, 16] {
            for write in [false, true] {
                let id = (mine.len() & 1) as u8;
                plan.push(if write {
                    Op::WriteBurst(base, beats, id)
                } else {
                    Op::ReadBurst(base, beats, id)
                });
                plan.push(Op::Wait(16));
                mine.push((name, base, beats, write));
            }
        }
        let net = Net {
            serve: true,
            video: true,
            until_planned: true,
            ..Net::default()
        };
        let spin = [vreteno32::isa::jal(0, 0)];
        let ran = run_all(&spin, &[], b"", &[], 2_000_000, net, &plan);
        assert_eq!(ran.bursts.len(), mine.len(), "every burst to {name}");
        seen.extend(ran.bursts);
        asked.extend(mine);
    }
    let mut wrong = Vec::new();
    let mut slowest = 0;
    for (seen, &(name, base, beats, write)) in seen.iter().zip(&asked) {
        let what = format!(
            "{name} at {base:#x}, a {} of {beats}",
            if write { "write" } else { "read" }
        );
        slowest = slowest.max(seen.cycles);
        if !seen.done {
            wrong.push(format!("{what}: no answer in {BURST_BOUND} cycles"));
            continue;
        }
        if write {
            if seen.bs != 1 {
                wrong.push(format!("{what}: {} write responses", seen.bs));
            }
            if seen.b_early {
                wrong.push(format!("{what}: answered before its last beat"));
            }
        } else {
            if seen.beats != beats {
                wrong.push(format!("{what}: {} beats came back", seen.beats));
            }
            if seen.lasts != [beats - 1] {
                wrong.push(format!("{what}: last on beats {:?}", seen.lasts));
            }
        }
        if !seen.ids_ok {
            wrong.push(format!("{what}: another burst's identifier"));
        }
        if base == hole && seen.resps.iter().any(|&r| r != 3) {
            wrong.push(format!("{what}: {:?}, not DecErr", seen.resps));
        }
    }
    eprintln!("{} bursts, the slowest {slowest} cycles", asked.len());
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A scanout whose line never comes says so (issue 1197). Its base is
/// the third slot of the peripheral page, which these runs tie off, so
/// the fetch of the first line waits for good, as the boot memory's one
/// beat made it wait on the board (#1178). Two line times after that
/// line was asked for, the pair sets `stuck` with the line's address,
/// rather than leaving a black screen and a clear underflow bit.
#[test]
fn a_line_that_never_comes_is_stuck() {
    let frame = 6 * SCAN_LINE;
    let net = Net {
        scan: Some(Scan {
            base: 0x3200,
            show_at: frame,
        }),
        ..Net::default()
    };
    let ran = run_all(
        hello_program::TEXT,
        hello_program::DATA,
        b"",
        &[],
        4 * frame,
        net,
        &[],
    );
    let first = ran.scan.lines.first().expect("a line was asked for");
    assert_eq!(first.0, 0x3200, "the base was asked for first");
    assert!(first.2.is_none(), "and it never came");
    let (when, at) = ran.scan.stuck.expect("the pair said it was stuck");
    assert_eq!(at, 0x3200, "the line that did not come");
    assert!(
        when <= first.1 + 3 * SCAN_LINE,
        "stuck at {when}, the line asked for at {}",
        first.1
    );
}

/// The load `scanprobe` puts on the board while it watches the scanout
/// (issue 1209): the core copies 256 KiB back and forth in the DDR3, a
/// word at a time with no pause, and every sixteen words starts the
/// Ethernet port sending a full frame from the DDR3 if it is ready.
/// With `razboj`, every sixteen words it also rings Razboj for a
/// full-screen rectangle when the last one is drawn, which writes the
/// frame a pixel a beat: a seventh host on the same memory.
fn load_program(razboj: bool) -> Vec<u32> {
    use txhdl_parts::ethslots::regs;
    use vreteno32::board::{RAZBOJ_DL, RAZBOJ_DOORBELL};
    use vreteno32::isa::{addi, andi, beq, bne, jal, lw, sw};
    let mut a = vreteno32::program::Asm::default();
    li(&mut a, 10, 0x3400); // x10: the Ethernet port's registers
    li(&mut a, 4, 1514);
    a.emit(sw(4, 10, regs::tx_length as i32));
    a.emit(sw(0, 10, regs::tx_slot as i32));
    li(&mut a, 14, RAZBOJ_DOORBELL as u32); // x14: the doorbell
    if razboj {
        // The list: one rectangle over the visible 640 by 480.
        li(&mut a, 15, RAZBOJ_DL as u32);
        let rect = [1 | (0x12_3456 << 2), 0, 639 | (479 << 16)];
        for (i, w) in rect.iter().enumerate() {
            li(&mut a, 4, *w);
            a.emit(sw(4, 15, 4 * i as i32));
        }
    }
    li(&mut a, 11, 0x4300_0000); // one half
    li(&mut a, 12, 0x4304_0000); // the other
    let outer = a.label();
    a.place(outer);
    a.emit(addi(5, 11, 0)); // from
    a.emit(addi(6, 12, 0)); // to
    li(&mut a, 7, 0x4004_0000);
    a.emit(vreteno32::isa::add(7, 7, 11)); // from's end: from + 256 KiB
    li(&mut a, 13, 0x4000_0000);
    a.emit(vreteno32::isa::sub(7, 7, 13));
    let word = a.label();
    let skip = a.label();
    a.place(word);
    a.emit(lw(8, 5, 0));
    a.emit(sw(8, 6, 0));
    a.emit(addi(5, 5, 4));
    a.emit(addi(6, 6, 4));
    a.emit(andi(9, 5, 63));
    a.to(skip, |off| bne(9, 0, off));
    if razboj {
        let busy = a.label();
        a.emit(lw(9, 14, 0)); // the count
        a.to(busy, |off| bne(9, 0, off));
        a.emit(addi(9, 0, 1));
        a.emit(sw(9, 14, 0)); // ring for the one rectangle
        a.place(busy);
    }
    a.emit(lw(9, 10, regs::tx_ready as i32));
    a.to(skip, |off| beq(9, 0, off));
    a.emit(addi(9, 0, 1));
    a.emit(sw(9, 10, regs::tx_start as i32));
    a.place(skip);
    a.to(word, |off| bne(5, 7, off));
    // The halves change places, and the copy goes back.
    a.emit(addi(13, 11, 0));
    a.emit(addi(11, 12, 0));
    a.emit(addi(12, 13, 0));
    a.to(outer, |off| jal(0, off));
    a.words()
}

/// How a run's lines came: the longest any took from being asked for
/// to its last word, the mean, and how many took longer than a line.
fn line_times(log: &ScanLog) -> (u64, u64, usize, usize) {
    let took: Vec<u64> = log
        .lines
        .iter()
        .filter_map(|(_, asked, got)| got.map(|g| g - asked))
        .collect();
    let late = took.iter().filter(|&&t| t > SCAN_LINE).count();
    let max = took.iter().copied().max().unwrap_or(0);
    let mean = took.iter().sum::<u64>() / took.len().max(1) as u64;
    (max, mean, late, took.len())
}

/// The scanout against the board's own load, a line as long as the
/// board's (issue 1209): idle, with the core copying in the DDR3 and the
/// Ethernet port sending, and with Razboj drawing as well. Every line
/// must come within half of the line it has, and no column may be shown
/// before its word.
///
/// With the scanout's bursts of sixteen beats, as before #1209, the
/// longest line took 1964 cycles idle, 4682 under the load and 4754
/// with Razboj too, against a line of 3175: every line late under load,
/// as the board showed. With the flagship's bursts of 64 the same runs
/// take 974, 1206 and 1200; with bursts of 128 they took 809, 942 and
/// 904, at a cost to the core that `a_core_load_waits_behind_the_scanout`
/// measures.
///
/// Each run is some 25 lines. Over 77 the longest line came first at
/// line 0 idle, at line 8 under the load, and at line 44 with Razboj,
/// where it was 1187 against 1172 by line 18; all of them are some 400
/// cycles inside the half line the test allows, so 25 lines say what 77
/// said in a third of the time (issue 1250).
#[test]
fn the_scanout_keeps_up_with_the_boards_load() {
    let rows = 6 * 7;
    for (what, text) in [
        ("idle", hello_program::TEXT.to_vec()),
        ("load", load_program(false)),
        ("load and Razboj", load_program(true)),
    ] {
        let net = Net {
            scan: Some(Scan {
                base: 0x4200_0000,
                show_at: 6 * SCAN_LINE,
            }),
            ..Net::default()
        };
        let ran = run_all(
            &text,
            hello_program::DATA,
            b"",
            &[],
            (rows + 6) * SCAN_LINE,
            net,
            &[],
        );
        let (max, mean, late, n) = line_times(&ran.scan);
        eprintln!(
            "scan {what}: {n} lines, longest {max} cycles, mean {mean}, \
             {late} longer than a line of {SCAN_LINE}, starved {:?}, \
             frames sent {}",
            ran.scan.starved,
            ran.sent.len()
        );
        assert!(n >= 20, "{what}: only {n} lines came");
        assert_eq!(late, 0, "{what}: lines later than a line");
        assert_eq!(ran.scan.starved, None, "{what}: a column starved");
        assert!(
            2 * max < SCAN_LINE,
            "{what}: the longest line took {max} of {SCAN_LINE} cycles"
        );
    }
}

/// The serial line's reset, in the flagship's cycles: a pulse of
/// 100 000, from the count of 2 000 000 to 2 100 000 in
/// `flagship/board/flagship.v`.
const BRK_PULSE: u64 = 100_000;

/// The serial line's reset in the middle of the board's load, as
/// `load --reset` gives the flagship (issue 1317): the core copying in
/// the DDR3, the Ethernet port sending, Razboj drawing, and the scanout
/// reading its lines, when the design and the scanout are reset and the
/// DDR3 controller, which that reset does not reach, is not. Once the
/// reset ends, the scanout must have its lines again: on the board, one
/// such reset in some twenty left no line arriving at all.
///
/// The reset lands at a different cycle in each run, so that across
/// them it meets the bus in different states.
#[test]
fn the_bus_answers_after_a_reset_in_the_middle_of_its_load() {
    let starts: Vec<u64> = std::env::var("BRK_STARTS")
        .ok()
        .map(|s| s.split(',').map(|v| v.parse().unwrap()).collect())
        .unwrap_or_else(|| vec![9 * SCAN_LINE]);
    for from in starts {
        let net = Net {
            scan: Some(Scan {
                base: 0x4200_0000,
                show_at: 6 * SCAN_LINE,
            }),
            reset_at: Some((from, BRK_PULSE)),
            ..Net::default()
        };
        let ran = run_all(
            &load_program(true),
            hello_program::DATA,
            b"",
            &[],
            from + BRK_PULSE + 6 * 6 * SCAN_LINE,
            net,
            &[],
        );
        let back = ran.reset_from.expect("the reset began") + BRK_PULSE;
        // The lines asked for once the reset has been over for a frame.
        let after: Vec<_> = ran
            .scan
            .lines
            .iter()
            .filter(|(_, asked, _)| {
                *asked > back + 12 * SCAN_LINE
                    && *asked + 3 * SCAN_LINE < ran.ran_for
            })
            .collect();
        let came = after.iter().filter(|(_, _, got)| got.is_some()).count();
        eprintln!(
            "reset at {from}: {} lines asked after it, {came} came, \
             stuck {:?}, frames sent {}",
            after.len(),
            ran.scan.stuck,
            ran.sent.len()
        );
        assert!(
            after.len() >= 10,
            "reset at {from}: only {} asked",
            after.len()
        );
        assert!(
            came + 1 >= after.len(),
            "reset at {from}: {came} of {} lines came",
            after.len()
        );
    }
}

/// A program that times the core's loads from the DDR3 (issue 1209):
/// two thousand single-word loads, 64 bytes apart, each between two
/// reads of `mcycle` and used at once, and the longest said in hex.
fn load_wait_program() -> Vec<u32> {
    use vreteno32::isa::{
        add, addi, andi, bgeu, blt, bne, csrrs, halt, lui, lw, srli, sub, sw,
        CSR_MCYCLE, UART_BASE,
    };
    let mut a = vreteno32::program::Asm::default();
    a.emit(lui(1, UART_BASE >> 12)); // x1: the serial port, for say
    li(&mut a, 5, 0x4300_0000);
    a.emit(addi(20, 0, 0)); // x20: the longest
    li(&mut a, 21, 2000);
    let each = a.label();
    let shorter = a.label();
    a.place(each);
    a.emit(csrrs(6, CSR_MCYCLE, 0));
    a.emit(lw(8, 5, 0));
    a.emit(add(9, 8, 0)); // the word, used
    a.emit(csrrs(7, CSR_MCYCLE, 0));
    a.emit(sub(7, 7, 6));
    a.to(shorter, |off| bgeu(20, 7, off));
    a.emit(addi(20, 7, 0));
    a.place(shorter);
    a.emit(addi(5, 5, 64));
    a.emit(addi(21, 21, -1));
    a.to(each, |off| bne(21, 0, off));
    // The longest, in eight hex digits and a newline.
    for k in (0..8).rev() {
        a.emit(srli(22, 20, 4 * k));
        a.emit(andi(22, 22, 15));
        a.emit(addi(23, 0, 10));
        let digit = a.label();
        a.emit(addi(3, 22, b'0' as i32));
        a.to(digit, |off| blt(22, 23, off));
        a.emit(addi(3, 22, b'a' as i32 - 10));
        a.place(digit);
        let wait = a.label();
        a.place(wait);
        a.emit(lw(2, 1, 0));
        a.to(wait, |off| blt(2, 0, off));
        a.emit(sw(3, 1, 0));
    }
    say(&mut a, b"\n");
    a.emit(halt());
    a.words()
}

/// What the scanout's bursts cost the core (issue 1209): the longest a
/// single load from the DDR3 waits, with no scanout and with the
/// scanout fetching lines of the board's length. A burst holds the DDR3
/// path for its beats, so a load behind one waits for it: 88 cycles
/// with no scanout, and beside it 88 with bursts of 16, 94 with the
/// flagship's 64 and 153 with 128.
#[test]
fn a_core_load_waits_behind_the_scanout() {
    let text = load_wait_program();
    let longest = |scan: Option<Scan>| -> u32 {
        let net = Net {
            scan,
            ..Net::default()
        };
        let ran = run_all(&text, &[], b"", &[], 40 * SCAN_LINE, net, &[]);
        let line = ran.said.lines().next().unwrap_or("");
        u32::from_str_radix(line, 16)
            .unwrap_or_else(|_| panic!("said {:?}", ran.said))
    };
    let alone = longest(None);
    let beside = longest(Some(Scan {
        base: 0x4200_0000,
        show_at: SCAN_LINE,
    }));
    eprintln!(
        "a core load waits at most {alone} cycles alone, {beside} beside \
         the scanout"
    );
    assert!(beside < alone + 32, "a load waited {beside} cycles");
}

/// EGL's machine on the board (issue 996): `eglboard.rs` writes a
/// rectangle into Razboj's display list through the board's machine at
/// the second buffer's rows, has Razboj draw it, points the scanout
/// there and waits for the blanking, as a swap does; and the pixels,
/// the scanout's base and its show bit read back as a swap leaves them.
#[test]
fn the_board_machine_draws_shows_and_waits_for_the_blanking() {
    let net = Net {
        video: true,
        ..Net::default()
    };
    let ran = run_all(
        eglboard_program::TEXT,
        eglboard_program::DATA,
        b"",
        &[],
        200_000,
        net,
        &[],
    );
    assert!(ran.halted_at.is_some(), "and halted: {}", ran.said);
    assert_eq!(ran.said, "inside ff005aa5\nbase 42200000\negl board ok\n");
}

/// The instruction cache timed by the core (issue 1320): the board
/// program that gives the board's figures, run here on the simulated
/// board, so that it is known to say four lines, and what they are in
/// simulation.
///
/// Each run retires the same 3004 instructions: the loop's 3000, its
/// count, the `fence.i` or the nop in its place, and a counter read. A
/// cold run takes the 1024 cycles the tags take to clear, and its
/// misses, more than the warm run after it: 8077 against 7036 from the
/// data memory, 8149 against 7084 from the DDR3. Warm, the loop runs at
/// 2.34 cycles an instruction from either memory, below `icache_test`'s
/// figure, since the routine starts its loop at the start of a line and
/// `icache_test`'s loop starts a word in.
#[test]
fn the_cache_loop_is_timed_by_the_core() {
    let ran = run(cpi_program::TEXT, cpi_program::DATA, b"", 200_000);
    assert!(ran.halted_at.is_some(), "and halted: {}", ran.said);
    let of = |what: &str| -> (u32, u32) {
        let line = ran
            .said
            .lines()
            .find_map(|l| l.strip_prefix(&format!("cpi {what} ")))
            .unwrap_or_else(|| panic!("no line for {what}: {}", ran.said));
        let mut n = line.split(' ').map(|n| n.parse().expect("a number"));
        (n.next().expect("cycles"), n.next().expect("instret"))
    };
    let (dc, dw, rc, rw) = (
        of("dmem cold"),
        of("dmem warm"),
        of("ddr3 cold"),
        of("ddr3 warm"),
    );
    for (what, (_, i)) in [
        ("dmem cold", dc),
        ("dmem warm", dw),
        ("ddr3 cold", rc),
        ("ddr3 warm", rw),
    ] {
        assert_eq!(i, 3004, "{what}: the instructions retired");
    }
    for (what, cold, warm) in [("dmem", dc, dw), ("ddr3", rc, rw)] {
        assert!(
            cold.0 >= warm.0 + 1024,
            "{what}: cold {} takes the tags' clear over warm {}",
            cold.0,
            warm.0
        );
        assert!(
            warm.0 < 3 * warm.1,
            "{what}: warm, {} cycles for {} instructions",
            warm.0,
            warm.1
        );
    }
}
