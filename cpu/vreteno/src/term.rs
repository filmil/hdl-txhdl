// SPDX-License-Identifier: Apache-2.0
//! A terminal on the serial line, for the run and the lockstep test:
//! it reads what the core sends, a start bit, eight bits least
//! significant first sampled in the middle of each, and a stop bit;
//! and once the core has said a line, it types its reply at the
//! port's rate, the bytes back to back, ahead of the program that
//! reads them, which the port's buffer allows. One bit is `DIV`
//! cycles, as in the port the run and the test wire.

/// Cycles per bit, the port's `DIV` in the runs that are checked.
pub const DIV: u32 = 4;

/// The pause before the reply, in cycles, as a typist's would be.
const PAUSE: u32 = 20;

#[derive(Default)]
pub struct Terminal {
    /// What the core has said so far.
    pub said: String,
    frame: u32,
    bit: u32,
    phase: u32,
    in_frame: bool,
    /// What the terminal types once the core has said a line.
    reply: Vec<u8>,
    typed: usize,
    /// The frame going out, as bits, and how far along it is; none
    /// between frames.
    typing: Option<(u32, u32)>,
    pause: u32,
    spoken_to: bool,
}

impl Terminal {
    /// A terminal that will type `reply` after the core's first line.
    pub fn new(reply: &[u8]) -> Self {
        Self {
            reply: reply.to_vec(),
            pause: PAUSE,
            ..Default::default()
        }
    }

    /// The core's line, once per cycle, as the edge left it.
    pub fn see(&mut self, line: bool) {
        if !self.in_frame {
            if !line {
                self.in_frame = true;
                (self.frame, self.bit, self.phase) = (0, 0, 0);
            }
            return;
        }
        self.phase += 1;
        let middle = self.phase % DIV == DIV / 2;
        if self.phase < DIV || !middle {
            return;
        }
        if self.bit < 8 {
            self.frame |= (line as u32) << self.bit;
        }
        self.bit += 1;
        if self.bit == 9 {
            let c = self.frame as u8 as char;
            self.said.push(c);
            self.spoken_to |= c == '\n';
            self.in_frame = false;
        }
    }

    /// The terminal's line for this cycle: high at rest, else the bit
    /// of the frame being typed.
    pub fn level(&mut self) -> bool {
        if let Some((frame, at)) = self.typing {
            let level = frame >> (at / DIV) & 1 == 1;
            self.typing = if at + 1 < 10 * DIV {
                Some((frame, at + 1))
            } else {
                self.typed += 1;
                None
            };
            return level;
        }
        if !self.spoken_to || self.typed >= self.reply.len() {
            return true;
        }
        if self.pause > 0 {
            self.pause -= 1;
            return true;
        }
        // A start bit low, the byte, a stop bit high, bit 0 first.
        let frame = (self.reply[self.typed] as u32) << 1 | 1 << 9;
        self.typing = Some((frame, 1));
        false
    }

    /// Whether a frame is on either line.
    pub fn busy(&self) -> bool {
        self.in_frame || self.typing.is_some()
    }
}
