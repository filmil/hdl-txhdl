// SPDX-License-Identifier: Apache-2.0
//! A fastboot client on the far end of the model's Ethernet cable
//! (issue 1390), so that a download to the fastboot server is timed on
//! the machine model rather than only on the board.
//!
//! The client's TCP/IP is smoltcp's, not a stack written here. It runs
//! over the model's port as a smoltcp `Device`: what the board sends is
//! its receive queue, and what it sends goes into the port's inbox,
//! where frames arrive at the line's pace and are dropped when both
//! slots are held, as on the board (#1313, #1314). Its clock is the
//! model's: a hundred steps a microsecond, the core's 100 MHz.
//!
//! The protocol is AOSP's fastboot over TCP, as `zephyr/fastboot`
//! answers it: `FB01` each way, then messages of an eight-byte
//! big-endian length and the bytes; `download:%08x`, answered `DATA`,
//! the image as one message, answered `OKAY`.
use std::collections::VecDeque;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr};

/// The client's station: srv's side of the board's cable.
pub const CLIENT_MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
pub const CLIENT_IP: [u8; 4] = [192, 168, 1, 1];
/// The fastboot server's address and port, as its image has them.
pub const SERVER_IP: [u8; 4] = [192, 168, 1, 50];
pub const SERVER_PORT: u16 = 5554;

/// Model steps a microsecond: the core runs at 100 MHz.
const STEPS_A_MICROSECOND: u64 = 100;

/// The step the client first connects at: 50 ms of the model's time,
/// after the server listens, which it does within 2 million steps. A
/// connection asked for earlier is refused by a stack not yet up, and
/// TCP's backoff then waits a second before the next try.
pub const CONNECT_AT: u64 = 5_000_000;

/// The cable as smoltcp sees it.
#[derive(Default)]
pub struct Wire {
    /// Frames the board sent, waiting for the client.
    pub rx: VecDeque<Vec<u8>>,
    /// Frames the client sent, for the port's inbox.
    pub tx: Vec<Vec<u8>>,
}

pub struct WireRx(Vec<u8>);
pub struct WireTx<'a>(&'a mut Vec<Vec<u8>>);

impl RxToken for WireRx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl TxToken for WireTx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut frame = vec![0u8; len];
        let r = f(&mut frame);
        self.0.push(frame);
        r
    }
}

impl Device for Wire {
    type RxToken<'a> = WireRx;
    type TxToken<'a> = WireTx<'a>;

    fn receive(
        &mut self,
        _now: Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let f = self.rx.pop_front()?;
        Some((WireRx(f), WireTx(&mut self.tx)))
    }

    fn transmit(&mut self, _now: Instant) -> Option<Self::TxToken<'_>> {
        Some(WireTx(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps
    }
}

/// Where the client is in the protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Connecting,
    Hello,
    Download,
    Data,
    Okay,
    Done,
    Failed,
}

/// The client: its stack, its socket, the image it sends, and what it
/// has counted.
pub struct FbClient {
    pub wire: Wire,
    iface: Interface,
    sockets: SocketSet<'static>,
    socket: SocketHandle,
    pub step: Step,
    /// The image the download sends.
    pub image: Vec<u8>,
    /// What is still to go: the framed bytes of the step in hand.
    out: Vec<u8>,
    /// Bytes received and not yet a whole message.
    inbox: Vec<u8>,
    /// The step the transfer began and ended at: the `download`
    /// command's first byte, and the `OKAY` after the image.
    pub began: Option<u64>,
    pub ended: Option<u64>,
    /// The client's TCP segments with data, and those of them that
    /// carried bytes it had already sent: its retransmits.
    pub segments: u64,
    pub retransmits: u64,
    /// The highest sequence number past data sent so far.
    sent_to: Option<u32>,
    /// What the server answered, in order.
    pub answers: Vec<String>,
}

impl std::fmt::Debug for FbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FbClient")
            .field("step", &self.step)
            .field("image", &self.image.len())
            .finish()
    }
}

impl FbClient {
    /// A client that will send `image`, its stack started at step 0.
    pub fn new(image: Vec<u8>) -> Self {
        let mut wire = Wire::default();
        let mut config =
            Config::new(HardwareAddress::Ethernet(EthernetAddress(CLIENT_MAC)));
        config.random_seed = 0x1390;
        let mut iface = Interface::new(config, &mut wire, Instant::ZERO);
        iface.update_ip_addrs(|a| {
            let _ = a.push(IpCidr::new(IpAddress::v4(192, 168, 1, 1), 24));
        });
        let rx = tcp::SocketBuffer::new(vec![0u8; 4096]);
        let tx = tcp::SocketBuffer::new(vec![0u8; 65536]);
        let mut sockets = SocketSet::new(Vec::new());
        let socket = sockets.add(tcp::Socket::new(rx, tx));
        FbClient {
            wire,
            iface,
            sockets,
            socket,
            step: Step::Connecting,
            image,
            out: Vec::new(),
            inbox: Vec::new(),
            began: None,
            ended: None,
            segments: 0,
            retransmits: 0,
            sent_to: None,
            answers: Vec::new(),
        }
    }

    /// A frame the board sent.
    pub fn received(&mut self, frame: Vec<u8>) {
        self.wire.rx.push_back(frame);
    }

    /// The frames the client has sent since the last call, for the
    /// port's inbox.
    pub fn take_sent(&mut self) -> Vec<Vec<u8>> {
        let sent = std::mem::take(&mut self.wire.tx);
        for f in &sent {
            self.count(f);
        }
        sent
    }

    /// A TCP segment of the client's with data: a retransmit if its
    /// bytes start below what was sent before.
    fn count(&mut self, f: &[u8]) {
        if f.len() < 34 || f[12..14] != [0x08, 0x00] || f[23] != 6 {
            return;
        }
        let ihl = ((f[14] & 0xf) as usize) * 4;
        let total = u16::from_be_bytes([f[16], f[17]]) as usize;
        let tcp = 14 + ihl;
        if f.len() < tcp + 20 {
            return;
        }
        let doff = ((f[tcp + 12] >> 4) as usize) * 4;
        let data = total.saturating_sub(ihl + doff) as u32;
        if data == 0 {
            return;
        }
        let seq = u32::from_be_bytes(f[tcp + 4..tcp + 8].try_into().unwrap());
        let end = seq.wrapping_add(data);
        self.segments += 1;
        match self.sent_to {
            Some(to) if (seq.wrapping_sub(to) as i32) < 0 => {
                self.retransmits += 1;
            }
            _ => {}
        }
        match self.sent_to {
            Some(to) if (end.wrapping_sub(to) as i32) <= 0 => {}
            _ => self.sent_to = Some(end),
        }
    }

    /// One message: its length, eight bytes big-endian, then its bytes.
    fn message(bytes: &[u8]) -> Vec<u8> {
        let mut m = (bytes.len() as u64).to_be_bytes().to_vec();
        m.extend_from_slice(bytes);
        m
    }

    /// A whole message from the server, if one has come.
    fn answer(&mut self) -> Option<String> {
        if self.inbox.len() < 8 {
            return None;
        }
        let n =
            u64::from_be_bytes(self.inbox[..8].try_into().unwrap()) as usize;
        if self.inbox.len() < 8 + n {
            return None;
        }
        let body = String::from_utf8_lossy(&self.inbox[8..8 + n]).to_string();
        self.inbox.drain(..8 + n);
        self.answers.push(body.clone());
        Some(body)
    }

    /// The client at model step `now`: its stack polled, its socket
    /// fed and read, and the protocol moved on.
    pub fn poll(&mut self, now: u64) {
        let at = Instant::from_micros((now / STEPS_A_MICROSECOND) as i64);
        self.iface.poll(at, &mut self.wire, &mut self.sockets);
        let sock = self.sockets.get_mut::<tcp::Socket>(self.socket);
        if self.step == Step::Connecting && now < CONNECT_AT {
            return;
        }
        if self.step == Step::Connecting && !sock.is_open() {
            let server = (IpAddress::v4(192, 168, 1, 50), SERVER_PORT);
            if sock.connect(self.iface.context(), server, 49152).is_err() {
                self.step = Step::Failed;
                return;
            }
        }
        if self.step == Step::Connecting && sock.may_send() {
            self.out = b"FB01".to_vec();
            self.step = Step::Hello;
        }
        // What is ready to go, into the socket as room allows.
        if !self.out.is_empty() && sock.can_send() {
            if let Ok(n) = sock.send_slice(&self.out) {
                self.out.drain(..n);
            }
        }
        // What has come, onto the end of the inbox.
        let mut buf = [0u8; 2048];
        while sock.can_recv() {
            match sock.recv_slice(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => self.inbox.extend_from_slice(&buf[..n]),
            }
        }
        match self.step {
            Step::Hello if self.inbox.len() >= 4 => {
                if &self.inbox[..4] != b"FB01" {
                    self.step = Step::Failed;
                    return;
                }
                self.inbox.drain(..4);
                let cmd = format!("download:{:08x}", self.image.len());
                self.out = Self::message(cmd.as_bytes());
                self.began = Some(now);
                self.step = Step::Download;
            }
            Step::Download => {
                if let Some(a) = self.answer() {
                    if !a.starts_with("DATA") {
                        self.step = Step::Failed;
                        return;
                    }
                    self.out = Self::message(&self.image);
                    self.step = Step::Data;
                }
            }
            Step::Data if self.out.is_empty() => self.step = Step::Okay,
            Step::Okay => {
                if let Some(a) = self.answer() {
                    if a.starts_with("OKAY") {
                        self.ended = Some(now);
                        self.step = Step::Done;
                    } else {
                        self.step = Step::Failed;
                    }
                }
            }
            _ => {}
        }
    }
}
