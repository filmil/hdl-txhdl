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
    /// What the terminal types once the core has said a line. In
    /// blocks, where a sender waits for the core to answer between
    /// them: the first block goes after the first line, and each block
    /// after it once one more byte has come back.
    reply: Vec<u8>,
    typed: usize,
    /// The reply's blocks, in bytes: the terminal types one and waits
    /// for the core to answer before the next. Empty means all of the
    /// reply, back to back.
    blocks: Vec<usize>,
    /// Which block is being typed, and whether this terminal is
    /// waiting at its end for the core to answer.
    at_block: usize,
    waiting: bool,
    /// How many bytes the core had said when the block began.
    said_at_block: usize,
    /// The frame going out, as bits, and how far along it is; none
    /// between frames.
    typing: Option<(u32, u32)>,
    pause: u32,
    spoken_to: bool,
}

impl Terminal {
    /// A terminal that will type `reply` after the core's first line,
    /// back to back.
    pub fn new(reply: &[u8]) -> Self {
        Self {
            reply: reply.to_vec(),
            pause: PAUSE,
            ..Default::default()
        }
    }

    /// The same, in blocks: the terminal types one block and waits for
    /// the core to say one more byte before the next, which is what a
    /// sender does when the far end acknowledges. The blocks are given
    /// in bytes and have to be the ones the far end acknowledges at,
    /// since a sender that pauses anywhere else waits for a byte that
    /// is not coming. A loader that writes into memory cannot keep up
    /// with a line that never pauses, and a port buffers eight bytes.
    pub fn paced(reply: &[u8], blocks: &[usize]) -> Self {
        Self {
            reply: reply.to_vec(),
            pause: PAUSE,
            blocks: blocks.to_vec(),
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
        // In blocks: at the end of each one the terminal waits for the
        // core to say a byte it had not said when the block ended.
        // Counting from the end rather than from the start matters: a
        // sender that goes one block early is a block ahead of the far
        // end for the whole stream, and the port buffers eight bytes.
        if !self.blocks.is_empty() {
            let ends: usize = self.blocks[..self.at_block + 1]
                .iter()
                .sum::<usize>()
                .min(self.reply.len());
            if self.typed == ends && self.at_block + 1 < self.blocks.len() {
                if !self.waiting {
                    self.waiting = true;
                    self.said_at_block = self.said.len();
                    return true;
                }
                if self.said.len() <= self.said_at_block {
                    return true;
                }
                self.waiting = false;
                self.at_block += 1;
            }
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

    /// How many bytes of the reply have been typed, for a test that
    /// wants to say where a stream stopped.
    pub fn typed(&self) -> usize {
        self.typed
    }

    /// Whether a frame is on either line.
    pub fn busy(&self) -> bool {
        self.in_frame || self.typing.is_some()
    }
}
