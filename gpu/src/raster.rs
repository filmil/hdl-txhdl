// SPDX-License-Identifier: Apache-2.0
//! The rasteriser: one pixel a cycle, written into memory over AXI.
//!
//! It walks the box a display list entry gives it, one pixel per
//! cycle, and writes the entry's colour at every pixel that is in the
//! primitive. For a box that is every pixel; for a triangle it is
//! every pixel at which none of the three edge functions is negative.
//!
//! An edge function is linear, so stepping to the pixel on the right
//! adds a constant and stepping to the start of the next row adds
//! another. The rasteriser therefore keeps, per edge, the value at
//! the current pixel, the value at the start of the current row, and
//! the two steps; the only multiplications are the six of the setup,
//! done in the cycle the entry is taken. This is what makes the
//! per-pixel work three adds and three sign tests.
//!
//! It is an AXI host, and it writes as a host client writes: a burst
//! of one beat per pixel, issued on `issue` with its beat on `wbeat`
//! in the same cycle, with the identifier the tracker granted handed
//! back on `release` when the write response arrives. Several writes
//! are in flight, as many as the tracker has identifiers.
//!
//! The framebuffer is at address zero and a pixel is one word, so a
//! pixel's address is `((y << LOGW) + x) * 4`.
use txhdl::comp::{mux, Clock, DefaultClock, Out, Reg, Rx, Tx, Unit, Wire};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::{BurstKind, Done, Grant, Issue, W};

use crate::op::{Insn, Kind};

/// A pixel is one word, so a pixel's byte address is its index in the
/// framebuffer shifted by this.
const WORD: usize = 2;

// begin{state}
/// The rasteriser. `A` is the address width, `I` the AXI identifier
/// width, and the screen is `1 << LOGW` by `H` pixels.
#[derive(Trace, Default)]
pub struct Raster<
    const A: usize,
    const I: usize,
    const LOGW: usize,
    const H: usize,
> {
    /// Walking a primitive.
    pub busy: Reg<U<1>>,
    /// Which entry is being walked. A triangle is the one whose edge
    /// functions are tested, and the waveform names it.
    pub kind: Reg<Kind>,
    pub colour: Reg<U<24>>,
    /// Where the walk is, and the box it walks: the first column, the
    /// last column and the last row.
    pub x: Reg<U<16>>,
    pub y: Reg<U<16>>,
    pub xa: Reg<U<16>>,
    pub xb: Reg<U<16>>,
    pub yb: Reg<U<16>>,
    /// Each edge at this pixel, at the start of this row, and its two
    /// steps. Two's complement in thirty-two bits, which is wide
    /// enough for every product of two differences of vertices; the
    /// sign is the top bit.
    pub e0: Reg<U<32>>,
    pub e1: Reg<U<32>>,
    pub e2: Reg<U<32>>,
    pub r0: Reg<U<32>>,
    pub r1: Reg<U<32>>,
    pub r2: Reg<U<32>>,
    pub d0x: Reg<U<32>>,
    pub d1x: Reg<U<32>>,
    pub d2x: Reg<U<32>>,
    pub d0y: Reg<U<32>>,
    pub d1y: Reg<U<32>>,
    pub d2y: Reg<U<32>>,
    /// Writes issued whose response has not come back.
    pub inflight: Reg<U<4>>,
    /// Whether this pixel is in the primitive. A wire and not a
    /// `let`, so that it has a name in the trace and in the netlist:
    /// it is the one thing a waveform of a triangle wants to show.
    pub hit: Wire<Bit>,
}
// end{state}

// begin{run}
#[lower]
impl<const A: usize, const I: usize, const LOGW: usize, const H: usize> Unit
    for Raster<A, I, LOGW, H>
{
    async fn run(
        &mut self,
        (ops, grant, done): (Rx<Insn>, Rx<Grant<I>>, Rx<Done<I>>),
        (issue, wbeat, release, idle): (
            Tx<Issue<A>>,
            Tx<W<32, 4>>,
            Tx<Grant<I>>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // The writes already out. An identifier is handed back as
            // its response arrives; the identifier a burst was granted
            // is of no use here, but the channel must not fill, so it
            // is taken and dropped.
            let dh = done.head();
            let dgo = done.peek().is_some() & release.ready();
            let _ = done.recv_if(release.ready());
            let _ = grant.recv_if(grant.peek().is_some());

            // Where the walk is, and whether this pixel is in the
            // primitive: every pixel of a box, and of a triangle the
            // pixels at which no edge function is negative.
            // The locals may not take a register's name: two
            // declarations of one name is what the lowering would
            // write. The pixel is `px`, `py`; the steps taken from a
            // new entry are `t0x` and its like.
            let px = self.x.get();
            let py = self.y.get();
            let going = self.busy.get() == 1;
            let n0 = !self.e0.get().bit(31);
            let n1 = !self.e1.get().bit(31);
            let n2 = !self.e2.get().bit(31);
            // The local may not be called `hit`: that is the wire's
            // name, and one name declared twice is what the lowering
            // would write.
            let covered = (self.kind.get() != Kind::Tri) | (n0 & n1 & n2);
            self.hit.set(covered);
            // A pixel is written when there is room for the burst and
            // for its beat; the walk steps when the pixel wanted no
            // write, or when its write went out.
            let room = issue.ready() & wbeat.ready();
            let write = going & covered & room;
            let step = going & (!covered | room);
            let eol = px == self.xb.get();
            let eof = eol & (py == self.yb.get());

            // The next entry, taken once the walk has finished, and
            // the setup it asks for: per edge, the two steps and the
            // value at the box's first pixel.
            let op = ops.head();
            let more = ops.peek().is_some();
            let start = !going & more;
            let _ = ops.recv_if(!going);
            // A vertex is two's complement and is widened by its
            // sign; the box's first pixel is a screen coordinate and
            // is widened by zero.
            let ax = op.ax.sext::<32>();
            let ay = op.ay.sext::<32>();
            let bx = op.bx.sext::<32>();
            let by = op.by.sext::<32>();
            let cx = op.cx.sext::<32>();
            let cy = op.cy.sext::<32>();
            // The box. A clear says only its colour, so its box is
            // the screen, which the rasteriser knows from its own
            // type; a rectangle and a triangle carry theirs.
            let clearing = op.kind == Kind::Clear;
            let zero16 = U::<16>::from(0u8);
            let last_x = U::<16>::from(((1usize << LOGW) - 1) as u32);
            let last_y = U::<16>::from((H - 1) as u32);
            let wx = mux(clearing, zero16, op.x0.resize::<16>());
            let wy = mux(clearing, zero16, op.y0.resize::<16>());
            let bx1 = mux(clearing, last_x, op.x1.resize::<16>());
            let by1 = mux(clearing, last_y, op.y1.resize::<16>());
            // The edge functions are set up at the box's first pixel,
            // and only a triangle ever reads them.
            let sx = wx.resize::<32>();
            let sy = wy.resize::<32>();
            let zero = U::<32>::from(0u8);
            let t0x = zero - (by - ay);
            let t0y = bx - ax;
            let s0 =
                (bx - ax).mul::<32>(sy - ay) - (by - ay).mul::<32>(sx - ax);
            let t1x = zero - (cy - by);
            let t1y = cx - bx;
            let s1 =
                (cx - bx).mul::<32>(sy - by) - (cy - by).mul::<32>(sx - bx);
            let t2x = zero - (ay - cy);
            let t2y = ax - cx;
            let s2 =
                (ax - cx).mul::<32>(sy - cy) - (ay - cy).mul::<32>(sx - cx);
            // The next row's values, wanted in two places below.
            let q0 = self.r0.get() + self.d0y.get();
            let q1 = self.r1.get() + self.d1y.get();
            let q2 = self.r2.get() + self.d2y.get();

            with!(self <= {
                start ? {
                    busy: U::<1>::from(1u8),
                    kind: op.kind,
                    colour: op.colour,
                    x: wx,
                    y: wy,
                    xa: wx,
                    xb: bx1,
                    yb: by1,
                    e0: s0, r0: s0, d0x: t0x, d0y: t0y,
                    e1: s1, r1: s1, d1x: t1x, d1y: t1y,
                    e2: s2, r2: s2, d2x: t2x, d2y: t2y,
                },
                step & eof ? busy: U::<1>::from(0u8),
                step & !eol ? {
                    x: px + 1,
                    e0: self.e0.get() + self.d0x.get(),
                    e1: self.e1.get() + self.d1x.get(),
                    e2: self.e2.get() + self.d2x.get(),
                },
                step & eol & !eof ? {
                    x: self.xa.get(),
                    y: py + 1,
                    e0: q0, r0: q0,
                    e1: q1, r1: q1,
                    e2: q2, r2: q2,
                },
                inflight: self.inflight.get() + write.zext::<4>()
                    - dgo.zext::<4>(),
            });

            // One pixel, as a burst of one beat at the pixel's word.
            let addr = ((py << LOGW) + px) << WORD;
            if write.to_bool() {
                issue.send(Issue {
                    read: Bit::Zero,
                    addr: addr.resize::<A>(),
                    len: U::<8>::from(0u8),
                    size: U::<3>::from(2u8),
                    burst: BurstKind::Incr,
                    lock: Bit::Zero,
                    cache: U::<4>::from(0u8),
                    prot: U::<3>::from(0u8),
                    qos: U::<4>::from(0u8),
                    region: U::<4>::from(0u8),
                });
                wbeat.send(W {
                    data: self.colour.get().resize::<32>(),
                    strb: U::<4>::from(15u8),
                    last: Bit::One,
                });
            }
            if dgo.to_bool() {
                release.send(Grant { id: dh.id });
            }
            // Nothing left to draw and nothing left in flight.
            idle.set(!going & !more & (self.inflight.get() == 0));
        }
    }
}
// end{run}
