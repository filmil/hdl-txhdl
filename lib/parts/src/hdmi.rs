// SPDX-License-Identifier: Apache-2.0
//! HDMI through an encoder chip: a video peripheral on AXI-Lite, and
//! the I2C master that configures the chip.
//!
//! The Alinx AX7A200B drives its HDMI connector through a SiI9134, which
//! takes pixels in parallel and does the TMDS encoding and the
//! serialising itself. What the FPGA gives it is a pixel clock, 24 bits
//! of colour, the horizontal and vertical syncs and a data enable, and
//! the chip is set up once over I2C before it shows anything.
//!
//! [`Hdmi`] is the video side, one unit on the pixel clock. It counts
//! the raster, reads its framebuffer under the beam, and drives the
//! chip's parallel inputs from registers. The framebuffer is written
//! over AXI-Lite, so a host paints the picture a pixel at a time and
//! the unit shows it for as long as it runs. Every number of the video
//! mode is a parameter, so a test can run a raster of a few hundred
//! pixels where the board runs one of 420 000.
//!
//! [`I2cInit`] holds the chip in reset, then writes its configuration
//! registers over I2C, and says whether every byte was acknowledged.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};

/// Words in the framebuffer: an address is the row in its top seven
/// bits and the column in its low eight, so the picture is at most
/// 256 columns by 128 rows.
pub const FB_WORDS: usize = 1 << 15;

// begin{modes}
/// 640 by 480 at 60 Hz, as VESA states it: 800 columns and 525 rows
/// per frame at 25.175 MHz, both syncs low while active.
pub mod vga {
    /// Visible columns.
    pub const HV: usize = 640;
    /// Columns of front porch.
    pub const HFP: usize = 16;
    /// Columns of horizontal sync.
    pub const HSW: usize = 96;
    /// Columns of back porch.
    pub const HBP: usize = 48;
    /// Visible rows.
    pub const VV: usize = 480;
    /// Rows of front porch.
    pub const VFP: usize = 10;
    /// Rows of vertical sync.
    pub const VSW: usize = 2;
    /// Rows of back porch.
    pub const VBP: usize = 33;
}
// end{modes}

// begin{fns}
/// The last count of an axis, blanking included: the visible part, the
/// front porch, the sync pulse and the back porch, less one.
///
/// One function serves both axes. A call binds a function's const
/// parameters by position, so the names here are the axis's rather
/// than either axis's own, which is issue 127; before that they had to
/// be the unit's names, and each axis had a function of its own.
#[lower]
fn axis_end<
    const V: usize,
    const FP: usize,
    const SW: usize,
    const BP: usize,
>(
    c: U<12>,
) -> bool {
    c == V + FP + SW + BP - 1
}

/// Whether a count is inside its axis's sync pulse.
#[lower]
fn in_sync<const V: usize, const FP: usize, const SW: usize>(c: U<12>) -> bool {
    (c >= V + FP) & (c < V + FP + SW)
}

/// Whether a count is in the visible part of its axis.
#[lower]
fn visible<const V: usize>(c: U<12>) -> bool {
    c < V
}

/// The last column of the framebuffer: the visible columns, divided by
/// the size of a framebuffer pixel on the screen.
#[lower]
fn last_col<const HV: usize, const SHIFT: usize>(x: U<8>) -> bool {
    x == (HV >> SHIFT) - 1
}

/// The last row of the framebuffer.
#[lower]
fn last_row<const VV: usize, const SHIFT: usize>(y: U<7>) -> bool {
    y == (VV >> SHIFT) - 1
}

/// A 4-bit colour as 8 bits, its bits repeated, so 15 is full scale.
#[lower]
fn wide(c: U<4>) -> U<8> {
    c.concat::<4, 8>(c)
}
// end{fns}

/// The video peripheral. A framebuffer pixel is 12 bits, four each of
/// red, green and blue from the top, and covers `1 << SHIFT` by
/// `1 << SHIFT` pixels of the screen.
///
/// Its AXI-Lite words:
///
/// * Word 0, status, read: bit 0 high in vertical blanking, bits 31 to
///   16 the count of frames shown.
/// * Word 1, cursor, read and write: the column in bits 7 to 0 and the
///   row in bits 14 to 8.
/// * Word 2, pixel, write: the colour in bits 11 to 0 goes into the
///   framebuffer at the cursor, and the cursor moves to the next
///   column, and from the last column to the first of the next row, and
///   from the last row to the first. A fixed burst to this word paints
///   a run of pixels.
///
/// The chip's inputs are registers: the pixel read at the edge from the
/// framebuffer, and the syncs and the enable delayed to meet it.
// begin{state}
#[derive(Trace, Default)]
pub struct Hdmi<
    const HV: usize,
    const HFP: usize,
    const HSW: usize,
    const HBP: usize,
    const VV: usize,
    const VFP: usize,
    const VSW: usize,
    const VBP: usize,
    const SHIFT: usize,
> {
    /// The framebuffer.
    pub fb: Mem<U<12>, FB_WORDS>,
    /// The column of the raster.
    pub hc: Reg<U<12>>,
    /// The row of the raster.
    pub vc: Reg<U<12>>,
    /// The pixel under the beam, read at the edge.
    pub px: Reg<U<12>>,
    /// The horizontal sync, low while active, a cycle late to meet the
    /// pixel.
    pub hs_q: Reg<Bit>,
    /// The vertical sync, the same way.
    pub vs_q: Reg<Bit>,
    /// The data enable, high over the visible part, the same way.
    pub de_q: Reg<Bit>,
    /// The column a pixel written goes to.
    pub cx: Reg<U<8>>,
    /// The row a pixel written goes to.
    pub cy: Reg<U<7>>,
    /// Frames shown.
    pub frames: Reg<U<16>>,
}
// end{state}

// begin{run}
#[lower]
impl<
        const HV: usize,
        const HFP: usize,
        const HSW: usize,
        const HBP: usize,
        const VV: usize,
        const VFP: usize,
        const VSW: usize,
        const VBP: usize,
        const SHIFT: usize,
    > Unit for Hdmi<HV, HFP, HSW, HBP, VV, VFP, VSW, VBP, SHIFT>
{
    async fn run(
        &mut self,
        (aw, ar, w): (Rx<LiteAw<32>>, Rx<LiteAr<32>>, Rx<LiteW<32, 4>>),
        (b, r, rgb, hsync, vsync, de): (
            Tx<LiteB>,
            Tx<LiteR<32>>,
            Out<U<24>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            // The raster.
            let hc = self.hc.get();
            let vc = self.vc.get();
            let h_last = axis_end::<HV, HFP, HSW, HBP>(hc);
            let v_last = axis_end::<VV, VFP, VSW, VBP>(vc);
            let shown = visible::<HV>(hc) & visible::<VV>(vc);
            let hs_on = in_sync::<HV, HFP, HSW>(hc);
            let vs_on = in_sync::<VV, VFP, VSW>(vc);
            // The framebuffer's pixel under the beam.
            let fx = (hc >> SHIFT).slice::<0, 8>();
            let fy = (vc >> SHIFT).slice::<0, 7>();
            let beam = fy.concat::<_, 15>(fx);
            // The pixel read at the last edge, for the chip, its three
            // colours each a wire of its own.
            let shade = self.px.get();
            let red = shade.slice::<8, 4>();
            let green = shade.slice::<4, 4>();
            let blue = shade.slice::<0, 4>();
            // The bus.
            let arh = ar.head();
            let awh = aw.head();
            let wh = w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            let rgo = r.ready() & ar.peek().is_some();
            let _ = ar.recv_if(r.ready());
            let wgo = b.ready() & aw.peek().is_some() & w.peek().is_some();
            let _ = aw.recv_if(wgo);
            let _ = w.recv_if(wgo);
            let written = wh.data;
            let cx = self.cx.get();
            let cy = self.cy.get();
            let cursor = cy.concat::<_, 15>(cx);
            let put = wgo & (wsel == 2);
            let place = wgo & (wsel == 1);
            let row_done = last_col::<HV, SHIFT>(cx);
            let rows_done = last_row::<VV, SHIFT>(cy);
            let blank = Bit::from(!visible::<VV>(vc));
            let status = self
                .frames
                .get()
                .concat::<_, 31>(U::<15>::from(0u8))
                .concat::<_, 32>(blank.zext::<1>());
            let where_at = U::<17>::from(0u8).concat::<_, 32>(cursor);
            let word = select!(rsel.raw() => {
                0 => status,
                1 => where_at,
                _ => U::<32>::from(0u8),
            });
            with!(self <= {
                hc: mux(h_last, U::<12>::from(0u8), hc + 1),
                h_last ? vc: mux(v_last, U::<12>::from(0u8), vc + 1),
                h_last & v_last ? frames: self.frames.get() + 1,
                px: self.fb.read(beam),
                hs_q: Bit::from(!hs_on),
                vs_q: Bit::from(!vs_on),
                de_q: Bit::from(shown),
                put ? fb.at(cursor): written.slice::<0, 12>(),
                put ? {
                    cx: mux(row_done, U::<8>::from(0u8), cx + 1),
                    cy: mux(
                        row_done,
                        mux(rows_done, U::<7>::from(0u8), cy + 1),
                        cy,
                    ),
                },
                place ? {
                    cx: written.slice::<0, 8>(),
                    cy: written.slice::<8, 7>(),
                },
            });
            if rgo.to_bool() {
                r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                b.send(LiteB { resp: Resp::Okay });
            }
            rgb.set(
                wide(red)
                    .concat::<8, 16>(wide(green))
                    .concat::<8, 24>(wide(blue)),
            );
            hsync.set(self.hs_q.get());
            vsync.set(self.vs_q.get());
            de.set(self.de_q.get());
        }
    }
}
// end{run}

// begin{table}
/// The SiI9134's configuration, one register write per entry: the
/// chip's I2C address as eight bits, write bit included, the register,
/// and the value. No document this part was written from states the
/// chip's registers, so the values are to be confirmed against the
/// chip's documentation or the board vendor's example before the board
/// is expected to show a picture.
pub const SII9134_WRITES: [(u8, u8, u8); 2] = [
    // System control: out of power down, with the input bus as the
    // board wires it.
    (0x72, 0x08, 0x35),
    // The output as DVI: no HDMI packets, only the picture.
    (0x7a, 0x2f, 0x00),
];

/// The address of entry `e`.
#[lower]
fn table_dev(e: U<3>) -> U<8> {
    select!(e.raw() => {
        0 => U::<8>::from(0x72u8),
        _ => U::<8>::from(0x7au8),
    })
}

/// The register of entry `e`.
#[lower]
fn table_reg(e: U<3>) -> U<8> {
    select!(e.raw() => {
        0 => U::<8>::from(0x08u8),
        _ => U::<8>::from(0x2fu8),
    })
}

/// The value of entry `e`.
#[lower]
fn table_val(e: U<3>) -> U<8> {
    select!(e.raw() => {
        0 => U::<8>::from(0x35u8),
        _ => U::<8>::from(0x00u8),
    })
}
// end{table}

/// Whether the last cycle of a quarter of an I2C bit has come.
#[lower]
fn quarter_end<const DIV: usize>(tick: U<16>) -> bool {
    tick == DIV - 1
}

/// Whether the reset or the settling time is over.
#[lower]
fn hold_end<const HOLD: usize>(timer: U<24>) -> bool {
    timer == HOLD - 1
}

/// The I2C master that configures the chip.
///
/// It holds the chip's reset low for `HOLD` cycles, releases it, waits
/// `HOLD` cycles more, and then writes each entry of the table as one
/// I2C transaction: a start, the address, the register, the value, and
/// a stop, with the chip's acknowledge after each byte. A bit is four
/// quarters of `DIV` cycles each; the clock is low in the first and the
/// last, and the data changes only while it is low. The lines are open
/// drain: `scl_low` and `sda_low` high pull a line low, and a line not
/// pulled is high through the board's pull-up. A byte the chip does not
/// acknowledge sets `failed`, and `done` rises when the table is
/// written.
// begin{i2cstate}
#[derive(Trace, Default)]
pub struct I2cInit<const DIV: usize, const HOLD: usize> {
    /// Reset held 0, settling 1, writing 2, done 3.
    pub phase: Reg<U<2>>,
    /// Cycles into the reset or the settling.
    pub timer: Reg<U<24>>,
    /// Cycles into the quarter.
    pub tick: Reg<U<16>>,
    /// The quarter of the bit.
    pub quarter: Reg<U<2>>,
    /// The step of the transaction: the start 0, the address's bits 1
    /// to 8 and its acknowledge 9, the register's 10 to 17 and 18, the
    /// value's 19 to 26 and 27, and the stop 28.
    pub step: Reg<U<5>>,
    /// The entry being written.
    pub entry: Reg<U<3>>,
    /// The byte going out, its next bit on top.
    pub shift: Reg<U<8>>,
    /// A byte was not acknowledged.
    pub nak: Reg<Bit>,
}
// end{i2cstate}

// begin{i2c}
#[lower]
impl<const DIV: usize, const HOLD: usize> Unit for I2cInit<DIV, HOLD> {
    async fn run(
        &mut self,
        sda_in: In<Bit>,
        (nreset, scl_low, sda_low, done, failed): (
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let phase = self.phase.get();
            let timer = self.timer.get();
            let tick = self.tick.get();
            let q = self.quarter.get();
            let step = self.step.get();
            let entry = self.entry.get();
            let shift = self.shift.get();
            let writing = phase == 2;
            let timed = hold_end::<HOLD>(timer);
            let q_last = quarter_end::<DIV>(tick);
            let q_go = writing & q_last;
            let step_go = q_go & (q == 3);
            // Where in the transaction the step is.
            let starting = step == 0;
            let stopping = step == 28;
            let acking = (step == 9) | (step == 18) | (step == 27);
            // The lines as the step and the quarter want them, high
            // meaning released.
            let clock_in_bit = (q == 1) | (q == 2);
            let scl = mux(starting, q < 2, mux(stopping, q != 0, clock_in_bit));
            let data_bit = shift.bit(7).to_bool();
            let sda = mux(
                starting,
                q == 0,
                mux(stopping, q >= 2, mux(acking, true, data_bit)),
            );
            let last_entry = entry == 1;
            // The next byte, loaded as its first bit begins.
            let next_byte = select!(step.raw() => {
                0 => table_dev(entry),
                9 => table_reg(entry),
                _ => table_val(entry),
            });
            let loading = (step == 0) | (step == 9) | (step == 18);
            with!(self <= {
                (phase != 2) & (phase != 3) ? timer: timer + 1,
                (phase == 0) & timed ? {
                    phase: U::<2>::from(1u8),
                    timer: U::<24>::from(0u8),
                },
                (phase == 1) & timed ? {
                    phase: U::<2>::from(2u8),
                    tick: U::<16>::from(0u8),
                    quarter: U::<2>::from(0u8),
                    step: U::<5>::from(0u8),
                    entry: U::<3>::from(0u8),
                },
                writing ? tick: mux(q_last, U::<16>::from(0u8), tick + 1),
                q_go ? quarter: q + 1,
                q_go & (q == 2) & acking ?
                    nak: self.nak.get() | sda_in.get(),
                step_go ? step: step + 1,
                step_go & !acking & !starting & !stopping ? shift: shift << 1,
                step_go & loading ? shift: next_byte,
                step_go & stopping ? {
                    step: U::<5>::from(0u8),
                    entry: entry + 1,
                },
                step_go & stopping & last_entry ? phase: U::<2>::from(3u8),
            });
            nreset.set(Bit::from(phase != 0));
            scl_low.set(Bit::from(writing & !scl));
            sda_low.set(Bit::from(writing & !sda));
            done.set(Bit::from(phase == 3));
            failed.set(self.nak.get());
        }
    }
}
// end{i2c}

/// An I2C device that acknowledges every byte and records what it was
/// sent, for the tests and the example: it watches the two lines as the
/// board resolves them and pulls the data line low for each
/// acknowledge.
#[derive(Default)]
pub struct I2cDevice {
    scl: bool,
    sda: bool,
    in_transaction: bool,
    bits: u32,
    byte: u8,
    bytes: Vec<u8>,
    acking: bool,
    /// Each transaction's bytes, in the order they came.
    pub transactions: Vec<Vec<u8>>,
}

impl I2cDevice {
    /// A device on idle lines.
    pub fn new() -> Self {
        I2cDevice {
            scl: true,
            sda: true,
            ..Default::default()
        }
    }

    /// Whether the device pulls the data line low now.
    pub fn pulls_sda(&self) -> bool {
        self.acking
    }

    /// One cycle of the lines: `scl_low` and `sda_low` are the master's
    /// pulls, and the data line is also low while the device pulls it.
    pub fn step(&mut self, scl_low: bool, sda_low: bool) {
        let scl = !scl_low;
        let sda = !sda_low && !self.acking;
        if self.scl && scl {
            if self.sda && !sda {
                // A start.
                self.in_transaction = true;
                self.bits = 0;
                self.bytes.clear();
            } else if !self.sda && sda && self.in_transaction {
                // A stop.
                self.in_transaction = false;
                self.transactions.push(std::mem::take(&mut self.bytes));
            }
        }
        if self.in_transaction && !self.scl && scl {
            // A rising clock: a data bit, or the acknowledge's clock.
            if self.bits < 8 {
                self.byte = (self.byte << 1) | sda as u8;
            }
            self.bits += 1;
        }
        if self.in_transaction && self.scl && !scl {
            // A falling clock: after the eighth bit the device pulls the
            // line for the acknowledge, and after the ninth lets it go.
            if self.bits == 8 {
                self.acking = true;
                self.bytes.push(self.byte);
            } else if self.bits == 9 {
                self.acking = false;
                self.bits = 0;
            }
        }
        self.scl = scl;
        self.sda = sda;
    }
}

/// The peripheral and the master against models: the raster's syncs
/// and enable where the mode puts them, the pixels written where the
/// beam shows them, and the chip's table on the I2C lines as a device
/// decodes it.
#[cfg(test)]
mod tests {
    use super::*;
    use txhdl::comp::{join2, signal, Running};

    /// A tiny mode: 8 by 6 visible, 2 of front porch, 3 of sync and 3
    /// of back porch across, 1, 2 and 2 down; each framebuffer pixel is
    /// 2 by 2 on the screen, so the framebuffer is 4 by 3.
    type Tiny = Hdmi<8, 2, 3, 3, 6, 1, 2, 2, 1>;

    #[test]
    fn the_raster_is_where_the_mode_puts_it() {
        let lite = crate::bus::axi_lite::axi_lite::<32, 32, 4>();
        let (paw, par, pw, pb, pr) = lite.per;
        let (rgb_out, _rgb) = signal::<U<24>, DefaultClock>();
        let (hs_out, hs) = signal::<Bit, DefaultClock>();
        let (vs_out, vs) = signal::<Bit, DefaultClock>();
        let (de_out, de) = signal::<Bit, DefaultClock>();
        let mut unit = Tiny::default();
        let mut sim = Running::new(
            unit.run((paw, par, pw), (pb, pr, rgb_out, hs_out, vs_out, de_out)),
        );
        let (width, height) = (16usize, 11usize);
        let mut seen: Vec<(bool, bool, bool)> = Vec::new();
        for _ in 0..(2 * width * height) {
            sim.cycle();
            seen.push((
                hs.get().to_bool(),
                vs.get().to_bool(),
                de.get().to_bool(),
            ));
        }
        // The outputs trail the counters by a fixed number of cycles.
        // One lag must fit the enable and both syncs at once, which is
        // what keeps the three aligned for the chip.
        let fits = |lag: usize| {
            seen.iter().enumerate().skip(lag).all(|(t, &(h, v, d))| {
                let n = t - lag;
                let (x, y) = (n % width, (n / width) % height);
                d == (x < 8 && y < 6)
                    && h == !(10..13).contains(&x)
                    && v == !(7..9).contains(&y)
            })
        };
        let lags: Vec<usize> = (0..4).filter(|&l| fits(l)).collect();
        assert_eq!(lags.len(), 1, "one lag fits all three: {lags:?}");
    }

    #[test]
    fn the_chip_is_configured_over_i2c() {
        let (sda_out, sda_in) = signal::<Bit, DefaultClock>();
        let (rst_out, rst) = signal::<Bit, DefaultClock>();
        let (scl_out, scl) = signal::<Bit, DefaultClock>();
        let (sdal_out, sdal) = signal::<Bit, DefaultClock>();
        let (done_out, done) = signal::<Bit, DefaultClock>();
        let (fail_out, fail) = signal::<Bit, DefaultClock>();
        let mut master = I2cInit::<3, 5>::default();
        let mut sim = Running::new(join2(
            master
                .run(sda_in, (rst_out, scl_out, sdal_out, done_out, fail_out)),
            async {},
        ));
        let mut dev = I2cDevice::new();
        let mut released_at = None;
        for t in 0..2000 {
            sim.cycle();
            if released_at.is_none() && rst.get().to_bool() {
                released_at = Some(t);
            }
            dev.step(scl.get().to_bool(), sdal.get().to_bool());
            let line = !sdal.get().to_bool() && !dev.pulls_sda();
            sda_out.set(Bit::from_bool(line));
            if done.get().to_bool() {
                break;
            }
        }
        assert!(done.get().to_bool(), "the table was written");
        assert!(!fail.get().to_bool(), "every byte acknowledged");
        // Low for cycles 0 to 4, so high from cycle 5: HOLD cycles.
        assert_eq!(released_at, Some(5), "the reset held for HOLD cycles");
        let want: Vec<Vec<u8>> = SII9134_WRITES
            .iter()
            .map(|&(d, r, v)| vec![d, r, v])
            .collect();
        assert_eq!(dev.transactions, want);
    }
}
