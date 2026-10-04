// SPDX-License-Identifier: Apache-2.0
//! A stock OpenOCD on the simulated board, through the RISC-V debug
//! transport behind `BSCANE2` (issue 154).
//!
//! OpenOCD 0.12.0, unmodified and pinned (`//third_party/openocd`),
//! talks to this test over its `remote_bitbang` adapter. The test is
//! the cable and the device's TAP: a model of the Artix-7's TAP, its
//! instruction register six bits, whose `USER4` drives the board's
//! `BSCANE2` pins as the device would, and behind them the whole board,
//! simulated. OpenOCD is told `riscv use_bscan_tunnel 5`, exactly as it
//! will be on the board, and does what a session does first: finds the
//! device, examines the hart, halts it, reads `pc`, reads memory by
//! system bus access, and resumes.
use remote_bitbang::{serve, Openocd, Pins};
use std::net::TcpListener;
use std::time::Duration;
use txhdl::comp::{chan, pad, signal, Clock, DefaultClock, Out, Running, Unit};
use txhdl::types::{Bit, U};
use txhdl_parts::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
use txhdl_parts::bus::axi_pins::AxiHostPins;
use txhdl_parts::dtm::Tck;
use txhdl_parts::eth::EthByte;
use vreteno32::board::{Board, BoardIn, BoardOut};
use vreteno32::core::Vreteno;
use vreteno32::dmem::Dmem;
use vreteno32::rom::Rom;

/// The Artix-7 XC7A200T's identity, which OpenOCD is told to expect.
const IDCODE: u32 = 0x1363_6093;
/// The 7-series instruction register: six bits, `IDCODE` 0x09 and the
/// fourth user chain, where the transport is, 0x23.
const IR_LEN: u32 = 6;
const IR_IDCODE: u32 = 0x09;
const IR_USER4: u32 = 0x23;

/// The words the test puts at the start of the data memory, which
/// OpenOCD reads back by system bus access.
const WORDS: [u32; 4] = [0xdead_beef, 0x0123_4567, 0x89ab_cdef, 0x5a5a_a5a5];

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Reset,
    Idle,
    SelectDr,
    CaptureDr,
    ShiftDr,
    Exit1Dr,
    PauseDr,
    Exit2Dr,
    UpdateDr,
    SelectIr,
    CaptureIr,
    ShiftIr,
    Exit1Ir,
    PauseIr,
    Exit2Ir,
    UpdateIr,
}

fn next(s: State, tms: bool) -> State {
    use State::*;
    match (s, tms) {
        (Reset, false) => Idle,
        (Reset, true) => Reset,
        (Idle, false) => Idle,
        (Idle, true) => SelectDr,
        (SelectDr, false) => CaptureDr,
        (SelectDr, true) => SelectIr,
        (CaptureDr, false) | (ShiftDr, false) | (Exit2Dr, false) => ShiftDr,
        (CaptureDr, true) | (ShiftDr, true) => Exit1Dr,
        (Exit1Dr, false) | (PauseDr, false) => PauseDr,
        (Exit1Dr, true) | (Exit2Dr, true) => UpdateDr,
        (PauseDr, true) => Exit2Dr,
        (UpdateDr, false) | (UpdateIr, false) => Idle,
        (UpdateDr, true) | (UpdateIr, true) => SelectDr,
        (SelectIr, false) => CaptureIr,
        (SelectIr, true) => Reset,
        (CaptureIr, false) | (ShiftIr, false) | (Exit2Ir, false) => ShiftIr,
        (CaptureIr, true) | (ShiftIr, true) => Exit1Ir,
        (Exit1Ir, false) | (PauseIr, false) => PauseIr,
        (Exit1Ir, true) | (Exit2Ir, true) => UpdateIr,
        (PauseIr, true) => Exit2Ir,
    }
}

/// The simulation, whatever its future: one tick at a time.
trait Sim {
    fn tick(&mut self);
}

impl<F: std::future::Future<Output = ()>> Sim for Running<F> {
    fn tick(&mut self) {
        self.step();
    }
}

/// The device's TAP in front of the board. Instructions other than
/// `USER4` are its own, `IDCODE` and bypass, as the model in
/// `//tools/openocd` has them; `USER4` drives `BSCANE2`'s pins, which
/// the board's transport samples on each rise of the cable's clock.
struct Rig {
    sim: Box<dyn Sim>,
    state: State,
    tck: bool,
    ir: u32,
    shift: u64,
    len: u32,
    sel: Out<Bit, Tck>,
    shift_o: Out<Bit, Tck>,
    capture: Out<Bit, Tck>,
    update: Out<Bit, Tck>,
    tdi: Out<Bit, Tck>,
    reset: Out<Bit, Tck>,
    /// The transport's TDO register, which the device's TAP presents
    /// between edges. Not the port: a port carries, at an edge, the value
    /// it had going into that edge, which is how the trace and the
    /// netlist agree, and between edges the pin is the register.
    out: txhdl::comp::Reg<Bit, Tck>,
    /// The tick the simulation is at.
    t: u64,
    /// A JTAG-to-AXI master on the board's `jtag_` pins, when a test
    /// wants the second host's other half busy too.
    axi: Option<Reader>,
}

/// The JTAG-to-AXI master, reading: one read in flight at a time, each
/// of `WORDS` in turn, for ever, the way `board_test`'s master drives
/// its pins before an edge and looks at them after it.
struct Reader {
    araddr: Out<U<32>, DefaultClock>,
    arvalid: Out<Bit, DefaultClock>,
    rready: Out<Bit, DefaultClock>,
    arready: txhdl::comp::In<Bit, DefaultClock>,
    rvalid: txhdl::comp::In<Bit, DefaultClock>,
    rdata: txhdl::comp::In<U<32>, DefaultClock>,
    /// Which word is asked for next, and whether its address went.
    at: usize,
    asked: bool,
    /// The address offered on the edge just gone: what `arready` answers.
    offered: bool,
    /// What came back, and the tick it came at.
    got: Vec<(u32, u64)>,
}

impl Reader {
    fn drive(&mut self) {
        let addr = 0x1000 + 4 * (self.at % WORDS.len()) as u32;
        self.araddr.set(U::from(addr));
        self.offered = !self.asked;
        self.arvalid.set(Bit::from_bool(self.offered));
        self.rready.set(Bit::One);
    }

    fn observe(&mut self, t: u64) {
        if self.offered && self.arready.get() == Bit::One {
            self.asked = true;
            self.offered = false;
        }
        if self.asked && self.rvalid.get() == Bit::One {
            self.got.push((self.rdata.get().raw() as u32, t));
            self.at += 1;
            self.asked = false;
        }
    }
}

impl Rig {
    /// One tick, with the JTAG-to-AXI master driven before the board's
    /// clock rises and looked at after its cycle ends.
    fn tick(&mut self) {
        if self.t.is_multiple_of(2) {
            if let Some(a) = self.axi.as_mut() {
                a.drive();
            }
        }
        self.sim.tick();
        self.t += 1;
        if self.t.is_multiple_of(2) {
            let t = self.t;
            if let Some(a) = self.axi.as_mut() {
                a.observe(t);
            }
        }
    }

    /// One rise of TCK. `BSCANE2`'s pins are the state the TAP is in as
    /// the edge comes, set half a cable clock before the board samples
    /// them, and the board runs one cable clock; then the TAP moves.
    fn rise(&mut self, tms: bool, tdi: bool) {
        use State::*;
        let user = self.ir == IR_USER4;
        self.sel.set(Bit::from_bool(user));
        self.capture.set(Bit::from_bool(self.state == CaptureDr));
        self.shift_o.set(Bit::from_bool(self.state == ShiftDr));
        self.update.set(Bit::from_bool(self.state == UpdateDr));
        self.reset.set(Bit::from_bool(self.state == Reset));
        self.tdi.set(Bit::from_bool(tdi));
        for _ in 0..Tck::PERIOD {
            self.tick();
        }
        if matches!(self.state, ShiftDr | ShiftIr) {
            self.shift = (self.shift >> 1) | (u64::from(tdi) << (self.len - 1));
        }
        self.state = next(self.state, tms);
        match self.state {
            Reset => self.ir = IR_IDCODE,
            CaptureIr => {
                self.shift = 0b000001;
                self.len = IR_LEN;
            }
            CaptureDr => {
                if self.ir == IR_IDCODE {
                    self.shift = u64::from(IDCODE);
                    self.len = 32;
                } else {
                    self.shift = 0;
                    self.len = 1;
                }
            }
            UpdateIr => self.ir = (self.shift & ((1 << IR_LEN) - 1)) as u32,
            _ => {}
        }
    }
}

impl Pins for Rig {
    fn pins(&mut self, tck: bool, tms: bool, tdi: bool) {
        if tck && !self.tck {
            self.rise(tms, tdi);
        }
        self.tck = tck;
    }

    fn tdo(&mut self) -> bool {
        match self.state {
            State::ShiftDr if self.ir == IR_USER4 => self.out.get() == Bit::One,
            State::ShiftDr | State::ShiftIr => self.shift & 1 != 0,
            _ => false,
        }
    }

    fn reset(&mut self, trst: bool, _srst: bool) {
        if trst {
            self.state = State::Reset;
            self.ir = IR_IDCODE;
        }
    }
}

/// A program that never ends by itself: a count, round and round, so
/// that the core is running when OpenOCD comes to halt it.
fn spin() -> Vec<u32> {
    use vreteno32::isa::{addi, blt};
    let mut a = vreteno32::program::Asm::default();
    a.emit(addi(5, 0, 0));
    a.emit(addi(6, 0, 1));
    let again = a.label();
    a.place(again);
    a.emit(addi(5, 5, 1));
    a.to(again, |off| blt(0, 6, off));
    a.words()
}

/// A program that waits, as an idle kernel does: `wfi` with no interrupt
/// enabled, so nothing ever wakes it, and a jump back for the wake that
/// never comes. A halt request is the one thing that ends the wait
/// (issue 930).
fn waiting() -> Vec<u32> {
    use vreteno32::isa::{jal, wfi};
    vec![wfi(), jal(0, -4)]
}

/// The board, running `spin`, with `WORDS` in its data memory, behind
/// the TAP; with `busy`, the JTAG-to-AXI master reads all the while.
fn rig(busy: bool) -> Rig {
    rig_with(spin(), busy)
}

/// The same, running `text`.
fn rig_with(text: Vec<u32>, busy: bool) -> Rig {
    let data: Vec<u8> = WORDS.iter().flat_map(|w| w.to_le_bytes()).collect();
    let board = Box::leak(Box::new(Board::<4> {
        cpu: Vreteno::with(&text),
        rom: Rom::with(&text),
        dmem: Dmem::with(&data),
        ..Default::default()
    }));
    let dtm_out = board.dtm.out;
    // The JTAG-to-AXI master's read pins, as handles; its burst a
    // single beat, a word wide, incrementing.
    let (araddr_o, araddr) = signal::<U<32>, DefaultClock>();
    let (arvalid_o, arvalid) = signal::<Bit, DefaultClock>();
    let (rready_o, rready) = signal::<Bit, DefaultClock>();
    let (arready_o, arready) = signal::<Bit, DefaultClock>();
    let (rvalid_o, rvalid) = signal::<Bit, DefaultClock>();
    let (rdata_o, rdata) = signal::<U<32>, DefaultClock>();
    let (arsize_o, arsize) = signal::<U<3>, DefaultClock>();
    let (arburst_o, arburst) = signal::<U<2>, DefaultClock>();
    arsize_o.set(U::from(2u8));
    arburst_o.set(U::from(1u8));
    let (rst_o, rst) = signal::<Bit, DefaultClock>();
    let (rx_o, rx) = signal::<Bit, DefaultClock>();
    let lo = || signal::<Bit, DefaultClock>().1;
    let out = || signal::<Bit, DefaultClock>().0;
    let (sel_o, sel) = signal::<Bit, Tck>();
    let (shift_o, shift) = signal::<Bit, Tck>();
    let (capture_o, capture) = signal::<Bit, Tck>();
    let (update_o, update) = signal::<Bit, Tck>();
    let (tdi_o, tdi) = signal::<Bit, Tck>();
    let (reset_o, reset) = signal::<Bit, Tck>();
    let (tdo_o, _tdo) = signal::<Bit, Tck>();
    let mut sim = Running::new(board.run(
        BoardIn {
            rst,
            irq: lo(),
            rx,
            sys_clk: lo(),
            sys_rst: lo(),
            vb: chan::<LiteB, DefaultClock>().1,
            vr: chan::<LiteR<32>, DefaultClock>().1,
            net_rx: chan::<EthByte, DefaultClock>().1,
            fl_miso: lo(),
            phy_mdio_in: lo(),
            sd_cmd_in: lo(),
            sd_dat_in: signal::<U<4>, DefaultClock>().1,
            bscan_sel: sel,
            bscan_shift: shift,
            bscan_capture: capture,
            bscan_update: update,
            bscan_tdi: tdi,
            bscan_reset: reset,
            jtag: AxiHostPins {
                awid: signal::<U<1>, DefaultClock>().1,
                awaddr: signal::<U<32>, DefaultClock>().1,
                awlen: signal::<U<8>, DefaultClock>().1,
                awsize: signal::<U<3>, DefaultClock>().1,
                awburst: signal::<U<2>, DefaultClock>().1,
                awlock: lo(),
                awcache: signal::<U<4>, DefaultClock>().1,
                awprot: signal::<U<3>, DefaultClock>().1,
                awvalid: lo(),
                wdata: signal::<U<32>, DefaultClock>().1,
                wstrb: signal::<U<4>, DefaultClock>().1,
                wlast: lo(),
                wvalid: lo(),
                bready: lo(),
                arid: signal::<U<1>, DefaultClock>().1,
                araddr,
                arlen: signal::<U<8>, DefaultClock>().1,
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
            halt: out(),
            tx: out(),
            pwm_pins: signal::<U<4>, DefaultClock>().0,
            calib: out(),
            ui_clk: out(),
            ui_rst: out(),
            ck_p: out(),
            ck_n: out(),
            mem_rst_n: out(),
            cke: out(),
            cs_n: out(),
            ras_n: out(),
            cas_n: out(),
            we_n: out(),
            row: signal::<U<15>, DefaultClock>().0,
            bank: signal::<U<3>, DefaultClock>().0,
            dm: signal::<U<4>, DefaultClock>().0,
            odt: out(),
            dq: pad::<U<32>, DefaultClock>(),
            dqs: pad::<U<4>, DefaultClock>(),
            dqs_n: pad::<U<4>, DefaultClock>(),
            vaw: chan::<LiteAw<32>, DefaultClock>().0,
            var: chan::<LiteAr<32>, DefaultClock>().0,
            vw: chan::<LiteW<32, 4>, DefaultClock>().0,
            net_tx: chan::<EthByte, DefaultClock>().0,
            jtag_awready: out(),
            jtag_wready: out(),
            jtag_bid: signal::<U<1>, DefaultClock>().0,
            jtag_bresp: signal::<U<2>, DefaultClock>().0,
            jtag_bvalid: out(),
            jtag_arready: arready_o,
            jtag_rid: signal::<U<1>, DefaultClock>().0,
            jtag_rdata: rdata_o,
            jtag_rresp: signal::<U<2>, DefaultClock>().0,
            jtag_rlast: out(),
            jtag_rvalid: rvalid_o,
            fl_cs_n: out(),
            fl_mosi: out(),
            fl_cclk: out(),
            fl_refused: out(),
            phy_mdc: out(),
            phy_mdio_out: out(),
            phy_mdio_oe: out(),
            sd_clk: out(),
            sd_cmd_out: out(),
            sd_cmd_oe: out(),
            sd_dat_out: signal::<U<4>, DefaultClock>().0,
            sd_dat_oe: out(),
            bscan_tdo: tdo_o,
        },
    ));
    // Out of reset, the serial line idle, and the cable's half period
    // run, so that every pin set from here on is set between the
    // cable clock's edges.
    rst_o.set(Bit::One);
    sim.cycle();
    rst_o.set(Bit::Zero);
    rx_o.set(Bit::One);
    for _ in 0..Tck::PERIOD / 2 {
        sim.step();
    }
    Rig {
        sim: Box::new(sim),
        state: State::Reset,
        tck: false,
        ir: IR_IDCODE,
        shift: 0,
        len: 1,
        sel: sel_o,
        shift_o,
        capture: capture_o,
        update: update_o,
        tdi: tdi_o,
        reset: reset_o,
        out: dtm_out,
        t: Tck::PERIOD / 2 + 2,
        axi: busy.then_some(Reader {
            araddr: araddr_o,
            arvalid: arvalid_o,
            rready: rready_o,
            arready,
            rvalid,
            rdata,
            at: 0,
            asked: false,
            offered: false,
            got: Vec::new(),
        }),
    }
}

fn openocd() -> std::path::PathBuf {
    std::env::var_os("OPENOCD")
        .expect("OPENOCD names the binary")
        .into()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::var_os("TEST_TMPDIR")
        .map_or_else(std::env::temp_dir, Into::into);
    let dir = d.join(format!("openocd-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn openocd_halts_the_core_reads_pc_and_memory_and_resumes() {
    let mut rig = rig(false);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        "adapter driver remote_bitbang\n\
         remote_bitbang host 127.0.0.1\n\
         remote_bitbang port {port}\n\
         transport select jtag\n\
         jtag newtap xc7 tap -irlen {IR_LEN} -expected-id {IDCODE:#010x}\n\
         target create xc7.cpu riscv -chain-position xc7.tap\n\
         riscv use_bscan_tunnel 5\n\
         riscv set_mem_access sysbus\n\
         riscv set_command_timeout_sec 120\n\
         init\n\
         halt\n\
         echo \"state [xc7.cpu curstate]\"\n\
         echo [capture {{reg pc}}]\n\
         echo [capture {{mdw 0x1000 4}}]\n\
         step\n\
         echo \"stepped [capture {{reg pc}}]\"\n\
         mww 0x1010 0x12345678\n\
         echo [capture {{mdw 0x1010}}]\n\
         resume\n\
         echo \"state [xc7.cpu curstate]\"\n\
         shutdown\n"
    );
    let run = Openocd::start(&openocd(), &script, &tmp("halt")).unwrap();
    let served = serve(listener, &mut rig, Duration::from_secs(120)).unwrap();
    let done = run.finish(Duration::from_secs(60)).unwrap();
    let log = &done.log;
    // The session, kept in the test's log whether it passes or not.
    eprintln!("{log}");
    assert!(
        served.rises > 0,
        "OpenOCD clocked the TAP: {served:?}\n{log}"
    );
    assert!(log.contains("tap/device found: 0x13636093"), "{log}");
    assert!(
        log.contains("Examined RISC-V core"),
        "the hart examined:\n{log}"
    );
    assert!(log.contains("state halted"), "halted:\n{log}");
    assert!(log.contains("pc (/32): 0x"), "pc read:\n{log}");
    assert!(
        log.contains("stepped pc (/32): 0x"),
        "single-stepped:\n{log}"
    );
    assert!(
        log.contains("0x00001010: 12345678"),
        "a word written by system bus access and read back:\n{log}"
    );
    assert!(log.contains("state running"), "resumed:\n{log}");
    assert!(
        log.contains("0x00001000: deadbeef 01234567 89abcdef 5a5aa5a5"),
        "memory read by system bus access:\n{log}"
    );
    assert!(
        done.status.is_some_and(|s| s.success()),
        "OpenOCD ended well: {:?}\n{log}",
        done.status
    );
}

/// What OpenOCD's bitbang driver does for one bit: TCK low with TMS
/// and TDI, TDO read, then TCK high.
fn bit(rig: &mut Rig, tms: bool, tdi: bool) -> bool {
    rig.pins(false, tms, tdi);
    let got = rig.tdo();
    rig.pins(true, tms, tdi);
    got
}

/// From Run-Test/Idle through a scan of `out` and back, as OpenOCD's
/// bitbang driver moves the TAP: `ir` picks the instruction path.
fn scan(rig: &mut Rig, ir: bool, out: &[bool]) -> Vec<bool> {
    bit(rig, true, false); // Select-DR
    if ir {
        bit(rig, true, false); // Select-IR
    }
    bit(rig, false, false); // Capture
    bit(rig, false, false); // Shift
    let mut got = Vec::new();
    for (i, &b) in out.iter().enumerate() {
        got.push(bit(rig, i + 1 == out.len(), b));
    }
    bit(rig, true, false); // Update
    bit(rig, false, false); // Run-Test/Idle
    got
}

fn bits(v: u64, n: u32) -> Vec<bool> {
    (0..n).map(|i| (v >> i) & 1 == 1).collect()
}

fn value(b: &[bool]) -> u64 {
    b.iter().rev().fold(0, |a, &x| (a << 1) | x as u64)
}

/// The scans OpenOCD makes to read `dtmcs` through the tunnel, driven
/// bit by bit as its bitbang driver drives them, without OpenOCD: the
/// fast check that runs in every suite.
#[test]
fn dtmcs_reads_through_the_tunnel_as_openocd_scans_it() {
    let mut rig = rig(false);
    for _ in 0..5 {
        bit(&mut rig, true, false);
    }
    bit(&mut rig, false, false);
    scan(&mut rig, true, &bits(u64::from(IR_USER4), IR_LEN));
    // The tunneled instruction scan selecting `dtmcs`.
    let mut v = vec![false];
    v.extend(bits(5, 7));
    v.extend(bits(0x10, 5));
    v.extend([false; 3]);
    scan(&mut rig, false, &v);
    // The tunneled data scan of 32 bits, read from the second bit of
    // its payload on.
    let mut v = vec![true];
    v.extend(bits(32, 7));
    v.extend(bits(0, 33));
    v.extend([false; 3]);
    let got = scan(&mut rig, false, &v);
    let dtmcs = value(&got[9..41]) as u32;
    assert_eq!(
        dtmcs & 0xf,
        1,
        "version 1: {dtmcs:#x}, bits {:?}",
        &got[8..42]
    );
    assert_eq!((dtmcs >> 4) & 0x3f, 7, "abits 7: {dtmcs:#x}");
}

/// Run-Test/Idle for `n` clocks, as OpenOCD waits between scans.
fn idle(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        bit(rig, false, false);
    }
}

/// One tunneled `dmi` scan: what it sends, and the op and word of the
/// access before it, which the scan reads back.
fn dmi(rig: &mut Rig, a: u32, d: u32, op: u32) -> (u32, u32) {
    let v = (u64::from(a) << 34) | (u64::from(d) << 2) | u64::from(op);
    let mut s = vec![true];
    s.extend(bits(41, 7));
    s.extend(bits(v, 41));
    s.push(false);
    s.extend([false; 3]);
    let got = scan(rig, false, &s);
    let r = value(&got[9..50]);
    ((r & 3) as u32, ((r >> 2) & 0xffff_ffff) as u32)
}

/// The JTAG-to-AXI master and the transport share the arbiter's
/// second host through an `Arbiter2` (issue 154). With the master
/// reading the data memory without a pause, the transport reads it too,
/// by system bus access: both get the right words, and the master's
/// reads go on while the transport's is in flight, so neither waits
/// for the other to stop.
#[test]
fn the_jtag_master_and_the_transport_take_turns_on_one_host() {
    let mut rig = rig(true);
    for _ in 0..5 {
        bit(&mut rig, true, false);
    }
    bit(&mut rig, false, false);
    scan(&mut rig, true, &bits(u64::from(IR_USER4), IR_LEN));
    // The tunneled instruction scan selecting `dmi`.
    let mut v = vec![false];
    v.extend(bits(5, 7));
    v.extend(bits(0x11, 5));
    v.extend([false; 3]);
    scan(&mut rig, false, &v);
    // `sbcs`: `sbreadonaddr`, 32-bit access.
    dmi(&mut rig, 0x38, (1 << 20) | (2 << 17), 2);
    idle(&mut rig, 8);
    let before = rig.t;
    dmi(&mut rig, 0x39, 0x1004, 2);
    idle(&mut rig, 8);
    dmi(&mut rig, 0x3c, 0, 1);
    idle(&mut rig, 8);
    let (op, word) = dmi(&mut rig, 0, 0, 0);
    let after = rig.t;
    assert_eq!((op, word), (0, WORDS[1]), "the transport's system bus read");
    let got = &rig.axi.as_ref().unwrap().got;
    assert!(got.len() > 8, "the master kept reading: {}", got.len());
    for (i, (w, _)) in got.iter().enumerate() {
        assert_eq!(*w, WORDS[i % WORDS.len()], "the master's read {i}");
    }
    let during = got
        .iter()
        .filter(|(_, t)| (before..after).contains(t))
        .count();
    eprintln!(
        "the master read {} words, {during} of them between ticks {before} \
         and {after}",
        got.len()
    );
    assert!(
        during > 2,
        "the master's reads went on while the transport's was in flight: \
         {during} of {} between ticks {before} and {after}",
        got.len()
    );
}

fn gdb() -> std::path::PathBuf {
    std::env::var_os("GDB")
        .expect("GDB names the binary")
        .into()
}

/// gdb-multiarch (`//third_party/gdb`) through OpenOCD's gdb server, on
/// the same simulated board (issue 872). gdb loads `gdbprobe`'s ELF
/// into the DDR3 by system bus access, puts a breakpoint on `reached`,
/// and continues; the core runs the program, meets the `ebreak` gdb put
/// there, and stops in debug mode. At the breakpoint the sum of 0 to 9
/// is in `a0`, the argument, and in `SUM`, in memory.
///
/// OpenOCD and gdb are two processes and the board is this thread, so
/// gdb is started from a second thread once OpenOCD's log says its gdb
/// port is open, and ends the session with `monitor shutdown`.
#[test]
fn gdb_loads_a_program_and_stops_at_a_breakpoint() {
    let mut rig = rig(false);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let gport = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let script = format!(
        "adapter driver remote_bitbang\n\
         remote_bitbang host 127.0.0.1\n\
         remote_bitbang port {port}\n\
         transport select jtag\n\
         jtag newtap xc7 tap -irlen {IR_LEN} -expected-id {IDCODE:#010x}\n\
         target create xc7.cpu riscv -chain-position xc7.tap\n\
         riscv use_bscan_tunnel 5\n\
         riscv set_mem_access sysbus\n\
         riscv set_command_timeout_sec 120\n\
         bindto 127.0.0.1\n\
         gdb_port {gport}\n\
         tcl_port disabled\n\
         telnet_port disabled\n\
         init\n"
    );
    let dir = tmp("gdb");
    let elf = std::env::var("GDBPROBE").expect("GDBPROBE names the ELF");
    let commands = format!(
        "set confirm off\n\
         set pagination off\n\
         set architecture riscv:rv32\n\
         set remotetimeout 300\n\
         file {elf}\n\
         target extended-remote 127.0.0.1:{gport}\n\
         load\n\
         break reached\n\
         continue\n\
         printf \"stopped at %#x\\n\", $pc\n\
         printf \"a0 %d\\n\", $a0\n\
         printf \"SUM %d\\n\", *(unsigned int *)&SUM\n\
         monitor shutdown\n"
    );
    let cmds = dir.join("gdb.cmds");
    std::fs::write(&cmds, commands).unwrap();
    let run = Openocd::start(&openocd(), &script, &dir).unwrap();
    let log_path = dir.join("openocd.log");
    let listening = format!("Listening on port {gport} for gdb connections");
    let session = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(300);
        while !std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .contains(&listening)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "OpenOCD's gdb port never opened"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let out = std::process::Command::new(gdb())
            .args(["-nx", "-batch", "-x"])
            .arg(&cmds)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr)
    });
    let served = serve(listener, &mut rig, Duration::from_secs(300)).unwrap();
    let said = session.join().unwrap();
    let done = run.finish(Duration::from_secs(60)).unwrap();
    let log = &done.log;
    // Both sides of the session, kept in the test's log either way.
    eprintln!("--- gdb\n{said}\n--- OpenOCD\n{log}");
    assert!(served.rises > 0, "OpenOCD clocked the TAP: {served:?}");
    assert!(said.contains("Loading section .text"), "loaded:\n{said}");
    assert!(
        said.contains("Breakpoint 1, ") && said.contains("in reached"),
        "stopped at the breakpoint:\n{said}"
    );
    assert!(said.contains("a0 45\n"), "the argument:\n{said}");
    assert!(said.contains("SUM 45\n"), "the word in memory:\n{said}");
}

/// A core waiting in `wfi`, with nothing to wake it, halts when
/// OpenOCD asks (issue 930): the specification ends the wait on a halt
/// request. Zephyr idles in `wfi`, and before this OpenOCD's examine,
/// which halts the hart first, gave up with "unable to halt hart 0".
/// The `wfi` has retired, so the hart halts on the jump after it; and
/// once resumed it waits again.
#[test]
fn openocd_halts_a_core_waiting_in_wfi() {
    let mut rig = rig_with(waiting(), false);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        "adapter driver remote_bitbang\n\
         remote_bitbang host 127.0.0.1\n\
         remote_bitbang port {port}\n\
         transport select jtag\n\
         jtag newtap xc7 tap -irlen {IR_LEN} -expected-id {IDCODE:#010x}\n\
         target create xc7.cpu riscv -chain-position xc7.tap\n\
         riscv use_bscan_tunnel 5\n\
         riscv set_mem_access sysbus\n\
         riscv set_command_timeout_sec 120\n\
         init\n\
         halt\n\
         echo \"state [xc7.cpu curstate]\"\n\
         echo [capture {{reg pc}}]\n\
         resume\n\
         echo \"state [xc7.cpu curstate]\"\n\
         shutdown\n"
    );
    let run = Openocd::start(&openocd(), &script, &tmp("wfi")).unwrap();
    let served = serve(listener, &mut rig, Duration::from_secs(120)).unwrap();
    let done = run.finish(Duration::from_secs(60)).unwrap();
    let log = &done.log;
    eprintln!("{log}");
    assert!(served.rises > 0, "OpenOCD clocked the TAP: {served:?}\n{log}");
    assert!(
        !log.contains("unable to halt"),
        "a core in wfi refused the halt:\n{log}"
    );
    assert!(
        log.contains("Examined RISC-V core"),
        "the hart examined:\n{log}"
    );
    assert!(log.contains("state halted"), "halted:\n{log}");
    assert!(
        log.contains("pc (/32): 0x00000004"),
        "halted on the jump after the wfi:\n{log}"
    );
    assert!(log.contains("state running"), "resumed:\n{log}");
    assert!(
        done.status.is_some_and(|s| s.success()),
        "OpenOCD ended well: {:?}\n{log}",
        done.status
    );
}
