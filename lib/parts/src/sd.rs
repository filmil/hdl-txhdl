// SPDX-License-Identifier: Apache-2.0
//! A native-mode SD card host on AXI-Lite, and a model of a card to
//! check it against.
//!
//! A card in SPI mode is a few hundred kilobytes a second on one data
//! line, which is what [`crate::spi`] gives. The native mode is the
//! card's own protocol: a command line, `CMD`, that carries 48-bit
//! commands out and 48- or 136-bit responses back, and one or four
//! data lines, `DAT`, that carry blocks of 512 bytes either way, each
//! line with a CRC-16 of its own, at 25 or 50 MHz once the card is
//! up. Every command carries a CRC-7, and so does every response but
//! the one to `ACMD41`, whose CRC field is all ones (issue 153).
//!
//! The host here does one command at a time, and with it at most one
//! block. A program writes the argument and the command word, and the
//! command word says what follows: no response, a short one or a long
//! one; a block to read after it or a block to write; whether to waited
//! for the card's busy line afterwards; and whether to check the
//! response's CRC. The host sends the command, takes the response,
//! moves the block through a buffer of 128 words that the program
//! reads or fills through one register, and says in `status` how it
//! went. A multi-block transfer is one command, `CMD18` or `CMD25`,
//! followed by data-only transfers, one a block, and `CMD12`; a card
//! streams a read back to back, and keeping up with that at 25 MHz
//! through a register is what DMA is for, which is left for later.
//!
//! | Offset | Name | What it is |
//! |---|---|---|
//! | `0x00` | `ctrl` | divider, four lines, interrupt enable; bit 10 clears |
//! | `0x04` | `cmd` | index, response, transfer, checks; written, it starts |
//! | `0x08` | `arg` | the argument |
//! | `0x0c` | `status` | busy, done, faults, CRC status, busy line, pointers |
//! | `0x10` to `0x1c` | `resp0` to `resp3` | the response |
//! | `0x20` | `data` | read: the next word out; written: the next word in |
//!
//! `ctrl` is the divider in bits 0 to 7, `wide` in bit 8 and the
//! interrupt enable in bit 9. A half of a card clock takes `div + 1`
//! cycles, so the card's clock is the system's over `2 * (div + 1)`:
//! at 100 MHz, 124 is 400 kHz for the start, 1 is 25 MHz and 0 is
//! 50 MHz.
//!
//! `cmd` is the index in bits 0 to 5, the response in bits 6 and 7
//! (0 none, 1 short, 2 long), read in bit 8, write in bit 9, waited for
//! busy in bit 10, skip the response's CRC in bit 11, data only in
//! bit 12, which sends no command and moves a block, and clocks alone
//! in bit 13, which runs eighty clocks with the command line held high
//! and nothing else: a card wants at least 74 of those after power-up
//! before its first command.
//!
//! `status` is busy in bit 0, done in bit 1, a response that never
//! came in bit 2, a response whose CRC was wrong in bit 3, a block
//! that never came or a card that never left busy in bit 4, a block
//! whose CRC was wrong, or that the card refused, in bit 5, the card's
//! three CRC status bits in bits 6 to 8, the data line's level in bit
//! 9, the response's index in bits 10 to 15, the write pointer in bits
//! 16 to 23 and the read pointer in bits 24 to 31. Writing bit 1
//! clears done and the faults.
//!
//! Timing is the card's: the host drives its lines on the falling edge
//! of the clock it makes and samples the card's on the rising edge,
//! and the card does the same. A CRC is checked the way a shift
//! register does it: the received CRC is fed into the same register
//! after the data, and what is left is zero when they agree.
use txhdl::comp::{mux, Clock, DefaultClock, In, Mem, Out, Reg, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, with, Trace};

use crate::bus::axi::Resp;
use crate::bus::axi_lite::{LiteB, LitePort, LiteR};
use txhdl::regmap;

regmap! { regs (regs_read, regs_we), 4: [
    (0, ctrl, rw, "the divider, the lines, the interrupt enable", [
        (div, 0, 8, rw, 0, "cycles a half of the card clock, less one"),
        (wide, 8, 1, rw, 0, "four data lines rather than one"),
        (ie, 9, 1, rw, 0, "a finished command raises the interrupt"),
        (clear, 10, 1, wo, 0, "written, the buffer's pointers go to zero"),
    ]),
    (1, cmd, rw, "the command word; written, it starts", [
        (index, 0, 6, rw, 0, "the command's index"),
        (resp, 6, 2, rw, 0, "the response: none, short, long"),
        (read, 8, 1, rw, 0, "a block comes after the response"),
        (write, 9, 1, rw, 0, "a block goes after the response"),
        (busy, 10, 1, rw, 0, "wait for the card's busy line afterwards"),
        (nocrc, 11, 1, rw, 0, "the response carries no CRC"),
        (only, 12, 1, rw, 0, "no command, only the block"),
        (clocks, 13, 1, rw, 0, "no command and no block: eighty clocks"),
    ]),
    (2, arg, rw, "the argument"),
    (3, status, w1c, "how the last command went", [
        (busy, 0, 1, ro, 0, "a command is running"),
        (done, 1, 1, w1c, 0, "finished; written, clears the faults too"),
        (rtimeout, 2, 1, ro, 0, "no response came"),
        (rcrc, 3, 1, ro, 0, "the response's CRC was wrong"),
        (dtimeout, 4, 1, ro, 0, "no block came, or the card stayed busy"),
        (dcrc, 5, 1, ro, 0, "the block's CRC was wrong or refused"),
        (crcstat, 6, 3, ro, 0, "the card's CRC status token"),
        (dat0, 9, 1, ro, 0, "the first data line's level"),
        (index, 10, 6, ro, 0, "the response's index"),
        (wptr, 16, 8, ro, 0, "where the next word goes in"),
        (rptr, 24, 8, ro, 0, "where the next word comes out"),
    ]),
    (4, resp0, ro, "the response's first word"),
    (5, resp1, ro, "its second"),
    (6, resp2, ro, "its third"),
    (7, resp3, ro, "its fourth"),
    (8, data, rw, "read: the next word out; written: the next word in"),
] }

/// `ctrl` bit 8: four data lines.
pub const CTRL_WIDE: u32 = regs::ctrl_wide.mask();
/// `ctrl` bit 9: a finished command raises the interrupt.
pub const CTRL_IE: u32 = regs::ctrl_ie.mask();
/// `ctrl` bit 10, written: the buffer's pointers go to zero.
pub const CTRL_CLEAR: u32 = regs::ctrl_clear.mask();

/// `cmd` bits 6 and 7: a short response, 48 bits.
pub const CMD_SHORT: u32 = regs::cmd_resp.with(1);
/// `cmd` bits 6 and 7: a long response, 136 bits.
pub const CMD_LONG: u32 = regs::cmd_resp.with(2);
/// `cmd` bit 8: a block comes after the response.
pub const CMD_READ: u32 = regs::cmd_read.mask();
/// `cmd` bit 9: a block goes after the response.
pub const CMD_WRITE: u32 = regs::cmd_write.mask();
/// `cmd` bit 10: waited for the card's busy line afterwards.
pub const CMD_BUSY: u32 = regs::cmd_busy.mask();
/// `cmd` bit 11: the response carries no CRC.
pub const CMD_NOCRC: u32 = regs::cmd_nocrc.mask();
/// `cmd` bit 12: no command, only the block.
pub const CMD_DATA_ONLY: u32 = regs::cmd_only.mask();
/// `cmd` bit 13: no command and no block, eighty clocks with the
/// command line held high, which a card wants before `CMD0`.
pub const CMD_CLOCKS: u32 = regs::cmd_clocks.mask();

/// `status` bit 0: a command is running.
pub const STATUS_BUSY: u32 = regs::status_busy.mask();
/// `status` bit 1: the last command finished.
pub const STATUS_DONE: u32 = regs::status_done.mask();
/// `status` bit 2: no response came.
pub const STATUS_RTIMEOUT: u32 = regs::status_rtimeout.mask();
/// `status` bit 3: the response's CRC was wrong.
pub const STATUS_RCRC: u32 = regs::status_rcrc.mask();
/// `status` bit 4: no block came, or the card never left busy.
pub const STATUS_DTIMEOUT: u32 = regs::status_dtimeout.mask();
/// `status` bit 5: the block's CRC was wrong, or the card refused it.
pub const STATUS_DCRC: u32 = regs::status_dcrc.mask();

/// Words in the buffer: one block.
pub const WORDS: usize = 128;
/// Card clocks a response may take to start.
pub const RESPONSE_WAIT: u32 = 64;

// begin{state}
/// The host: one command and one block at a time.
#[derive(Trace, Default)]
pub struct Sd {
    /// A half of a card clock is this many cycles, less one.
    pub div: Reg<U<8>>,
    /// Four data lines rather than one.
    pub wide: Reg<Bit>,
    /// A finished command raises the interrupt.
    pub ie: Reg<Bit>,
    /// The command word as written.
    pub cmdw: Reg<U<14>>,
    /// The argument.
    pub arg: Reg<U<32>>,
    /// A command is running.
    pub busy: Reg<Bit>,
    /// Where the command is: idle, command, response waited, response,
    /// block waited, block in, block out, CRC status, busy, done.
    pub phase: Reg<U<4>>,
    /// Cycles into the half clock.
    pub tick: Reg<U<8>>,
    /// The card clock's level.
    pub half: Reg<Bit>,
    /// Bits or nibbles into the phase.
    pub n: Reg<U<13>>,
    /// Card clocks waited for something that has not come.
    pub waited: Reg<U<24>>,
    /// The command's first forty bits, its next on top.
    pub cmdsr: Reg<U<40>>,
    /// The CRC-7, of the command going out or the response coming in.
    pub crc7: Reg<U<7>>,
    /// The response, its last 128 bits.
    pub resp: Reg<U<128>>,
    /// The word going out or coming in.
    pub dsr: Reg<U<32>>,
    /// Bits of it done.
    pub dn: Reg<U<6>>,
    /// The CRC-16 of each data line.
    pub crc0: Reg<U<16>>,
    /// The second line's.
    pub crc1: Reg<U<16>>,
    /// The third line's.
    pub crc2: Reg<U<16>>,
    /// The fourth line's.
    pub crc3: Reg<U<16>>,
    /// The block.
    pub words: Mem<U<32>, WORDS>,
    /// Where the next word goes in. Seven bits, since the buffer is
    /// 128 words and a netlist indexes it as written (issue 556).
    pub wptr: Reg<U<7>>,
    /// Where the next word comes out.
    pub rptr: Reg<U<7>>,
    /// The last command finished.
    pub done: Reg<Bit>,
    /// No response came.
    pub rtimeout: Reg<Bit>,
    /// The response's CRC was wrong.
    pub rcrc: Reg<Bit>,
    /// No block came, or the card stayed busy.
    pub dtimeout: Reg<Bit>,
    /// The block's CRC was wrong or the card refused it.
    pub dcrc: Reg<Bit>,
    /// The card's CRC status token.
    pub crcstat: Reg<U<3>>,
    /// What the host drives on `CMD`, and whether it drives it.
    pub cmd_o: Reg<Bit>,
    /// Whether the host drives `CMD`.
    pub cmd_drv: Reg<Bit>,
    /// What the host drives on `DAT`.
    pub dat_o: Reg<U<4>>,
    /// Whether the host drives `DAT`.
    pub dat_drv: Reg<Bit>,
}
// end{state}

/// The phases.
const IDLE: u8 = 0;
const SEND: u8 = 1;
const RWAIT: u8 = 2;
const RESP: u8 = 3;
const DWAIT: u8 = 4;
const DRECV: u8 = 5;
const DSEND: u8 = 6;
const CRCSTAT: u8 = 7;
const BUSY: u8 = 8;
const FINISH: u8 = 9;
const CLOCKS: u8 = 10;

/// One step of a CRC-7, x^7 + x^3 + 1, on one bit.
#[lower]
fn crc7_step(crc: U<7>, bit: Bit) -> U<7> {
    let inv = bit ^ crc.bit(6);
    (crc << 1u32) ^ mux(inv, U::<7>::from(0x09u8), U::<7>::from(0u8))
}

/// One step of a CRC-16, x^16 + x^12 + x^5 + 1, on one bit.
#[lower]
fn crc16_step(crc: U<16>, bit: Bit) -> U<16> {
    let inv = bit ^ crc.bit(15);
    (crc << 1u32) ^ mux(inv, U::<16>::from(0x1021u32), U::<16>::from(0u8))
}

// begin{run}
#[lower]
impl Unit for Sd {
    async fn run(
        &mut self,
        bus: LitePort<32, 32, 4>,
        (cmd_in, dat_in, sclk, cmd_out, cmd_oe, dat_out, dat_oe, irq): (
            In<Bit>,
            In<U<4>>,
            Out<Bit>,
            Out<Bit>,
            Out<Bit>,
            Out<U<4>>,
            Out<Bit>,
            Out<Bit>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
            let div = self.div.get();
            let wide = self.wide.get();
            let ie = self.ie.get();
            let cmdw = self.cmdw.get();
            let arg = self.arg.get();
            let busy = self.busy.get();
            let phase = self.phase.get();
            let tick = self.tick.get();
            let half = self.half.get();
            let n = self.n.get();
            let waited = self.waited.get();
            let cmdsr = self.cmdsr.get();
            let crc7 = self.crc7.get();
            let resp = self.resp.get();
            let dsr = self.dsr.get();
            let dn = self.dn.get();
            let crc0 = self.crc0.get();
            let crc1 = self.crc1.get();
            let crc2 = self.crc2.get();
            let crc3 = self.crc3.get();
            let wptr = self.wptr.get();
            let rptr = self.rptr.get();
            let done = self.done.get();
            let cmd_o = self.cmd_o.get();
            let dat_o = self.dat_o.get();
            let cin = cmd_in.get();
            let din = dat_in.get();
            let dat0 = din.bit(0);
            // The command word's fields.
            let rlong = cmdw.slice::<6, 2>() == 2;
            let rnone = cmdw.slice::<6, 2>() == 0;
            let rd = cmdw.bit(8);
            let wr = cmdw.bit(9);
            let waitbusy = cmdw.bit(10);
            let nocrc = cmdw.bit(11);
            // The card clock. `strobe` is the moment it turns over,
            // `falling` the turn down, on which the host drives, and
            // `rising` the turn up, on which it samples.
            let strobe = busy & (tick == div);
            let falling = strobe & half;
            let rising = strobe & !half;
            // The bus.
            let arh = bus.ar.head();
            let awh = bus.aw.head();
            let wh = bus.w.head();
            let rsel = arh.addr.slice::<2, 4>();
            let wsel = awh.addr.slice::<2, 4>();
            let rgo = bus.r.ready() & bus.ar.peek().is_some();
            let _ = bus.ar.recv_if(bus.r.ready());
            let wgo = bus.b.ready()
                & bus.aw.peek().is_some()
                & bus.w.peek().is_some();
            let _ = bus.aw.recv_if(wgo);
            let _ = bus.w.recv_if(wgo);
            let written = wh.data;
            // A write to `cmd` starts a command, and is ignored while
            // one runs: a program reads `status` first.
            // The map's write enables, a bit a register in its order.
            let we = regs_we(wgo, wsel);
            let start = we.bit(1) & !busy;
            let start_rd = regs_cmd_read(written);
            let start_wr = regs_cmd_write(written);
            let start_only = regs_cmd_only(written);
            let start_clocks = regs_cmd_clocks(written);
            // Where a command starts: clocks alone, a block alone, or
            // the command itself.
            let start_data =
                mux(start_rd, U::<4>::from(DWAIT), U::<4>::from(DSEND));
            let start_phase = mux(
                start_clocks,
                U::<4>::from(CLOCKS),
                mux(start_only, start_data, U::<4>::from(SEND)),
            );
            // A read of `data`, the ninth word, pops; the map has no
            // read enables, so the index is written here.
            let pop = rgo & (rsel == 8);
            let push = we.bit(8);
            let clear = we.bit(0) & regs_ctrl_clear(written);
            let ack = we.bit(3) & regs_status_done(written);
            // The phases.
            let in_send = phase == SEND;
            let in_rwait = phase == RWAIT;
            let in_resp = phase == RESP;
            let in_dwait = phase == DWAIT;
            let in_drecv = phase == DRECV;
            let in_dsend = phase == DSEND;
            let in_crcstat = phase == CRCSTAT;
            let in_busy = phase == BUSY;
            let in_finish = phase == FINISH;
            let in_clocks = phase == CLOCKS;
            // Sending: forty bits from the shift register, seven of
            // CRC, and the end bit; the bit driven now is also the one
            // the CRC takes.
            let send_data = falling & in_send & (n < 40);
            let send_crc =
                falling & in_send & Bit::from((40..47).contains(&n.raw()));
            let send_end = falling & in_send & (n == 47);
            let send_over = falling & in_send & (n == 48);
            // The response: its start bit ends the waited, and the CRC
            // covers every bit before its own for a short one and the
            // 120 after the header for a long one.
            let rlen = mux(rlong, U::<13>::from(136u32), U::<13>::from(48u32));
            let rstart = rising & in_rwait & !cin;
            let rgiveup = rising & in_rwait & (waited == RESPONSE_WAIT - 1);
            let rbit = rising & in_resp;
            let rcovered =
                mux(rlong, Bit::from(n >= 8), Bit::One) & (n < rlen - 1);
            let rlast = rbit & (n == rlen - 1);
            let rbad = rlast & !nocrc & (crc7 != 0);
            // A block coming in: the start bit on the first line ends
            // the waited; then the data, the CRC of each line, the end.
            let dlen =
                mux(wide, U::<13>::from(1024u32), U::<13>::from(4096u32));
            let dstart = rising & in_dwait & !dat0;
            let dgiveup = rising & in_dwait & (waited == 0xff_ffff);
            let dbit = rising & in_drecv & (n < dlen);
            let dcrcbit = rising & in_drecv & (n >= dlen) & (n < dlen + 16);
            let dend = rising & in_drecv & (n == dlen + 16);
            let crc_all = crc0
                | mux(wide, crc1, U::<16>::from(0u8))
                | mux(wide, crc2, U::<16>::from(0u8))
                | mux(wide, crc3, U::<16>::from(0u8));
            let dbad = dend & (crc_all != 0);
            // The word gathered: the new bit or nibble at the bottom.
            let dnext = mux(
                wide,
                (dsr << 4u32) | din.zext::<32>(),
                (dsr << 1u32) | dat0.zext::<32>(),
            );
            let dstep = mux(wide, U::<6>::from(4u8), U::<6>::from(1u8));
            let dfull = dbit & (dn + dstep == 32);
            // A block going out: two clocks of quiet, the start bit,
            // the data from the buffer, the CRC of each line, the end
            // bit, then the line released for the card's answer.
            let out_start = falling & in_dsend & (n == 2);
            let out_data = falling & in_dsend & (n >= 3) & (n < dlen + 3);
            let out_crc =
                falling & in_dsend & (n >= dlen + 3) & (n < dlen + 19);
            let out_end = falling & in_dsend & (n == dlen + 19);
            let out_over = falling & in_dsend & (n == dlen + 20);
            // The word going out: loaded from the buffer as its first
            // bit or nibble is needed.
            let fresh = dn == 0;
            let src = mux(fresh, self.words.read(rptr), dsr);
            let obit = src.bit(31);
            let onib = src.slice::<28, 4>();
            let oshift = mux(wide, src << 4u32, src << 1u32);
            // The CRC status token: a start bit, three bits, the end.
            let tstart = rising & in_crcstat & (n == 0) & !dat0;
            let tgiveup =
                rising & in_crcstat & (n == 0) & (waited == 0xff_ffff);
            let tbit =
                rising & in_crcstat & Bit::from((1..4).contains(&n.raw()));
            let tend = rising & in_crcstat & (n == 4);
            let tbad = tend & (self.crcstat.get() != 2);
            // Busy: the card holds the first line low, and the host
            // waits until it has been high again after a few clocks.
            let bclear = rising & in_busy & (n >= 4) & dat0;
            let bgiveup = rising & in_busy & (waited == 0xff_ffff);
            // What follows the response, or the command with none.
            // The CRC of a short response covers its start bit, so the
            // register starts with it fed; a long one's covers only the
            // payload, so it starts empty.
            let crc7_first = mux(
                rlong,
                U::<7>::from(0u8),
                crc7_step(U::<7>::from(0u8), cin),
            );
            let after_data =
                mux(waitbusy, U::<4>::from(BUSY), U::<4>::from(FINISH));
            let obit1 = U::<3>::from(7u8).concat::<1, 4>(obit.zext::<1>());
            let ocrc1 =
                U::<3>::from(7u8).concat::<1, 4>(crc0.bit(15).zext::<1>());
            let after_resp = mux(
                rd,
                U::<4>::from(DWAIT),
                mux(
                    wr,
                    U::<4>::from(DSEND),
                    mux(waitbusy, U::<4>::from(BUSY), U::<4>::from(FINISH)),
                ),
            );
            let cmd_next = mux(
                rnone,
                after_resp,
                mux(rbad, U::<4>::from(FINISH), after_resp),
            );
            // The registers a program reads.
            // The registers a program reads, packed from their fields as
            // the map declares them.
            let ctrl = regs_ctrl_pack(div, wide, ie, Bit::Zero);
            let status = regs_status_pack(
                busy,
                done,
                self.rtimeout.get(),
                self.rcrc.get(),
                self.dtimeout.get(),
                self.dcrc.get(),
                self.crcstat.get(),
                dat0,
                resp.slice::<40, 6>(),
                wptr.zext::<8>(),
                rptr.zext::<8>(),
            );
            let resp0 =
                mux(rlong, resp.slice::<0, 32>(), resp.slice::<8, 32>());
            // The word a read answers, from the map.
            let word = regs_read(
                rsel,
                ctrl,
                cmdw.zext::<32>(),
                arg,
                status,
                resp0,
                resp.slice::<32, 32>(),
                resp.slice::<64, 32>(),
                resp.slice::<96, 32>(),
                self.words.read(rptr),
            );
            with!(self <= {
                we.bit(0) ? {
                    div: regs_ctrl_div(written),
                    wide: regs_ctrl_wide(written),
                    ie: regs_ctrl_ie(written),
                },
                we.bit(2) ? arg: written,
                clear ? {
                    wptr: U::<7>::from(0u8),
                    rptr: U::<7>::from(0u8),
                },
                ack ? {
                    done: Bit::Zero,
                    rtimeout: Bit::Zero,
                    rcrc: Bit::Zero,
                    dtimeout: Bit::Zero,
                    dcrc: Bit::Zero,
                },
                push ? {
                    words.at(wptr): written,
                    wptr: wptr + 1,
                },
                pop ? rptr: rptr + 1,
                // A command starts: the shift register is the two
                // header bits, the index and the argument.
                start ? {
                    cmdw: written.slice::<0, 14>(),
                    busy: Bit::One,
                    phase: start_phase,
                    tick: U::<8>::from(0u8),
                    half: Bit::Zero,
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                    cmdsr: U::<2>::from(1u8)
                        .concat::<6, 8>(written.slice::<0, 6>())
                        .concat::<32, 40>(arg),
                    crc7: U::<7>::from(0u8),
                    crc0: U::<16>::from(0u8),
                    crc1: U::<16>::from(0u8),
                    crc2: U::<16>::from(0u8),
                    crc3: U::<16>::from(0u8),
                    dn: U::<6>::from(0u8),
                    done: Bit::Zero,
                    rtimeout: Bit::Zero,
                    rcrc: Bit::Zero,
                    dtimeout: Bit::Zero,
                    dcrc: Bit::Zero,
                    cmd_o: Bit::One,
                    cmd_drv: !start_only,
                    dat_o: U::<4>::from(0xfu8),
                    dat_drv: Bit::Zero,
                },
                start & start_rd ? wptr: U::<7>::from(0u8),
                start & start_wr ? rptr: U::<7>::from(0u8),
                // The clock.
                busy ? tick: mux(strobe, U::<8>::from(0u8), tick + 1),
                strobe ? half: !half,
                // Clocks alone: the line stays high and the count runs.
                falling & in_clocks ? n: n + 1,
                falling & in_clocks & (n == 79) ? phase: U::<4>::from(FINISH),
                // Sending the command.
                send_data ? {
                    cmd_o: cmdsr.bit(39),
                    cmdsr: cmdsr << 1u32,
                    crc7: crc7_step(crc7, cmdsr.bit(39)),
                    n: n + 1,
                },
                send_crc ? {
                    cmd_o: crc7.bit(6),
                    crc7: crc7 << 1u32,
                    n: n + 1,
                },
                send_end ? {
                    cmd_o: Bit::One,
                    n: n + 1,
                },
                send_over ? {
                    cmd_drv: Bit::Zero,
                    phase: mux(rnone, after_resp, U::<4>::from(RWAIT)),
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                    crc7: U::<7>::from(0u8),
                },
                // The response.
                rising & in_rwait ? waited: waited + 1,
                rstart ? {
                    phase: U::<4>::from(RESP),
                    resp: cin.zext::<128>(),
                    crc7: crc7_first,
                    n: U::<13>::from(1u8),
                },
                rgiveup ? {
                    rtimeout: Bit::One,
                    phase: U::<4>::from(FINISH),
                },
                rbit ? {
                    resp: (resp << 1u32) | cin.zext::<128>(),
                    crc7: mux(rcovered, crc7_step(crc7, cin), crc7),
                    n: n + 1,
                },
                rbad ? rcrc: Bit::One,
                rlast ? {
                    phase: cmd_next,
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                },
                // A block in.
                rising & in_dwait ? waited: waited + 1,
                dstart ? {
                    phase: U::<4>::from(DRECV),
                    n: U::<13>::from(0u8),
                    dn: U::<6>::from(0u8),
                    waited: U::<24>::from(0u8),
                },
                dgiveup ? {
                    dtimeout: Bit::One,
                    phase: U::<4>::from(FINISH),
                },
                dbit ? {
                    dsr: dnext,
                    dn: dn + dstep,
                    n: n + 1,
                },
                dfull ? {
                    words.at(wptr): dnext,
                    wptr: wptr + 1,
                    dn: U::<6>::from(0u8),
                },
                (dbit | dcrcbit) ? {
                    crc0: crc16_step(crc0, dat0),
                    crc1: crc16_step(crc1, din.bit(1)),
                    crc2: crc16_step(crc2, din.bit(2)),
                    crc3: crc16_step(crc3, din.bit(3)),
                },
                dcrcbit ? n: n + 1,
                dbad ? dcrc: Bit::One,
                dend ? {
                    phase: after_data,
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                },
                // A block out.
                falling & in_dsend & (n < 2) ? n: n + 1,
                out_start ? {
                    dat_drv: Bit::One,
                    dat_o: U::<4>::from(0u8),
                    n: n + 1,
                    dn: U::<6>::from(0u8),
                },
                out_data ? {
                    dat_o: mux(wide, onib, obit1),
                    dsr: oshift,
                    dn: mux(dn + dstep == 32, U::<6>::from(0u8), dn + dstep),
                    n: n + 1,
                    crc0: crc16_step(crc0, mux(wide, onib.bit(0), obit)),
                    crc1: crc16_step(crc1, onib.bit(1)),
                    crc2: crc16_step(crc2, onib.bit(2)),
                    crc3: crc16_step(crc3, onib.bit(3)),
                },
                out_data & fresh ? rptr: rptr + 1,
                out_crc ? {
                    dat_o: mux(
                        wide,
                        crc3.bit(15).zext::<1>()
                            .concat::<1, 2>(crc2.bit(15).zext::<1>())
                            .concat::<1, 3>(crc1.bit(15).zext::<1>())
                            .concat::<1, 4>(crc0.bit(15).zext::<1>()),
                        ocrc1,
                    ),
                    crc0: crc0 << 1u32,
                    crc1: crc1 << 1u32,
                    crc2: crc2 << 1u32,
                    crc3: crc3 << 1u32,
                    n: n + 1,
                },
                out_end ? {
                    dat_o: U::<4>::from(0xfu8),
                    n: n + 1,
                },
                out_over ? {
                    dat_drv: Bit::Zero,
                    phase: U::<4>::from(CRCSTAT),
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                    crcstat: U::<3>::from(0u8),
                },
                // The card's CRC status, then its busy.
                rising & in_crcstat & (n == 0) ? waited: waited + 1,
                tstart ? n: U::<13>::from(1u8),
                tgiveup ? {
                    dtimeout: Bit::One,
                    phase: U::<4>::from(FINISH),
                },
                tbit ? {
                    crcstat: (self.crcstat.get() << 1u32) | dat0.zext::<3>(),
                    n: n + 1,
                },
                tbad ? dcrc: Bit::One,
                tend ? {
                    phase: U::<4>::from(BUSY),
                    n: U::<13>::from(0u8),
                    waited: U::<24>::from(0u8),
                },
                rising & in_busy ? {
                    n: mux(n == 8, n, n + 1),
                    waited: waited + 1,
                },
                bgiveup ? {
                    dtimeout: Bit::One,
                    phase: U::<4>::from(FINISH),
                },
                bclear ? phase: U::<4>::from(FINISH),
                // Done: the clock stops low.
                in_finish ? {
                    busy: Bit::Zero,
                    phase: U::<4>::from(IDLE),
                    done: Bit::One,
                    half: Bit::Zero,
                    cmd_drv: Bit::Zero,
                    dat_drv: Bit::Zero,
                },
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
            sclk.set(half);
            cmd_out.set(cmd_o);
            cmd_oe.set(self.cmd_drv.get());
            dat_out.set(dat_o);
            dat_oe.set(self.dat_drv.get());
            irq.set(done & ie);
        }
    }
}
// end{run}

/// A CRC-7 over `bits`, most significant first, as the card computes
/// it: x^7 + x^3 + 1, zero to start.
pub fn crc7(bits: &[bool]) -> u8 {
    let mut crc = 0u8;
    for &b in bits {
        let inv = b ^ (crc & 0x40 != 0);
        crc = (crc << 1) & 0x7f;
        if inv {
            crc ^= 0x09;
        }
    }
    crc
}

/// A CRC-16 over `bits`, most significant first: x^16 + x^12 + x^5 + 1.
pub fn crc16(bits: &[bool]) -> u16 {
    let mut crc = 0u16;
    for &b in bits {
        let inv = b ^ (crc & 0x8000 != 0);
        crc <<= 1;
        if inv {
            crc ^= 0x1021;
        }
    }
    crc
}

/// What the card is doing on its data lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Data {
    /// Nothing.
    Idle,
    /// A block is queued to go out after `at` clocks.
    Sending,
    /// A block is expected from the host.
    Expecting,
    /// A block is coming in.
    Receiving,
}

/// A card on the other end of the wires, as a simulation.
///
/// It answers the commands a host needs to bring a card up and move
/// blocks: `CMD0`, `CMD8`, `CMD55` with `ACMD41` and `ACMD6`, `CMD2`,
/// `CMD3`, `CMD7`, `CMD16`, `CMD17` and `CMD18`, `CMD24` and `CMD25`,
/// and `CMD12`. It samples the host's lines on the rising edge of the
/// clock the host makes and drives its own on the falling edge, as a
/// card does, with a few clocks between a command and its response and
/// between a response and its block. A block goes out with a CRC-16 a
/// line; a block coming in is checked the same way and answered with
/// the CRC status token and a spell of busy.
///
/// It is a model and not a part: it holds a `Vec` of blocks, and
/// nothing lowers it. Two switches make it lie once, for the tests
/// of the host's checks: `bad_resp_crc` spoils the next response's
/// CRC, and `bad_data_crc` the next block's.
#[derive(Debug)]
pub struct SdCard {
    /// The blocks, 512 bytes each.
    pub blocks: Vec<u8>,
    /// The card identification, 16 bytes with its own CRC at the end.
    pub cid: [u8; 16],
    /// The address the card answers `CMD3` with.
    pub rca: u16,
    /// Spoil the next response's CRC.
    pub bad_resp_crc: bool,
    /// Spoil the next block's CRC.
    pub bad_data_crc: bool,
    /// Clocks between a command's end and the response's start.
    pub ncr: u32,
    /// Clocks between a response's end and a block's start.
    pub nac: u32,
    /// Clocks the card is busy after taking a block.
    pub busy_len: u32,
    /// Clocks between the blocks of a multi-block read. A card sends
    /// them back to back; the gap is the model's leniency towards a
    /// host that empties its buffer through a register.
    pub multi_gap: u32,
    /// Commands whose CRC was wrong, which the card ignored.
    pub bad_commands: u32,
    /// Commands that came before the card had its clocks, ignored too.
    pub early: u32,
    /// Clocks seen with the command line high and no command on it.
    /// A card wants 74 of them after power-up before it listens.
    pub clocks: u32,
    /// How many times `ACMD41` has been asked; the card is ready on
    /// the second.
    pub acmd41: u32,
    sclk: bool,
    app: bool,
    wide: bool,
    selected: bool,
    cmd_bits: u64,
    cmd_n: u32,
    cmd_q: std::collections::VecDeque<bool>,
    cmd_delay: u32,
    dat_q: std::collections::VecDeque<u8>,
    dat_delay: u32,
    data: Data,
    multi_read: bool,
    multi_write: bool,
    addr: u32,
    busy: u32,
    rx_nibbles: Vec<u8>,
    rx_n: usize,
}

impl Default for SdCard {
    fn default() -> Self {
        SdCard::new(8)
    }
}

impl SdCard {
    /// A card of `blocks` blocks, each filled with a pattern of its
    /// own number.
    pub fn new(blocks: usize) -> Self {
        let mut bytes = vec![0u8; blocks * 512];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ((i / 512) as u8).wrapping_mul(37) ^ (i as u8);
        }
        let mut cid = [0u8; 16];
        for (i, b) in cid.iter_mut().enumerate() {
            *b = 0xa0 + i as u8;
        }
        let bits: Vec<bool> = cid[..15]
            .iter()
            .flat_map(|b| (0..8).rev().map(move |i| b & (1 << i) != 0))
            .collect();
        cid[15] = (crc7(&bits) << 1) | 1;
        SdCard {
            blocks: bytes,
            cid,
            rca: 0x1234,
            bad_resp_crc: false,
            bad_data_crc: false,
            ncr: 4,
            nac: 4,
            busy_len: 16,
            multi_gap: 2048,
            bad_commands: 0,
            early: 0,
            clocks: 0,
            acmd41: 0,
            sclk: false,
            app: false,
            wide: false,
            selected: false,
            cmd_bits: 0,
            cmd_n: 0,
            cmd_q: Default::default(),
            cmd_delay: 0,
            dat_q: Default::default(),
            dat_delay: 0,
            data: Data::Idle,
            multi_read: false,
            multi_write: false,
            addr: 0,
            busy: 0,
            rx_nibbles: Vec::new(),
            rx_n: 0,
        }
    }

    /// Whether the card drives `CMD` now.
    pub fn cmd_driving(&self) -> bool {
        self.cmd_delay == 0 && !self.cmd_q.is_empty()
    }

    /// What the card drives on `CMD`: the line rests high.
    pub fn cmd_out(&self) -> bool {
        if self.cmd_driving() {
            *self.cmd_q.front().unwrap()
        } else {
            true
        }
    }

    /// Whether the card drives `DAT` now.
    pub fn dat_driving(&self) -> bool {
        self.dat_delay == 0 && (self.busy > 0 || !self.dat_q.is_empty())
    }

    /// What the card drives on the four data lines: they rest high,
    /// and busy is the first held low.
    pub fn dat_out(&self) -> u8 {
        if self.dat_delay > 0 {
            // Nothing yet: neither the token nor busy comes before
            // its clocks have passed.
            return 0xf;
        }
        if let Some(&v) = self.dat_q.front() {
            return v;
        }
        if self.busy > 0 {
            0xe
        } else {
            0xf
        }
    }

    /// One cycle of the wires: the clock, and the command and data
    /// lines as the host drives them, with whether it does.
    pub fn step(
        &mut self,
        sclk: bool,
        cmd_drv: bool,
        cmd: bool,
        dat_drv: bool,
        dat: u8,
    ) {
        let was = self.sclk;
        self.sclk = sclk;
        if was == sclk {
            return;
        }
        if sclk {
            self.rising(cmd_drv, cmd, dat_drv, dat);
        } else {
            self.falling();
        }
    }

    /// The card samples.
    fn rising(&mut self, cmd_drv: bool, cmd: bool, dat_drv: bool, dat: u8) {
        let cmd = !cmd_drv || cmd;
        if self.cmd_n == 0 {
            if cmd {
                self.clocks += 1;
            }
            if cmd_drv && !cmd {
                self.cmd_bits = 0;
                self.cmd_n = 1;
            }
        } else {
            self.cmd_bits = (self.cmd_bits << 1) | u64::from(cmd);
            self.cmd_n += 1;
            if self.cmd_n == 48 {
                self.cmd_n = 0;
                self.command();
            }
        }
        match self.data {
            Data::Expecting if dat_drv && dat & 1 == 0 => {
                self.data = Data::Receiving;
                self.rx_nibbles.clear();
                self.rx_n = 0;
            }
            Data::Receiving => {
                self.rx_nibbles.push(dat & 0xf);
                let want = if self.wide { 1024 + 16 } else { 4096 + 16 };
                if self.rx_nibbles.len() == want + 1 {
                    self.received();
                }
            }
            _ => {}
        }
    }

    /// The card drives: the next bit of what it has queued.
    fn falling(&mut self) {
        if self.cmd_delay > 0 {
            self.cmd_delay -= 1;
        } else {
            self.cmd_q.pop_front();
        }
        if self.dat_delay > 0 {
            self.dat_delay -= 1;
            if self.dat_delay == 0 && self.data == Data::Sending {
                self.queue_block();
            }
        } else if !self.dat_q.is_empty() {
            self.dat_q.pop_front();
            if self.dat_q.is_empty() && self.data == Data::Sending {
                // A block has gone out whole.
                if self.multi_read {
                    self.addr += 1;
                    self.dat_delay = self.multi_gap;
                } else {
                    self.data = Data::Idle;
                }
            }
        } else if self.busy > 0 {
            // Busy counts once the token is out.
            self.busy -= 1;
            if self.busy == 0 && self.multi_write {
                self.data = Data::Expecting;
            }
        }
    }

    /// A command has arrived whole: check it, then answer it.
    fn command(&mut self) {
        let bits: Vec<bool> = (0..40)
            .rev()
            .map(|i| self.cmd_bits >> (i + 8) & 1 != 0)
            .collect();
        let crc = ((self.cmd_bits >> 1) & 0x7f) as u8;
        if crc7(&bits) != crc || self.cmd_bits & 1 == 0 {
            self.bad_commands += 1;
            self.app = false;
            return;
        }
        // Nothing is heard before the clocks a card wants (issue 562).
        if self.clocks < 74 {
            self.early += 1;
            self.app = false;
            return;
        }
        let index = ((self.cmd_bits >> 40) & 0x3f) as u8;
        let arg = ((self.cmd_bits >> 8) & 0xffff_ffff) as u32;
        let app = self.app;
        self.app = false;
        let status = 0x0000_0900u32 | if app { 0x20 } else { 0 };
        match (app, index) {
            (false, 0) => {
                self.selected = false;
                self.wide = false;
                self.acmd41 = 0;
            }
            (false, 8) => self.short(8, arg & 0xfff),
            (false, 55) => {
                self.app = true;
                self.short(55, status | 0x20);
            }
            (true, 41) => {
                self.acmd41 += 1;
                let ocr = 0x40ff_8000
                    | if self.acmd41 >= 2 { 0x8000_0000 } else { 0 };
                self.respond(63, ocr, true);
            }
            (false, 2) => self.long(),
            (false, 3) => self.short(3, (u32::from(self.rca) << 16) | 0x0500),
            (false, 7) => {
                self.selected = (arg >> 16) as u16 == self.rca;
                self.short(7, status);
                self.busy = 8;
            }
            (true, 6) => {
                self.wide = arg & 3 == 2;
                self.short(6, status);
            }
            (false, 16) => self.short(16, status),
            (false, 17) | (false, 18) => {
                self.short(index, status);
                self.addr = arg;
                self.multi_read = index == 18;
                self.data = Data::Sending;
                self.dat_delay = self.ncr + 48 + self.nac;
            }
            (false, 24) | (false, 25) => {
                self.short(index, status);
                self.addr = arg;
                self.multi_write = index == 25;
                self.data = Data::Expecting;
            }
            (false, 12) => {
                self.multi_read = false;
                self.multi_write = false;
                self.data = Data::Idle;
                self.dat_q.clear();
                self.dat_delay = 0;
                self.short(12, status);
                self.busy = 8;
            }
            _ => {}
        }
    }

    /// A short response: the index and a word, with the CRC unless
    /// `nocrc`, which answers `ACMD41` with the field all ones.
    fn respond(&mut self, index: u8, word: u32, nocrc: bool) {
        let mut bits: Vec<bool> = Vec::with_capacity(48);
        bits.push(false);
        bits.push(false);
        bits.extend((0..6).rev().map(|i| index >> i & 1 != 0));
        bits.extend((0..32).rev().map(|i| word >> i & 1 != 0));
        let crc = if nocrc { 0x7f } else { crc7(&bits) };
        let crc = if self.bad_resp_crc && !nocrc {
            crc ^ 0x01
        } else {
            crc
        };
        self.bad_resp_crc = false;
        bits.extend((0..7).rev().map(|i| crc >> i & 1 != 0));
        bits.push(true);
        self.cmd_q = bits.into();
        self.cmd_delay = self.ncr;
    }

    fn short(&mut self, index: u8, word: u32) {
        self.respond(index, word, false)
    }

    /// The long response: the header, then the CID with its own CRC.
    fn long(&mut self) {
        let mut bits: Vec<bool> =
            vec![false, false, true, true, true, true, true, true];
        let mut cid = self.cid;
        if self.bad_resp_crc {
            cid[15] ^= 0x02;
            self.bad_resp_crc = false;
        }
        bits.extend(
            cid.iter()
                .flat_map(|b| (0..8).rev().map(move |i| b & (1 << i) != 0)),
        );
        self.cmd_q = bits.into();
        self.cmd_delay = self.ncr;
    }

    /// The block at `addr` goes on the lines: the start bit, the
    /// bytes, the CRC of each line, the end bit.
    fn queue_block(&mut self) {
        let at = self.addr as usize * 512;
        let block: Vec<u8> = if at + 512 <= self.blocks.len() {
            self.blocks[at..at + 512].to_vec()
        } else {
            vec![0xff; 512]
        };
        let lines = if self.wide { 4 } else { 1 };
        let mut per_line: Vec<Vec<bool>> = vec![Vec::new(); lines];
        let mut q: std::collections::VecDeque<u8> = Default::default();
        q.push_back(if self.wide { 0 } else { 0xe });
        for b in block {
            if self.wide {
                for nib in [b >> 4, b & 0xf] {
                    for (i, line) in per_line.iter_mut().enumerate() {
                        line.push(nib >> i & 1 != 0);
                    }
                    q.push_back(nib);
                }
            } else {
                for i in (0..8).rev() {
                    let bit = b >> i & 1 != 0;
                    per_line[0].push(bit);
                    q.push_back(0xe | u8::from(bit));
                }
            }
        }
        let mut crcs: Vec<u16> = per_line.iter().map(|l| crc16(l)).collect();
        if self.bad_data_crc {
            crcs[0] ^= 0x0100;
            self.bad_data_crc = false;
        }
        for i in (0..16).rev() {
            let mut nib = 0u8;
            for (l, c) in crcs.iter().enumerate() {
                nib |= u8::from(c >> i & 1 != 0) << l;
            }
            q.push_back(if self.wide { nib } else { 0xe | nib });
        }
        q.push_back(0xf);
        self.dat_q = q;
    }

    /// A block has come in whole: check each line's CRC, answer with
    /// the status token, keep the block if it was good, and be busy.
    fn received(&mut self) {
        let lines = if self.wide { 4 } else { 1 };
        let n = self.rx_nibbles.len() - 1;
        let data_n = if self.wide { 1024 } else { 4096 };
        let mut good = true;
        for l in 0..lines {
            let bits: Vec<bool> = self.rx_nibbles[..data_n]
                .iter()
                .map(|v| v >> l & 1 != 0)
                .collect();
            let mut got = 0u16;
            for v in &self.rx_nibbles[data_n..n] {
                got = (got << 1) | u16::from(v >> l & 1 != 0);
            }
            if crc16(&bits) != got {
                good = false;
            }
        }
        if good {
            let mut bytes = Vec::with_capacity(512);
            if self.wide {
                for pair in self.rx_nibbles[..data_n].chunks(2) {
                    bytes.push((pair[0] << 4) | pair[1]);
                }
            } else {
                for byte in self.rx_nibbles[..data_n].chunks(8) {
                    bytes.push(
                        byte.iter().fold(0u8, |acc, v| (acc << 1) | (v & 1)),
                    );
                }
            }
            let at = self.addr as usize * 512;
            if at + 512 <= self.blocks.len() {
                self.blocks[at..at + 512].copy_from_slice(&bytes);
            }
            self.addr += 1;
        }
        // The token: a start bit, `010` or `101`, the end bit, on the
        // first line alone; then busy.
        let token: [u8; 5] = if good {
            [0, 0, 1, 0, 1]
        } else {
            [0, 1, 0, 1, 1]
        };
        self.dat_q = token.iter().map(|b| 0xe | b).collect();
        self.dat_delay = 2;
        self.busy = self.busy_len;
        self.data = Data::Idle;
    }
}

/// The host against the card, over AXI-Lite, on one and four lines.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::axi_lite::{axi_lite, LiteAw, LiteHost, LiteW};
    use std::cell::RefCell;
    use std::rc::Rc;
    use txhdl::comp::{join2, signal, Running};

    type Host = LiteHost<32, 32, 4>;

    async fn write(h: &Host, addr: u32, data: u32) {
        let (aw, _, w, b, _) = h;
        aw.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        w.send(LiteW {
            data: U::from(data),
            strb: U::from(0xfu8),
        });
        loop {
            DefaultClock::rising().await;
            if b.recv().is_some() {
                return;
            }
        }
    }

    async fn read(h: &Host, addr: u32) -> u32 {
        let (_, ar, _, _, r) = h;
        ar.send(LiteAw {
            addr: U::from(addr),
            prot: U::from(0u8),
        });
        loop {
            DefaultClock::rising().await;
            if let Some(got) = r.recv() {
                return got.data.raw() as u32;
            }
        }
    }

    /// A command: the argument, the word, then the wait for done, and
    /// the status as the host left it, cleared afterwards.
    async fn command(h: &Host, index: u32, arg: u32, flags: u32) -> u32 {
        write(h, regs::arg, arg).await;
        write(h, regs::cmd, index | flags).await;
        loop {
            let s = read(h, regs::status).await;
            if s & STATUS_DONE != 0 {
                write(h, regs::status, STATUS_DONE).await;
                return s;
            }
        }
    }

    /// The faults in a status word.
    fn faults(s: u32) -> u32 {
        s & (STATUS_RTIMEOUT | STATUS_RCRC | STATUS_DTIMEOUT | STATUS_DCRC)
    }

    /// The card brought up: reset, its voltage asked, its readiness
    /// waited for, its identity and address taken, selected, and put
    /// on four lines if `wide`. Answers the CID's four words.
    async fn bring_up(h: &Host, wide: bool) -> [u32; 4] {
        assert_eq!(faults(command(h, 0, 0, CMD_CLOCKS).await), 0, "clocks");
        assert_eq!(faults(command(h, 0, 0, 0).await), 0, "CMD0");
        let s = command(h, 8, 0x1aa, CMD_SHORT).await;
        assert_eq!(faults(s), 0, "CMD8: {s:#x}");
        assert_eq!(read(h, regs::resp0).await & 0xfff, 0x1aa, "CMD8 echoes");
        let mut ready = false;
        for _ in 0..4 {
            assert_eq!(faults(command(h, 55, 0, CMD_SHORT).await), 0, "CMD55");
            let s = command(h, 41, 0x4030_0000, CMD_SHORT | CMD_NOCRC).await;
            assert_eq!(faults(s), 0, "ACMD41: {s:#x}");
            if read(h, regs::resp0).await & 0x8000_0000 != 0 {
                ready = true;
                break;
            }
        }
        assert!(ready, "the card came ready");
        let s = command(h, 2, 0, CMD_LONG).await;
        assert_eq!(faults(s), 0, "CMD2: {s:#x}");
        let cid = [
            read(h, regs::resp3).await,
            read(h, regs::resp2).await,
            read(h, regs::resp1).await,
            read(h, regs::resp0).await,
        ];
        let s = command(h, 3, 0, CMD_SHORT).await;
        assert_eq!(faults(s), 0, "CMD3: {s:#x}");
        let rca = read(h, regs::resp0).await >> 16;
        assert_eq!(rca, 0x1234, "the address the card gave");
        let s = command(h, 7, rca << 16, CMD_SHORT | CMD_BUSY).await;
        assert_eq!(faults(s), 0, "CMD7: {s:#x}");
        if wide {
            assert_eq!(faults(command(h, 55, rca << 16, CMD_SHORT).await), 0);
            let s = command(h, 6, 2, CMD_SHORT).await;
            assert_eq!(faults(s), 0, "ACMD6: {s:#x}");
            let ctrl = read(h, regs::ctrl).await;
            write(h, regs::ctrl, ctrl | CTRL_WIDE).await;
        }
        assert_eq!(faults(command(h, 16, 512, CMD_SHORT).await), 0, "CMD16");
        cid
    }

    /// The buffer read out, 128 words.
    async fn take(h: &Host) -> Vec<u32> {
        let mut words = Vec::with_capacity(WORDS);
        for _ in 0..WORDS {
            words.push(read(h, regs::data).await);
        }
        words
    }

    /// The buffer filled with 128 words.
    async fn fill(h: &Host, words: &[u32]) {
        write(h, regs::ctrl, read(h, regs::ctrl).await | CTRL_CLEAR).await;
        for w in words {
            write(h, regs::data, *w).await;
        }
    }

    /// A block as the card holds it, as words in transmission order.
    fn block_words(card: &SdCard, at: usize) -> Vec<u32> {
        card.blocks[at * 512..at * 512 + 512]
            .chunks(4)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    /// Run `client` against the host wired to `card`, and answer the
    /// card as the run left it.
    fn run<F>(card: SdCard, client: impl FnOnce(Host) -> F) -> SdCard
    where
        F: std::future::Future<Output = ()>,
    {
        let link = axi_lite::<32, 32, 4>();
        let bus: LitePort<32, 32, 4> = link.per.into();
        let (cmd_in_o, cmd_in) = signal::<Bit, DefaultClock>();
        let (dat_in_o, dat_in) = signal::<U<4>, DefaultClock>();
        let (sclk_o, sclk) = signal::<Bit, DefaultClock>();
        let (cmd_out_o, cmd_out) = signal::<Bit, DefaultClock>();
        let (cmd_oe_o, cmd_oe) = signal::<Bit, DefaultClock>();
        let (dat_out_o, dat_out) = signal::<U<4>, DefaultClock>();
        let (dat_oe_o, dat_oe) = signal::<Bit, DefaultClock>();
        let (irq_o, _irq) = signal::<Bit, DefaultClock>();
        let done = Rc::new(RefCell::new(false));
        let d = done.clone();
        let body = client(link.host);
        let client = async move {
            body.await;
            *d.borrow_mut() = true;
        };
        let mut host = Sd::default();
        let mut sim = Running::new(join2(
            client,
            host.run(
                bus,
                (
                    cmd_in, dat_in, sclk_o, cmd_out_o, cmd_oe_o, dat_out_o,
                    dat_oe_o, irq_o,
                ),
            ),
        ));
        let mut card = card;
        cmd_in_o.set(Bit::One);
        dat_in_o.set(U::<4>::from(0xfu8));
        for _ in 0..4_000_000 {
            sim.cycle();
            let host_cmd = cmd_oe.get().to_bool();
            let host_dat = dat_oe.get().to_bool();
            let cmd = cmd_out.get().to_bool();
            let dat = dat_out.get().raw() as u8;
            card.step(sclk.get().to_bool(), host_cmd, cmd, host_dat, dat);
            // The lines: whoever drives, and high when nobody does.
            cmd_in_o.set(Bit::from_bool(if host_cmd {
                cmd
            } else {
                card.cmd_out()
            }));
            dat_in_o.set(U::<4>::from(if host_dat {
                dat
            } else {
                card.dat_out()
            }));
            if *done.borrow() {
                return card;
            }
        }
        panic!("the client did not finish");
    }

    /// A fast clock for the tests: a half of a card clock in two
    /// cycles.
    const DIV: u32 = 1;

    #[test]
    fn the_card_comes_up_and_says_who_it_is() {
        let card = SdCard::default();
        let cid = card.cid;
        let card = run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            let got = bring_up(&h, false).await;
            let want: Vec<u32> = cid
                .chunks(4)
                .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            assert_eq!(got.to_vec(), want, "the CID, as the card holds it");
        });
        assert_eq!(card.bad_commands, 0, "every command's CRC was right");
    }

    #[test]
    fn a_block_is_read_on_one_line() {
        let card = SdCard::default();
        let want = block_words(&card, 3);
        run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            bring_up(&h, false).await;
            let s = command(&h, 17, 3, CMD_SHORT | CMD_READ).await;
            assert_eq!(faults(s), 0, "CMD17: {s:#x}");
            assert_eq!(take(&h).await, want, "block 3");
        });
    }

    #[test]
    fn a_block_is_written_and_read_back_on_four_lines() {
        let card = SdCard::default();
        let words: Vec<u32> = (0..WORDS as u32)
            .map(|i| i.wrapping_mul(0x9e37_79b9))
            .collect();
        let sent = words.clone();
        let card = run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            bring_up(&h, true).await;
            fill(&h, &words).await;
            let s = command(&h, 24, 5, CMD_SHORT | CMD_WRITE).await;
            assert_eq!(faults(s), 0, "CMD24: {s:#x}");
            assert_eq!((s >> 6) & 7, 2, "the card accepted the block");
            let s = command(&h, 17, 5, CMD_SHORT | CMD_READ).await;
            assert_eq!(faults(s), 0, "CMD17: {s:#x}");
            assert_eq!(take(&h).await, words, "block 5 as written");
        });
        assert_eq!(block_words(&card, 5), sent, "and as the card holds it");
    }

    #[test]
    fn several_blocks_go_each_way() {
        let card = SdCard::default();
        let b1 = block_words(&card, 1);
        let b2 = block_words(&card, 2);
        let mut sent: Vec<Vec<u32>> = Vec::new();
        for k in 0..2u32 {
            sent.push((0..WORDS as u32).map(|i| (i + 1) * (k + 3)).collect());
        }
        let out = sent.clone();
        let card = run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            bring_up(&h, true).await;
            // Two blocks read: the command with the first, a
            // data-only transfer for the second, and the stop.
            let s = command(&h, 18, 1, CMD_SHORT | CMD_READ).await;
            assert_eq!(faults(s), 0, "CMD18: {s:#x}");
            assert_eq!(take(&h).await, b1, "block 1");
            let s = command(&h, 0, 0, CMD_DATA_ONLY | CMD_READ).await;
            assert_eq!(faults(s), 0, "the second block: {s:#x}");
            assert_eq!(take(&h).await, b2, "block 2");
            let s = command(&h, 12, 0, CMD_SHORT | CMD_BUSY).await;
            assert_eq!(faults(s), 0, "CMD12: {s:#x}");
            // Two blocks written the same way.
            fill(&h, &out[0]).await;
            let s = command(&h, 25, 6, CMD_SHORT | CMD_WRITE).await;
            assert_eq!(faults(s), 0, "CMD25: {s:#x}");
            fill(&h, &out[1]).await;
            let s = command(&h, 0, 0, CMD_DATA_ONLY | CMD_WRITE).await;
            assert_eq!(faults(s), 0, "the second block out: {s:#x}");
            let s = command(&h, 12, 0, CMD_SHORT | CMD_BUSY).await;
            assert_eq!(faults(s), 0, "CMD12: {s:#x}");
        });
        assert_eq!(block_words(&card, 6), sent[0], "block 6");
        assert_eq!(block_words(&card, 7), sent[1], "block 7");
    }

    #[test]
    fn a_wrong_crc_is_reported_and_a_silent_card_times_out() {
        let card = SdCard {
            bad_resp_crc: true,
            ..SdCard::default()
        };
        let card = run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            command(&h, 0, 0, CMD_CLOCKS).await;
            let s = command(&h, 8, 0x1aa, CMD_SHORT).await;
            assert_eq!(faults(s), STATUS_RCRC, "the spoiled response: {s:#x}");
            // The next is right again.
            let s = command(&h, 8, 0x1aa, CMD_SHORT).await;
            assert_eq!(faults(s), 0, "and the next: {s:#x}");
            // A command the card does not answer.
            let s = command(&h, 13, 0, CMD_SHORT).await;
            assert_eq!(faults(s), STATUS_RTIMEOUT, "no response: {s:#x}");
        });
        assert_eq!(card.bad_commands, 0);
        let card = SdCard {
            bad_data_crc: true,
            ..SdCard::default()
        };
        run(card, |h| async move {
            write(&h, regs::ctrl, DIV).await;
            bring_up(&h, true).await;
            let s = command(&h, 17, 2, CMD_SHORT | CMD_READ).await;
            assert_eq!(faults(s), STATUS_DCRC, "the spoiled block: {s:#x}");
            let s = command(&h, 17, 2, CMD_SHORT | CMD_READ).await;
            assert_eq!(faults(s), 0, "and the next: {s:#x}");
        });
    }

    /// A card hears nothing before its clocks: without them `CMD0` and
    /// `CMD8` go unanswered, and with them the same two commands are
    /// answered (issue 562).
    #[test]
    fn a_card_wants_its_clocks_first() {
        let card = run(SdCard::default(), |h| async move {
            write(&h, regs::ctrl, DIV).await;
            command(&h, 0, 0, 0).await;
            let s = command(&h, 8, 0x1aa, CMD_SHORT).await;
            assert_eq!(faults(s), STATUS_RTIMEOUT, "unheard: {s:#x}");
            let s = command(&h, 0, 0, CMD_CLOCKS).await;
            assert_eq!(faults(s), 0, "the clocks: {s:#x}");
            command(&h, 0, 0, 0).await;
            let s = command(&h, 8, 0x1aa, CMD_SHORT).await;
            assert_eq!(faults(s), 0, "heard now: {s:#x}");
        });
        assert_eq!(card.early, 2, "the two commands before the clocks");
        assert!(
            card.clocks >= 74,
            "and the clocks were given: {}",
            card.clocks
        );
    }

    /// The CRCs against known values: the CRC-7 of `CMD0` with a zero
    /// argument is `0x4a`, and the CRC-16 of 512 bytes of `0xff` is
    /// `0x7fa1`, both as the specification's examples have them.
    #[test]
    fn the_crcs_match_the_specification() {
        let mut bits = vec![false, true];
        bits.extend([false; 6]);
        bits.extend([false; 32]);
        assert_eq!(crc7(&bits), 0x4a);
        let ones = vec![true; 4096];
        assert_eq!(crc16(&ones), 0x7fa1);
    }
}
