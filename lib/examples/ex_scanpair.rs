// SPDX-License-Identifier: Apache-2.0
//! A scanout from memory, a line ahead of the beam, and a line that
//! starves when the memory stalls too long (issue 151).
//!
//! ```text
//!   Rom -- AxiPer -- AxiHost -- LineFetch -- ChanCdc --> LinePair
//!                                  ^                        |
//!                              ScanFetch <---- ChanCdc -----+
//!   <---------------- bus clock ------------>  ><  <- pixel clock ->
//! ```
//!
//! The raster here is the testbench's: a small one, eight visible
//! columns of sixty-four and four visible rows of six, on a pixel
//! clock unrelated to the bus clock. `LinePair` asks for each line at
//! the start of the line before it, `ScanFetch` takes the request
//! across and starts `LineFetch`, and the words come back across into
//! the half of the pair the beam is not reading.
//!
//! The memory takes a `hold`, which stalls its beats, standing in for
//! a refresh or a bank miss. Three frames run with a short stall in
//! every line, the size of a refresh against the board's line, and
//! every pixel comes out as the word at its place in the frame, with
//! the underflow bit never set. A fourth frame has one stall longer
//! than a line: the bit sets, and stays set until it is cleared. The
//! run is checked against the netlists of `LinePair` and `ScanFetch`
//! under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, join2, now, signal, Clock, DefaultClock, In, Mem, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{with, Trace};
use txhdl_parts::bus::axi::{axi_units, AxiHost, AxiPer, PerPort, Resp, R};
use txhdl_parts::cdc::ChanCdc;
use txhdl_parts::dma::LineFetch;
use txhdl_parts::scanout::{LinePair, ScanFetch};

const ADDR: usize = 32;
const IDB: usize = 2;
const IDS: usize = 4;
const BEATS: usize = 16;
/// The raster: visible columns of all of them, visible rows of all.
const LEN: usize = 8;
const AW: usize = 3;
const HT: usize = 64;
const ROWS: usize = 4;
const TOTAL: usize = 6;
/// Bytes from one line to the next in memory.
const STRIDE: usize = LEN * 4;
/// The frame's first word in memory.
const BASE_WORD: usize = 16;
const N: usize = 256;

/// The pixel clock, unrelated to the bus clock on purpose.
pub struct ClkPix;
impl Clock for ClkPix {
    const NAME: &'static str = "clk_pix";
    const PERIOD: u64 = 6;
    const PHASE: u64 = 2;
}

/// The memory, whose beats a `hold` stalls. A burst arrives as one
/// request carrying `len` and is owed `len + 1` beats, the last
/// marked; it never writes.
#[derive(Trace, Default)]
pub struct Rom<const A: usize, const I: usize, const M: usize> {
    pub px: Mem<U<32>, M>,
    pub busy: Reg<Bit>,
    pub at: Reg<U<16>>,
    pub left: Reg<U<9>>,
    pub rid: Reg<U<I>>,
}

/// Not lowered: it is the testbench's memory, and the units under check
/// are the scanout's two ends.
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

type Pair = LinePair<LEN, AW, ROWS, TOTAL, STRIDE, ClkPix>;
type Fetch = ScanFetch<ADDR, 16, LEN>;

fn main() {
    let link = axi_units::<ADDR, 32, 4, IDB>();
    let (issue, wbeat, release, grant, done, rdata) = link.host_client;
    let bus: PerPort<ADDR, 32, 4, IDB> = link.per_client.into();
    let (hold_o, hold) = signal::<Bit, DefaultClock>();
    let (at_o, at) = signal::<U<ADDR>, DefaultClock>();
    let (count_o, count) = signal::<U<16>, DefaultClock>();
    let (start_o, start) = signal::<Bit, DefaultClock>();
    let (run_o, running) = signal::<Bit, DefaultClock>();
    let (col_o, col) = signal::<U<AW>, ClkPix>();
    let (vis_o, vis) = signal::<Bit, ClkPix>();
    let (line_o, line) = signal::<Bit, ClkPix>();
    let (row_o, row) = signal::<U<12>, ClkPix>();
    let (frame_o, frame) = signal::<Bit, ClkPix>();
    let (base_o, base) = signal::<U<32>, ClkPix>();
    let (clear_o, clear) = signal::<Bit, ClkPix>();
    let (show_o, show) = signal::<Bit, ClkPix>();
    let (pix_o, pix) = signal::<U<32>, ClkPix>();
    let (starved_o, starved) = signal::<Bit, ClkPix>();
    let (stuck_o, stuck) = signal::<Bit, ClkPix>();
    let (stuck_at_o, stuck_at) = signal::<U<32>, ClkPix>();
    // The words, bus side and pixel side; the requests, pixel side
    // and bus side.
    let (fetched_tx, fetched_rx) = chan::<U<32>, DefaultClock>();
    let (inp_tx, inp) = chan::<U<32>, ClkPix>();
    let (req, req_rx) = chan::<U<32>, ClkPix>();
    let (lines_tx, lines) = chan::<U<32>, DefaultClock>();

    // Each word is its own address, so a pixel from the wrong place
    // says which place.
    let image: Vec<U<32>> = (0..N).map(|i| U::<32>::from(i as u32)).collect();
    let mut rom = Rom::<ADDR, IDB, N> {
        px: Mem::with(&image),
        ..Default::default()
    };
    let mut host = AxiHost::<ADDR, 32, 4, IDB, IDS>::default();
    let mut per = AxiPer::<ADDR, 32, 4, IDB>::default();
    let mut fetch = LineFetch::<ADDR, IDB, BEATS, 16>::default();
    let mut sfetch = Fetch::default();
    let mut down = ChanCdc::<U<32>, 4, 16, 5, DefaultClock, ClkPix>::default();
    let mut up = ChanCdc::<U<32>, 1, 2, 2, ClkPix, DefaultClock>::default();
    let mut pair = Pair::default();

    // The two lowered units under check, their ports under their own
    // names and each unit under its entity's name.
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.clock::<ClkPix>();
        w.add("inp", &inp);
        w.add("col", &col);
        w.add("vis", &vis);
        w.add("line", &line);
        w.add("row", &row);
        w.add("frame", &frame);
        w.add("base", &base);
        w.add("clear", &clear);
        w.add("show", &show);
        w.add("pix", &pix);
        w.add("req", &req);
        w.add("starved", &starved);
        w.add("stuck", &stuck);
        w.add("stuck_at", &stuck_at);
        w.add("linepair", &pair);
        w.add("lines", &lines);
        w.add("running", &running);
        w.add("at", &at);
        w.add("count", &count);
        w.add("start", &start);
        w.add("scanfetch", &sfetch);
        w.start();
    }

    hold_o.set(Bit::Zero);
    base_o.set(U::<32>::from((BASE_WORD * 4) as u32));
    clear_o.set(Bit::Zero);
    // Shown from the start: the pair asks for nothing until it is.
    show_o.set(Bit::One);

    // `LineFetch` before `ScanFetch`, so that `running` is this
    // step's when `ScanFetch` reads it, as the netlist has it.
    let mut sim = Running::new(join2(
        join2(
            join2(
                host.run(link.host_in, link.host_out),
                per.run(link.per_in, link.per_out),
            ),
            rom.run((bus, hold), ()),
        ),
        join2(
            join2(
                fetch.run(
                    (grant, done, rdata, at, count, start),
                    (issue, release, fetched_tx, run_o),
                ),
                sfetch.run((lines, running), (at_o, count_o, start_o)),
            ),
            join2(
                join2(down.run(fetched_rx, inp_tx), up.run(req_rx, lines_tx)),
                pair.run(
                    (inp, col, vis, line, row, frame, base, clear, show),
                    (pix_o, req, starved_o, stuck_o, stuck_at_o),
                ),
            ),
        ),
    ));
    let _ = wbeat;

    // The raster, a pixel-clock edge at a time. Each edge's inputs are
    // set before it; the pixel out after it is the one the column
    // before named.
    let mut x = 0usize;
    let mut y = 0usize;
    let mut frames = 0usize;
    let mut asked: Option<(bool, usize, usize)> = None;
    let mut wrong = 0usize;
    let mut checked = 0usize;
    let mut long_stall_at: Option<u64> = None;
    let mut under_seen = Vec::new();
    while frames < 5 {
        let t = now();
        if ClkPix::rising_at(t) {
            let visible = x < LEN && y < ROWS;
            col_o.set(U::<AW>::from(if visible { x as u32 } else { 0 }));
            vis_o.set(Bit::from_bool(visible));
            line_o.set(Bit::from_bool(x == 0));
            row_o.set(U::<12>::from(y as u32));
            frame_o.set(Bit::from_bool(x == 0 && y == ROWS));
            // Clear the bit at the start of the last frame.
            clear_o.set(Bit::from_bool(frames == 4 && x == 0 && y == 0));
            // A short stall in every line, a refresh's size against
            // this line; in the fourth frame, one longer than a line.
            let short = x == 4;
            let long = frames == 3 && y == 1;
            if long && long_stall_at.is_none() {
                long_stall_at = Some(t);
            }
            hold_o.set(Bit::from_bool(short || long));
        }
        sim.step();
        if ClkPix::rising_at(t) {
            // What the column asked at the edge before shows now.
            if let Some((v, r, c)) = asked {
                if v && (1..3).contains(&frames) {
                    let want = (BASE_WORD + r * LEN + c) as u128;
                    if pix.get().raw() != want {
                        wrong += 1;
                    }
                    checked += 1;
                }
            }
            asked = Some((x < LEN && y < ROWS, y, x));
            x += 1;
            if x == HT {
                x = 0;
                y += 1;
                if y == TOTAL {
                    y = 0;
                    under_seen.push(starved.get().to_bool());
                    frames += 1;
                }
            }
        }
    }
    stop();
    println!("pixels checked in frames 1 and 2: {checked}, wrong: {wrong}");
    println!("underflow at each frame's end:     {under_seen:?}");
    assert_eq!(wrong, 0, "every pixel is the word at its place");
    assert_eq!(checked, 2 * ROWS * LEN, "two frames of pixels checked");
    assert_eq!(
        under_seen,
        [false, false, false, true, false],
        "no underflow until the long stall, then one, then cleared"
    );
    let pair_net = Pair::lowered("linepair");
    let fetch_net = Fetch::lowered("scanfetch");
    txhdl::netlist::write_netlists_from_env(&[&pair_net, &fetch_net]);
    print!("\n{}", pair_net.verilog());
}
