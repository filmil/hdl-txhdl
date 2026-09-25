// SPDX-License-Identifier: Apache-2.0
//! A `for` of a constant count inside a process of several waits.
//!
//! `ex_seq` numbered a loop's waits into states. A step repeated a
//! fixed number of times is a `for` whose body waits, and the lowering
//! unrolls it: a state per turn, the loop's variable a number in each,
//! so `for i in 0..8` around one wait is eight states and `bit(i)` is
//! `bit(0)` to `bit(7)` in them. The bounds are written out, since
//! the macro counts the states; a `for` that does not wait keeps its
//! own unrolling, which takes a bound the macro cannot see.
//!
//! The serialiser takes a byte and puts its bits on a channel one a
//! cycle, low bit first, with no state register and no bit counter
//! written by hand. The run is checked against the netlist under nvc
//! and Verilator, and the netlist is printed: nine states, eight of
//! them the turns.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, now, Clock, DefaultClock, Reg, Running, Rx, Tx, Unit};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// Takes a byte and sends its bits, low bit first, one a cycle.
#[derive(Trace, Default)]
pub struct Serial {
    /// The byte being sent.
    pub sr: Reg<U<8>>,
}

#[lower]
impl Unit<Rx<U<8>>, Tx<Bit>> for Serial {
    async fn run(&mut self, bytes: Rx<U<8>>, bits: Tx<Bit>) {
        loop {
            let b = bytes.wait().await;
            self.sr.set(b);
            for i in 0..8 {
                DefaultClock::rising().await;
                bits.send(self.sr.get().bit(i));
            }
        }
    }
}
// end{unit}

fn main() {
    let (byte_tx, bytes) = chan::<U<8>, DefaultClock>();
    let (bits, bit_rx) = chan::<Bit, DefaultClock>();
    let mut serial = Serial::default();
    let sr = serial.sr;
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("bytes", &bytes);
        w.add("serial", &serial);
        w.add("bits", &bits);
        w.start();
    }
    let mut sim = Running::new(serial.run(bytes, bits));
    // Two bytes, each offered until taken; the bits are gathered as
    // they come and checked against the byte, low bit first.
    let sent = [0xa5u8, 0x3c];
    let mut next = 0usize;
    let mut got: Vec<u8> = Vec::new();
    println!(" t byte sr   bit");
    for _ in 0..22u32 {
        let byte = if next < sent.len() && byte_tx.ready().to_bool() {
            byte_tx.send(U::<8>::from(sent[next]));
            next += 1;
            format!("{:02x}", sent[next - 1])
        } else {
            " -".to_string()
        };
        let bit = match bit_rx.recv() {
            Some(b) => {
                got.push(u8::from(b.to_bool()));
                format!("{}", u8::from(b.to_bool()))
            }
            None => "-".to_string(),
        };
        sim.cycle();
        println!("{:2} {byte}   {:02x}   {bit}", now(), sr.get().raw());
    }
    let mut back: Vec<u8> = Vec::new();
    for word in got.chunks(8) {
        let v = word
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, b)| acc | (b << i));
        back.push(v);
    }
    assert_eq!(back, sent, "the bits, low first, are the bytes");
    stop();
    let net = Serial::lowered("serial");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
