// SPDX-License-Identifier: Apache-2.0
//! An SPI master on AXI-Lite, and a model of a flash chip to check it
//! against.
//!
//! Three registers. A program sets the divider, the mode and the chip
//! select in the first, writes a byte to the second to start a
//! transfer, and reads the second again to get the byte that came back
//! the other way, because SPI sends and receives in the same eight
//! clocks. The third says whether a transfer is running and whether
//! one has finished.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | the divider, the mode, the select, the enable |
//! | `0x04` | `data` | written: the byte to send; read: the byte that came |
//! | `0x08` | `state` | bit 0 running, bit 1 finished; write one to clear |
//!
//! `ctrl` is the divider in bits 0 to 7, then `cpol` in bit 8, `cpha`
//! in bit 9, the chip select in bit 10 and the interrupt enable in bit
//! 11. A half of a bit takes `div + 1` cycles, so the clock is the
//! system's over `2 * (div + 1)`.
//!
//! The chip select is a bit a program sets and clears rather than
//! something a transfer does, because every SPI command is several
//! bytes under one select: a read of this flash is a command byte,
//! three of address, a dummy, and then as many as are wanted.
//!
//! The four modes are the two bits they always are. `cpol` is the
//! level the clock idles at. `cpha` says which of the two edges of a
//! bit carries it: with `cpha` clear the master samples on the leading
//! edge and moves its data on the trailing one, and with `cpha` set it
//! does the reverse. The one place that is not symmetric is the first
//! bit, which with `cpha` set has to be on the wire before the first
//! leading edge and must not be moved by it; the run counts half
//! edges, so that is the one leading edge it does not shift on.
use txhdl::comp::{mux, Clock, DefaultClock, In, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, select, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};

// begin{state}
/// An SPI master: one byte each way at a time, under a chip select a
/// program holds.
#[derive(Trace, Default)]
pub struct Spi {
    /// A half of a bit is this many cycles, less one.
    pub div: Reg<U<8>>,
    /// The level the clock idles at.
    pub cpol: Reg<Bit>,
    /// Which edge of a bit carries it.
    pub cpha: Reg<Bit>,
    /// The chip select, high while a program holds the chip.
    pub sel: Reg<Bit>,
    /// Whether a finished transfer raises the interrupt.
    pub ie: Reg<Bit>,
    /// A transfer is running.
    pub busy: Reg<Bit>,
    /// A transfer has finished and nothing has cleared it.
    pub fired: Reg<Bit>,
    /// Cycles into the half bit.
    pub tick: Reg<U<8>>,
    /// Which half of the bit the clock is in.
    pub half: Reg<Bit>,
    /// Half edges done, sixteen to the byte.
    pub count: Reg<U<5>>,
    /// The byte going out, its next bit on top.
    pub txb: Reg<U<8>>,
    /// The byte coming in, its first bit at the bottom by the end.
    pub rxb: Reg<U<8>>,
}
// end{state}

// begin{run}
#[lower]
impl Unit for Spi {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (miso, sclk, mosi, cs_n, irq): (
            In<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let div = self.div.get();
            let cpol = self.cpol.get();
            let cpha = self.cpha.get();
            let sel = self.sel.get();
            let ie = self.ie.get();
            let busy = self.busy.get();
            let fired = self.fired.get();
            let tick = self.tick.get();
            let half = self.half.get();
            let count = self.count.get();
            let txb = self.txb.get();
            let rxb = self.rxb.get();
            // A half of a bit has passed. `strobe` is the moment the
            // clock turns over, and the two kinds of turn are the one
            // into the active level and the one back out of it.
            let strobe = busy & (tick == div);
            let leading = strobe & !half;
            let trailing = strobe & half;
            // Which edge carries the bit, and which moves the data.
            // The first leading edge is the one exception: with `cpha`
            // set the first bit is already on the wire and must stay.
            let taking = mux(cpha, trailing, leading);
            let moving = mux(cpha, leading & (count != 0), trailing);
            let last = strobe & (count == 15);
            // The bus.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data;
            // A write to `data` starts a transfer, and is ignored
            // while one is running: a program reads `state` first.
            let start = wgo & (wsel == 1) & !busy;
            // The control word, built from the top down: the enable,
            // the select, the two mode bits, then the divider in the
            // low eight.
            let ctrl = ie
                .zext::<1>()
                .concat::<1, 2>(sel.zext::<1>())
                .concat::<1, 3>(cpha.zext::<1>())
                .concat::<1, 4>(cpol.zext::<1>())
                .concat::<8, 12>(div)
                .zext::<32>();
            let state = fired
                .zext::<1>()
                .concat::<1, 2>(busy.zext::<1>())
                .zext::<32>();
            let word = select!(rsel.raw() => {
                0 => ctrl,
                1 => rxb.zext::<32>(),
                2 => state,
                _ => U::<32>::from(0u8),
            });
            let clearing = wgo & (wsel == 2) & written.bit(1).to_bool();
            with!(self <= {
                wgo & (wsel == 0) ? {
                    div: written.slice::<0, 8>(),
                    cpol: written.bit(8),
                    cpha: written.bit(9),
                    sel: written.bit(10),
                    ie: written.bit(11),
                },
                start ? {
                    busy: Bit::One,
                    txb: written.slice::<0, 8>(),
                    tick: U::<8>::from(0u8),
                    half: Bit::Zero,
                    count: U::<5>::from(0u8),
                },
                busy ? tick: mux(strobe, U::<8>::from(0u8), tick + 1),
                strobe ? {
                    half: !half,
                    count: count + 1,
                },
                taking ? rxb: (rxb << 1) | miso.get().zext::<8>(),
                moving ? txb: txb << 1,
                last ? {
                    busy: Bit::Zero,
                    half: Bit::Zero,
                    fired: Bit::One,
                },
                clearing & !last ? fired: Bit::Zero,
            });
            if rgo.to_bool() {
                bus.r.send(LiteR {
                    data: word,
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            sclk.set(cpol ^ half);
            mosi.set(txb.bit(7));
            cs_n.set(!sel);
            irq.set(fired & ie);
        }
    }
}
// end{run}

/// A flash chip on the other end of the wires, as a simulation.
///
/// It answers the two commands a bootloader needs: `0x9f`, which
/// returns the maker and the type, and `0x0b`, a fast read, which
/// takes three bytes of address and one that is thrown away and then
/// returns as many bytes as are clocked. A command runs while the
/// select is held and ends when it is released, which is what a real
/// chip does, so a program that forgets to release the select sees the
/// same wrong answer here as it would on a board.
///
/// It is a model and not a part: it holds a `Vec`, and nothing lowers
/// it. Its place is a testbench.
#[derive(Default)]
pub struct FlashDevice {
    /// What the chip holds.
    pub bytes: Vec<u8>,
    /// The maker and the type, answered to `0x9f`.
    pub id: [u8; 3],
    selected: bool,
    sclk: bool,
    bits: u32,
    in_byte: u8,
    out_byte: u8,
    taken: Vec<u8>,
    address: u32,
    cpol: bool,
    cpha: bool,
}

impl FlashDevice {
    /// A chip holding `bytes`, answering `id` to the identify command.
    pub fn new(bytes: Vec<u8>, id: [u8; 3], cpol: bool, cpha: bool) -> Self {
        FlashDevice {
            bytes,
            id,
            sclk: cpol,
            cpol,
            cpha,
            ..Default::default()
        }
    }

    /// What the chip drives on its output line now.
    pub fn miso(&self) -> bool {
        self.out_byte & 0x80 != 0
    }

    /// One cycle of the wires: the select as the master drives it, low
    /// while the chip is held, and the clock and the data line.
    pub fn step(&mut self, cs_n: bool, sclk: bool, mosi: bool) {
        let selected = !cs_n;
        if selected != self.selected {
            self.selected = selected;
            self.bits = 0;
            self.in_byte = 0;
            self.out_byte = 0;
            self.taken.clear();
            self.sclk = self.cpol;
            return;
        }
        if !selected {
            return;
        }
        let was = self.sclk;
        self.sclk = sclk;
        if was == sclk {
            return;
        }
        // The chip takes the master's bit on the same edge the master
        // takes the chip's, which is the leading one unless `cpha`
        // says otherwise.
        let leading = sclk != self.cpol;
        let taking = if self.cpha { !leading } else { leading };
        if taking {
            self.in_byte = (self.in_byte << 1) | u8::from(mosi);
            self.bits += 1;
            if self.bits == 8 {
                self.bits = 0;
                let byte = self.in_byte;
                self.in_byte = 0;
                self.taken.push(byte);
                // The next byte's top bit is on the wire from here,
                // and is not moved until the master has taken it.
                self.out_byte = self.answer();
            } else {
                // The master took the top bit on this edge, so the
                // next one goes up.
                self.out_byte <<= 1;
            }
        }
    }

    /// The byte to put out over the next eight clocks, from what the
    /// chip has been sent so far.
    fn answer(&mut self) -> u8 {
        match self.taken.first() {
            Some(0x9f) => {
                let i = self.taken.len() - 1;
                *self.id.get(i).unwrap_or(&0)
            }
            Some(0x0b) => {
                // The command, three of address, one thrown away, and
                // then the bytes.
                if self.taken.len() == 4 {
                    self.address = (u32::from(self.taken[1]) << 16)
                        | (u32::from(self.taken[2]) << 8)
                        | u32::from(self.taken[3]);
                }
                if self.taken.len() < 5 {
                    return 0;
                }
                let at = self.address as usize + (self.taken.len() - 5);
                *self.bytes.get(at).unwrap_or(&0xff)
            }
            _ => 0,
        }
    }
}
