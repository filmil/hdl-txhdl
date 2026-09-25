// SPDX-License-Identifier: Apache-2.0
//! A wait under `if`: a state the process visits only when a condition
//! holds.
//!
//! `ex_seq` numbered a loop's waits into states and `ex_serial`
//! unrolled a counted step. A step taken only sometimes is a wait
//! under `if`, and the lowering makes it a state with a way around
//! it: the state before the `if` goes to the wait inside when the
//! condition holds at its edge, and past it otherwise, so the
//! register's next value is a choice rather than a number. What
//! follows the `if` is placed in every state the `if` can end in.
//!
//! The framer sends a byte as a serial frame: a start bit, eight data
//! bits, a parity bit when `parity` asks for one, and a stop bit.
//! Twelve states, and the parity state is the one under `if`. The run
//! is checked against the netlist under nvc and Verilator, and the
//! netlist is printed, with the choice in it.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    chan, now, signal, Clock, DefaultClock, In, Reg, Running, Rx, Tx, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, Trace};

// begin{unit}
/// Sends a byte as a frame on a line: start, eight bits, parity when
/// asked, stop.
#[derive(Trace, Default)]
pub struct Framer {
    /// The byte being sent.
    pub sr: Reg<U<8>>,
}

/// The even parity of a byte.
#[lower]
fn even(b: U<8>) -> Bit {
    b.bit(0)
        ^ b.bit(1)
        ^ b.bit(2)
        ^ b.bit(3)
        ^ b.bit(4)
        ^ b.bit(5)
        ^ b.bit(6)
        ^ b.bit(7)
}

#[lower]
impl Unit<(Rx<U<8>>, In<Bit>), Tx<Bit>> for Framer {
    async fn run(
        &mut self,
        (bytes, parity): (Rx<U<8>>, In<Bit>),
        line: Tx<Bit>,
    ) {
        loop {
            let b = bytes.wait().await;
            self.sr.set(b);
            // The start bit.
            DefaultClock::rising().await;
            line.send(Bit::Zero);
            // The data, low bit first.
            for i in 0..8 {
                DefaultClock::rising().await;
                line.send(self.sr.get().bit(i));
            }
            // The parity bit, when asked for at the last data bit.
            if parity.get().to_bool() {
                DefaultClock::rising().await;
                line.send(even(self.sr.get()));
            }
            // The stop bit.
            DefaultClock::rising().await;
            line.send(Bit::One);
        }
    }
}
// end{unit}

fn main() {
    let (byte_tx, bytes) = chan::<U<8>, DefaultClock>();
    let (parity_out, parity) = signal::<Bit, DefaultClock>();
    let (line, line_rx) = chan::<Bit, DefaultClock>();
    let mut framer = Framer::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        // The ports are traced under the names `run` gives them.
        w.add("bytes", &bytes);
        w.add("parity", &parity);
        w.add("framer", &framer);
        w.add("line", &line);
        w.start();
    }
    let mut sim = Running::new(framer.run((bytes, parity), line));
    // Two bytes: the first without parity, the second with. The bits
    // are gathered as they come and the frames checked afterwards.
    let sent = [0xa5u8, 0x3c];
    let mut next = 0usize;
    let mut got: Vec<u8> = Vec::new();
    println!(" t byte par bit");
    for t in 0..28u32 {
        let byte = if next < sent.len() && byte_tx.ready().to_bool() {
            byte_tx.send(U::<8>::from(sent[next]));
            next += 1;
            format!("{:02x}", sent[next - 1])
        } else {
            " -".to_string()
        };
        // Parity is asked for from the middle of the run on, so the
        // first frame's last data bit sees it low and the second's
        // sees it high.
        let par = t >= 12;
        parity_out.set(par);
        let bit = match line_rx.recv() {
            Some(b) => {
                got.push(u8::from(b.to_bool()));
                format!("{}", u8::from(b.to_bool()))
            }
            None => "-".to_string(),
        };
        sim.cycle();
        println!("{:2} {byte}   {}   {bit}", now(), u8::from(par));
    }
    // Frame one: start, eight bits of 0xa5, stop. Frame two: start,
    // eight bits of 0x3c, parity (four ones: even, so 0), stop.
    let mut want: Vec<u8> = vec![0];
    want.extend((0..8).map(|i| (0xa5u8 >> i) & 1));
    want.push(1);
    want.push(0);
    want.extend((0..8).map(|i| (0x3cu8 >> i) & 1));
    want.push(0);
    want.push(1);
    assert_eq!(got, want, "ten bits, then eleven");
    stop();
    let net = Framer::lowered("framer");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
