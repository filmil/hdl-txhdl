// SPDX-License-Identifier: Apache-2.0
//! `as` in a lowered body (issue 496).
//!
//! A ring of 256 words written and read by pointers of twelve bits:
//! the pointers count on past the ring, and each is narrowed to its low
//! eight bits with `as` to address a word, `p.raw() as u8 as usize`.
//! Rust's `as` keeps the low bits of the value it converts, and so does
//! the netlist now, `wp[7:0]`. The lowering used to drop the cast and
//! keep the whole value, which was harmless while every cast in the
//! tree widened, as a survey of all 132 of them showed. Here it would
//! not be: the netlist would address word 256 as the run addressed word
//! 0, and nvc would stop on an index out of range.
//!
//! A cast to anything but an unsigned integer is refused with a message
//! naming the width methods instead. The words read are asserted
//! against what was written, and the netlist is checked against the run
//! under nvc and Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    signal, Clock, DefaultClock, In, Mem, Out, Reg, Running, Unit,
};
use txhdl::types::{Bit, U};
use txhdl::{lower, when, Trace};

// begin{unit}
/// A ring of 256 bytes and two pointers that run past it.
#[derive(Trace, Default)]
pub struct Ring {
    pub words: Mem<U<8>, 256>,
    pub wp: Reg<U<12>>,
    pub rp: Reg<U<12>>,
}

#[lower]
impl Unit for Ring {
    async fn run(
        &mut self,
        (put, take, d): (In<Bit>, In<Bit>, In<U<8>>),
        q: Out<U<8>>,
    ) {
        loop {
            DefaultClock::rising().await;
            let wp = self.wp.get();
            let rp = self.rp.get();
            let w = put.get();
            let r = take.get();
            // The low eight bits of each pointer, by `as`.
            let wa = wp.raw() as u8 as usize;
            let ra = rp.raw() as u8 as usize;
            when!(w => self { words.at(wa): d.get(), wp: wp + 1 });
            when!(r => self { rp: rp + 1 });
            q.set(self.words.read(ra));
        }
    }
}
// end{unit}

fn main() {
    let (put_out, put) = signal::<Bit, DefaultClock>();
    let (take_out, take) = signal::<Bit, DefaultClock>();
    let (d_out, d) = signal::<U<8>, DefaultClock>();
    let (q_out, q) = signal::<U<8>, DefaultClock>();
    let mut unit = Ring::default();
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("put", &put);
        w.add("take", &take);
        w.add("d", &d);
        w.add("ring", &unit);
        w.add("q", &q);
        w.start();
    }
    let mut sim = Running::new(unit.run((put, take, d), q_out));
    // 300 words through a ring of 256, eight ahead of the reader: both
    // pointers run past the end, and every word read is the one written
    // eight earlier.
    let mut read = Vec::new();
    for i in 0..308u32 {
        let writing = i < 300;
        let reading = i >= 8;
        put_out.set(if writing { Bit::One } else { Bit::Zero });
        take_out.set(if reading { Bit::One } else { Bit::Zero });
        d_out.set(U::from((i * 7 + 3) as u8));
        sim.cycle();
        if reading {
            read.push(q.get().raw() as u8);
        }
    }
    let want: Vec<u8> = (0..300u32).map(|i| (i * 7 + 3) as u8).collect();
    println!("read {} words, the first {:?}", read.len(), &read[..6]);
    assert_eq!(read, want, "every word read is the one written");
    stop();
    let net = Ring::lowered("ring");
    txhdl::netlist::write_vhdl_from_env(&net);
    print!("\n{}", net.verilog());
}
