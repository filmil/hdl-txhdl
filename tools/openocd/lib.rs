// SPDX-License-Identifier: Apache-2.0
//! OpenOCD's `remote_bitbang` adapter, served from a test, and OpenOCD
//! run against it (issue 853).
//!
//! `remote_bitbang` is OpenOCD's simplest adapter: it connects over TCP
//! and sends one byte a step. `0` to `7` set the three pins it drives,
//! TCK, TMS and TDI, as the bits of the digit from the top; `R` asks for
//! TDO, answered `0` or `1`; `r` to `u` set the two resets; `B` and `b`
//! turn a light on and off; `Q` ends the session. That is the whole
//! protocol, so a simulated design can sit behind it: [`serve`] reads
//! the bytes and turns each into a call on [`Pins`], which the design
//! implements, on the caller's own thread, so the design need not be
//! `Send`.
//!
//! [`Openocd`] runs the pinned OpenOCD, //third_party/openocd, on a
//! script, with its log in a file and a time limit, so that a test
//! starts it, serves it, and reads what it said.
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The design behind the adapter: the three pins OpenOCD drives, the
/// one it reads, and the two resets.
pub trait Pins {
    /// TCK, TMS and TDI as OpenOCD sets them. A design advances on a
    /// rise of `tck`.
    fn pins(&mut self, tck: bool, tms: bool, tdi: bool);
    /// TDO, as OpenOCD reads it now.
    fn tdo(&mut self) -> bool;
    /// The two resets, each asserted while true.
    fn reset(&mut self, trst: bool, srst: bool);
}

/// What a session did, for a test to check that it happened at all.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Served {
    /// Rises of TCK.
    pub rises: u64,
    /// Reads of TDO.
    pub reads: u64,
    /// Whether OpenOCD ended it with `Q`, rather than by closing.
    pub quit: bool,
}

/// Serves one OpenOCD session on `listener`, on this thread, until
/// OpenOCD sends `Q` or closes. `timeout` bounds the wait for it to
/// connect and every silence after, so a test whose OpenOCD never comes
/// fails rather than hangs.
pub fn serve(
    listener: TcpListener,
    pins: &mut impl Pins,
    timeout: Duration,
) -> io::Result<Served> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    let stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err(io::Error::new(
                        ErrorKind::TimedOut,
                        "OpenOCD did not connect",
                    ));
                }
                sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_nodelay(true)?;
    let mut input = BufReader::new(stream.try_clone()?);
    let mut output = BufWriter::new(stream);
    let mut served = Served::default();
    let mut tck = false;
    loop {
        // What OpenOCD sent in one go; the answers to it go back once it
        // is all handled, since OpenOCD waits for them before it sends
        // more.
        let chunk = input.fill_buf()?.to_vec();
        if chunk.is_empty() {
            output.flush()?;
            return Ok(served);
        }
        input.consume(chunk.len());
        for byte in chunk {
            match byte {
                b'0'..=b'7' => {
                    let v = byte - b'0';
                    let rise = !tck && v & 4 != 0;
                    tck = v & 4 != 0;
                    pins.pins(tck, v & 2 != 0, v & 1 != 0);
                    if rise {
                        served.rises += 1;
                    }
                }
                b'R' => {
                    served.reads += 1;
                    output.write_all(if pins.tdo() { b"1" } else { b"0" })?;
                }
                b'r'..=b'u' => {
                    let v = byte - b'r';
                    pins.reset(v & 2 != 0, v & 1 != 0);
                }
                b'B' | b'b' => {}
                b'Q' => {
                    served.quit = true;
                    output.flush()?;
                    return Ok(served);
                }
                other => {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        format!("not a remote_bitbang command: {other:#04x}"),
                    ))
                }
            }
        }
        output.flush()?;
    }
}

/// OpenOCD running on a script, with its log in a file.
pub struct Openocd {
    child: Child,
    log: PathBuf,
}

/// How a run of OpenOCD ended.
#[derive(Debug)]
pub struct Finished {
    /// Its exit status, or `None` if it was killed at the time limit.
    pub status: Option<ExitStatus>,
    /// Everything it wrote, both streams.
    pub log: String,
}

impl Openocd {
    /// Starts `binary`, the pinned OpenOCD, on `script`, which is written
    /// into `dir` beside the log.
    pub fn start(
        binary: &Path,
        script: &str,
        dir: &Path,
    ) -> io::Result<Openocd> {
        let cfg = dir.join("openocd.cfg");
        std::fs::write(&cfg, script)?;
        let log = dir.join("openocd.log");
        let out = File::create(&log)?;
        let child = Command::new(binary)
            .arg("-f")
            .arg(&cfg)
            .stdin(Stdio::null())
            .stdout(out.try_clone()?)
            .stderr(out)
            .spawn()?;
        Ok(Openocd { child, log })
    }

    /// Waits for it to exit, and kills it if it has not within `timeout`.
    pub fn finish(mut self, timeout: Duration) -> io::Result<Finished> {
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(s) = self.child.try_wait()? {
                break Some(s);
            }
            if Instant::now() > deadline {
                self.child.kill()?;
                self.child.wait()?;
                break None;
            }
            sleep(Duration::from_millis(20));
        };
        let log = std::fs::read_to_string(&self.log)?;
        Ok(Finished { status, log })
    }
}
