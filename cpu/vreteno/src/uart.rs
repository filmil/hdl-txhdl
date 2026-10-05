// SPDX-License-Identifier: Apache-2.0
//! The serial port: an AXI-Lite peripheral with a line each way, its
//! registers SiFive's `sifive,uart0` (issue 1011), so that Linux's
//! `serial/sifive.c`, OpenSBI's console and Zephyr's `uart_sifive`
//! drive it as they are. It sits behind a bridge from the core's AXI4
//! link, which hands it one transaction at a time with no identifier
//! and no burst, so a read is an address in and a word out, and a
//! write is an address and a word in and a response out.
//!
//! A byte written to `txdata` joins a queue of eight; while the queue
//! holds any and `txen` is set, the oldest goes out on the line as a
//! start bit, eight data bits least significant first and one stop bit,
//! or two with `nstop`, each `div` plus one cycles long. A write while
//! the queue is full is dropped, and a read of `txdata` says so in its
//! bit 31. A frame coming in on the other line while `rxen` is set,
//! sampled in the middle of each bit, lands in a queue of sixty-four
//! behind `rxdata`, whose read takes the oldest and says in bit 31 when
//! there was none; a byte that finds the queue full is dropped and
//! counted.
//! `ip` says which watermark is passed, the transmit queue holding
//! fewer than `txcnt` or the receive queue more than `rxcnt`, and the
//! port's interrupt line is high while one that `ie` enables is.
//! The lines rest high.
//!
//! Two resets differ from SiFive's, so that a program that never
//! writes the controls finds the port as it was before this map: both
//! enables start set. `div` starts at `DIV` less one: 867 for 115200
//! baud at 100 MHz, and 3 in the runs that are checked, so that a byte
//! takes forty cycles rather than nine thousand. `ie` starts clear, as
//! SiFive's does, so the interrupt line stays low until a program asks
//! for it; Linux's driver relies on that, and a receive bit that
//! started set let the loader's bytes leave a request pending in the
//! interrupt controller, which Linux took before its port was ready
//! (issues 1136 and 1137).
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::regmap;
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};
use txhdl_parts::bus::axi::Resp;
use txhdl_parts::bus::axi_lite::{LiteB, LitePort, LiteR};

// The pieces of the step, each a function of its own, inlined by the
// lowering where the step calls it.

/// The frame a byte goes out as, least significant bit first: a start
/// bit low, the byte, and two stop bits high, of which the second goes
/// out only with `nstop`.
#[lower]
fn frame(octet: U<8>) -> U<11> {
    U::<2>::from(3u8)
        .concat::<_, 10>(octet)
        .concat::<_, 11>(U::<1>::from(0u8))
}

/// The frame after a bit has gone out: shifted down, the line's rest
/// state shifted in at the top.
#[lower]
fn shifted(shift: U<11>) -> U<11> {
    U::<1>::from(1u8).concat::<_, 11>(shift.slice::<1, 10>())
}

/// The cycle a bit is sampled in, the middle of its `div` plus one.
#[lower]
fn half(div: U<16>) -> U<16> {
    (div + 1).slice::<1, 15>().zext::<16>()
}

/// A frame coming in, one more bit taken at the top: bit 0 of the
/// byte comes first and ends up lowest.
#[lower]
fn taken_in(shift: U<8>, line: Bit) -> U<8> {
    line.zext::<1>().concat::<_, 8>(shift.slice::<1, 7>())
}

// begin{map}
// The map: SiFive's seven words, three address bits above the byte
// bits selecting one, and the eighth reads zero (issue 1011).
regmap! { serial (serial_read, serial_we, serial_re), 3: [
    (0, txdata, rw, "a byte to send, and whether the queue is full", [
        (data, 0, 8, wo, 0, "the byte to send"),
        (full, 31, 1, ro, 0, "the queue is full; a byte written now is dropped"),
    ]),
    (1, rxdata, rc, "the oldest byte received", [
        (data, 0, 8, ro, 0, "the byte"),
        (empty, 31, 1, ro, 1, "nothing was waiting, and the byte is not one"),
    ]),
    (2, txctrl, rw, "the transmitter's controls", [
        (txen, 0, 1, rw, 1, "send what the queue holds"),
        (nstop, 1, 1, rw, 0, "two stop bits rather than one"),
        (txcnt, 16, 3, rw, 0, "txwm is pending while fewer than this wait"),
    ]),
    (3, rxctrl, rw, "the receiver's controls", [
        (rxen, 0, 1, rw, 1, "take frames from the line"),
        (rxcnt, 16, 3, rw, 0, "rxwm is pending while more than this wait"),
    ]),
    (4, ie, rw, "which watermarks raise the interrupt line", [
        (txwm, 0, 1, rw, 0, "the transmit watermark"),
        (rxwm, 1, 1, rw, 0, "the receive watermark"),
    ]),
    (5, ip, ro, "which watermarks are passed", [
        (txwm, 0, 1, ro, 0, "fewer than txcnt bytes wait to be sent"),
        (rxwm, 1, 1, ro, 0, "more than rxcnt bytes wait to be read"),
    ]),
    (6, div, rw, "the bit period in cycles, less one", [
        (div, 0, 16, rw, 867, "the divider; 867 for 115200 at 100 MHz"),
    ]),
] }
// end{map}

#[derive(Trace)]
pub struct Uart<const DIV: u32> {
    /// The frame going out, least significant bit first.
    pub shift: Reg<U<11>>,
    /// Bits left to send; busy while not zero.
    pub bits: Reg<U<4>>,
    /// Cycles left in the current bit.
    pub tick: Reg<U<16>>,
    /// The last byte the queue accepted, and how many it did.
    pub last: Reg<U<8>>,
    pub sent: Reg<U<8>>,
    /// The bytes written and not yet sent: a queue of eight, the index
    /// of the oldest and how many are held.
    pub txq: Mem<U<8>, 8>,
    pub tx_head: Reg<U<3>>,
    pub tx_count: Reg<U<4>>,
    /// The line in, as the edge left it: one register between the
    /// pin and the logic.
    pub line: Reg<Bit>,
    /// The frame coming in, the bits left of it, start and stop
    /// included, and the cycles into the current bit.
    pub rx_shift: Reg<U<8>>,
    pub rx_bits: Reg<U<4>>,
    pub rx_tick: Reg<U<16>>,
    /// The bytes received and not yet read: a queue of sixty-four, the
    /// index of the oldest and how many are held; how many came in
    /// all, and how many found the queue full and were dropped. Eight,
    /// SiFive's depth, dropped a line pasted at Linux's shell, which
    /// takes longer than eight characters' time to answer an interrupt
    /// on this core (issue 1153).
    pub fifo: Mem<U<8>, 64>,
    pub head: Reg<U<6>>,
    pub count: Reg<U<7>>,
    pub received: Reg<U<8>>,
    pub dropped: Reg<U<8>>,
    /// The controls, as `txctrl`, `rxctrl`, `ie` and `div` hold them.
    pub txen: Reg<Bit>,
    pub nstop: Reg<Bit>,
    pub txcnt: Reg<U<3>>,
    pub rxen: Reg<Bit>,
    pub rxcnt: Reg<U<3>>,
    pub ie_txwm: Reg<Bit>,
    pub ie_rxwm: Reg<Bit>,
    pub div: Reg<U<16>>,
}

impl<const DIV: u32> Default for Uart<DIV> {
    /// The controls start where the module comment says, the rest at
    /// zero (issue 890).
    fn default() -> Self {
        Uart {
            shift: Reg::default(),
            bits: Reg::default(),
            tick: Reg::default(),
            last: Reg::default(),
            sent: Reg::default(),
            txq: Mem::default(),
            tx_head: Reg::default(),
            tx_count: Reg::default(),
            line: Reg::default(),
            rx_shift: Reg::default(),
            rx_bits: Reg::default(),
            rx_tick: Reg::default(),
            fifo: Mem::default(),
            head: Reg::default(),
            count: Reg::default(),
            received: Reg::default(),
            dropped: Reg::default(),
            txen: Reg::new(Bit::One),
            nstop: Reg::default(),
            txcnt: Reg::default(),
            rxen: Reg::new(Bit::One),
            rxcnt: Reg::default(),
            ie_txwm: Reg::default(),
            ie_rxwm: Reg::default(),
            div: Reg::new(U::from(DIV - 1)),
        }
    }
}

#[lower]
impl<const DIV: u32> Unit for Uart<DIV> {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (rst, rx, tx, irq): (In<Bit>, In<Bit>, Out<Bit>, Out<Bit>),
    ) {
        loop {
            DefaultClock::rising().await;
            let rst = rst.get().to_bool();
            let rx_ready = self.count != 0;
            let rx_full = self.count == 64;
            let rx_data = self.fifo.read(self.head.get());
            let tx_full = self.tx_count == 8;
            let tx_some = self.tx_count != 0;
            let busy = self.bits != 0;
            let div = self.div.get();
            // The watermarks: the transmit queue holding fewer than its
            // count, the receive queue more than its.
            let txwm = self.tx_count.get() < self.txcnt.get().zext::<4>();
            let rxwm = self.count.get() > self.rxcnt.get().zext::<7>();
            // The bridge sends this peripheral only the transactions in
            // its range, so it checks no address. A read is answered in
            // the cycle it is taken, and a write is taken when its
            // address and its word are both there, and answered at once.
            let arh = bus.ar.head();
            let take_read = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let sel = arh.addr.slice::<2, 3>();
            let wsel = awh.addr.slice::<2, 3>();
            let we = serial_we(wgo, wsel);
            // Whether this read is the one that takes a received byte:
            // the map's read enable for `rxdata`.
            let read_rx = serial_re(take_read, sel).bit(1).to_bool();
            // A byte written joins the transmit queue unless it is full.
            let push_tx = we.bit(0).to_bool() & !tx_full;
            // The oldest byte starts out when the line is free and the
            // transmitter is enabled; a reset wins over everything.
            let start = !busy & tx_some & self.txen.to_bool();
            let octet = self.txq.read(self.tx_head.get());
            if rst {
                self.bits.set(0);
                self.tick.set(0);
            } else if start {
                self.shift.set(frame(octet));
                if self.nstop.to_bool() {
                    self.bits.set(11);
                } else {
                    self.bits.set(10);
                }
                self.tick.set(0);
            } else if busy {
                if self.tick.get() == div {
                    self.tick.set(0);
                    self.bits.set(self.bits - 1);
                    self.shift.set(shifted(self.shift.get()));
                } else {
                    self.tick.set(self.tick + 1);
                }
            }
            let tx_tail = self.tx_head + self.tx_count.get().slice::<0, 3>();
            if push_tx & !rst {
                let written = serial_txdata_data(wh.data);
                self.txq.at(tx_tail).set(written);
                self.last.set(written);
                self.sent.set(self.sent + 1);
            }
            let pop_tx = start & !rst;
            if rst {
                self.tx_head.set(0);
                self.tx_count.set(0);
            } else {
                if pop_tx {
                    self.tx_head.set(self.tx_head + 1);
                }
                if push_tx & !pop_tx {
                    self.tx_count.set(self.tx_count + 1);
                } else if pop_tx & !push_tx {
                    self.tx_count.set(self.tx_count - 1);
                }
            }
            // The controls: a reset puts them back where they started.
            if rst {
                self.txen.set(Bit::One);
                self.nstop.set(Bit::Zero);
                self.txcnt.set(0);
                self.rxen.set(Bit::One);
                self.rxcnt.set(0);
                self.ie_txwm.set(Bit::Zero);
                self.ie_rxwm.set(Bit::Zero);
                self.div.set(U::from(DIV - 1));
            } else {
                if we.bit(2).to_bool() {
                    self.txen.set(serial_txctrl_txen(wh.data));
                    self.nstop.set(serial_txctrl_nstop(wh.data));
                    self.txcnt.set(serial_txctrl_txcnt(wh.data));
                }
                if we.bit(3).to_bool() {
                    self.rxen.set(serial_rxctrl_rxen(wh.data));
                    self.rxcnt.set(serial_rxctrl_rxcnt(wh.data));
                }
                if we.bit(4).to_bool() {
                    self.ie_txwm.set(serial_ie_txwm(wh.data));
                    self.ie_rxwm.set(serial_ie_rxwm(wh.data));
                }
                if we.bit(6).to_bool() {
                    self.div.set(serial_div_div(wh.data));
                }
            }
            if take_read.to_bool() {
                bus.r.send(LiteR {
                    data: serial_read(
                        sel,
                        serial_txdata_pack(
                            U::<8>::from(0u8),
                            Bit::from(tx_full),
                        ),
                        serial_rxdata_pack(
                            mux(rx_ready, rx_data, U::<8>::from(0u8)),
                            Bit::from(!rx_ready),
                        ),
                        serial_txctrl_pack(
                            self.txen.get(),
                            self.nstop.get(),
                            self.txcnt.get(),
                        ),
                        serial_rxctrl_pack(self.rxen.get(), self.rxcnt.get()),
                        serial_ie_pack(self.ie_txwm.get(), self.ie_rxwm.get()),
                        serial_ip_pack(Bit::from(txwm), Bit::from(rxwm)),
                        serial_div_pack(div),
                    ),
                    resp: Resp::Okay,
                });
            }
            if wgo.to_bool() {
                bus.b.send(LiteB { resp: Resp::Okay });
            }
            tx.set(mux(busy, self.shift.get().bit(0), Bit::One));
            // The receive side. A low on the resting line, with the
            // receiver enabled, is a start bit; from then on the line is
            // sampled in the middle of each bit, ten of them: a high
            // where the start bit should be is a false start and the
            // frame is dropped, the eight bits in between are taken into
            // the shift register, and a high at the stop bit lands the
            // byte in the queue, whose oldest the read of `rxdata` takes;
            // the queue full, the byte is dropped and counted.
            self.line.set(rx.get());
            let receiving = self.rx_bits != 0;
            let sample = !rst & receiving & (self.rx_tick.get() == half(div));
            let at_start = self.rx_bits == 10;
            let at_stop = self.rx_bits == 1;
            let line = self.line.to_bool();
            if rst {
                self.rx_bits.set(0);
                self.rx_tick.set(0);
                self.head.set(0);
                self.count.set(0);
            } else if !receiving {
                if !line & self.rxen.to_bool() {
                    self.rx_bits.set(10);
                    self.rx_tick.set(0);
                }
            } else if self.rx_tick.get() == div {
                self.rx_tick.set(0);
                self.rx_bits.set(self.rx_bits - 1);
            } else {
                self.rx_tick.set(self.rx_tick + 1);
            }
            with!(self <= {
                sample & at_start & line ? rx_bits: 0,
                sample & !at_start & !at_stop ?
                    rx_shift: taken_in(self.rx_shift.get(), self.line.get()),
                sample & at_stop ? rx_bits: 0,
            });
            let landed = sample & at_stop & line;
            let push = landed & !rx_full;
            let pop = read_rx & rx_ready;
            let tail = self.head + self.count.get().slice::<0, 6>();
            if landed {
                if rx_full {
                    self.dropped.set(self.dropped + 1);
                } else {
                    self.fifo.at(tail).set(self.rx_shift);
                    self.received.set(self.received + 1);
                }
            }
            if !rst {
                if pop {
                    self.head.set(self.head + 1);
                }
                if push & !pop {
                    self.count.set(self.count + 1);
                } else if pop & !push {
                    self.count.set(self.count - 1);
                }
            }
            let tx_irq = self.ie_txwm.to_bool() & txwm;
            let rx_irq = self.ie_rxwm.to_bool() & rxwm;
            irq.set(tx_irq | rx_irq);
        }
    }
}
