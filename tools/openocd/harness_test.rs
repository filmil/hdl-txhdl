// SPDX-License-Identifier: Apache-2.0
//! The pinned OpenOCD against a model of a JTAG TAP served over
//! `remote_bitbang` (issue 853): it is the version it should be, it
//! connects, and it scans the chain and finds the TAP's identity, or
//! says when the identity is wrong. The model is the smallest TAP that
//! answers a scan, so the harness is proven before anything real is put
//! behind it.
use remote_bitbang::{serve, Openocd, Pins};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The XC7A200T's identity, the board's part.
const IDCODE: u32 = 0x1363_6093;
/// The 7-series instruction register is six bits; IDCODE is 0x09 and
/// all ones is BYPASS.
const IR_LEN: u32 = 6;
const IR_IDCODE: u32 = 0x09;

fn openocd() -> PathBuf {
    PathBuf::from(std::env::var("OPENOCD").expect("OPENOCD names the binary"))
}

/// A directory of its own for each test, since the tests run at once
/// and each writes a script and a log.
fn tmp(name: &str) -> PathBuf {
    let d =
        PathBuf::from(std::env::var("TEST_TMPDIR").unwrap_or("/tmp".into()));
    let dir = d.join(format!("openocd-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

/// A TAP with an instruction register, BYPASS and IDCODE, and the
/// standard sixteen states, advancing on a rise of TCK.
struct Tap {
    idcode: u32,
    state: State,
    tck: bool,
    ir: u32,
    shift: u64,
    len: u32,
}

impl Tap {
    fn new(idcode: u32) -> Tap {
        Tap {
            idcode,
            state: State::Reset,
            tck: false,
            ir: IR_IDCODE,
            shift: 0,
            len: 1,
        }
    }

    fn next(&self, tms: bool) -> State {
        use State::*;
        match (self.state, tms) {
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

    fn rise(&mut self, tms: bool, tdi: bool) {
        use State::*;
        match self.state {
            ShiftDr | ShiftIr => {
                self.shift =
                    (self.shift >> 1) | (u64::from(tdi) << (self.len - 1));
            }
            _ => {}
        }
        self.state = self.next(tms);
        match self.state {
            Reset => self.ir = IR_IDCODE,
            CaptureIr => {
                self.shift = 0b000001;
                self.len = IR_LEN;
            }
            CaptureDr => {
                if self.ir == IR_IDCODE {
                    self.shift = u64::from(self.idcode);
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

impl Pins for Tap {
    fn pins(&mut self, tck: bool, tms: bool, tdi: bool) {
        if tck && !self.tck {
            self.rise(tms, tdi);
        }
        self.tck = tck;
    }

    fn tdo(&mut self) -> bool {
        matches!(self.state, State::ShiftDr | State::ShiftIr)
            && self.shift & 1 != 0
    }

    fn reset(&mut self, trst: bool, _srst: bool) {
        if trst {
            self.state = State::Reset;
            self.ir = IR_IDCODE;
        }
    }
}

/// OpenOCD against `tap`, expecting the board's identity, to the end.
fn scan(
    tap: &mut Tap,
    name: &str,
) -> (remote_bitbang::Served, remote_bitbang::Finished) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        "adapter driver remote_bitbang\n\
         remote_bitbang host 127.0.0.1\n\
         remote_bitbang port {port}\n\
         transport select jtag\n\
         jtag newtap xc7 tap -irlen {IR_LEN} -expected-id {IDCODE:#010x}\n\
         init\n\
         shutdown\n"
    );
    let run = Openocd::start(&openocd(), &script, &tmp(name)).unwrap();
    let served = serve(listener, tap, Duration::from_secs(20)).unwrap();
    let done = run.finish(Duration::from_secs(20)).unwrap();
    (served, done)
}

#[test]
fn it_is_openocd_0_12_0() {
    let out = Command::new(openocd()).arg("--version").output().unwrap();
    let text = String::from_utf8_lossy(&out.stderr).to_string()
        + &String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Open On-Chip Debugger 0.12.0"),
        "the version: {text}"
    );
}

#[test]
fn it_connects_and_finds_the_tap() {
    let mut tap = Tap::new(IDCODE);
    let (served, done) = scan(&mut tap, "finds");
    assert!(
        served.rises > 0,
        "OpenOCD clocked the TAP: {served:?}\n{}",
        done.log
    );
    assert!(
        done.log.contains("tap/device found: 0x13636093"),
        "OpenOCD found the identity:\n{}",
        done.log
    );
    assert!(!done.log.contains("UNEXPECTED"), "{}", done.log);
    assert!(
        done.status.is_some_and(|s| s.success()),
        "it ended by itself and well: {:?}\n{}",
        done.status,
        done.log
    );
}

#[test]
fn a_wrong_identity_is_reported() {
    let mut tap = Tap::new(0x0bad_c0de | 1);
    let (_, done) = scan(&mut tap, "wrong");
    assert!(
        done.log.contains("UNEXPECTED"),
        "OpenOCD said the identity was not the one expected:\n{}",
        done.log
    );
}
