// SPDX-License-Identifier: Apache-2.0
//! An AXI4 to AXI-Lite bridge of two peripherals. A host client on an
//! AXI4 link issues bursts; the bridge takes them one at a time,
//! decodes each to one of two register banks or to nothing, and sends
//! every beat to its bank as an AXI-Lite transaction of its own. The
//! banks are AXI-Lite peripherals written as hardware: no tracker, no
//! identifier, no burst.
//!
//! The run writes a burst of three words, reads four back, reads one
//! word three times in a fixed burst, writes and reads the second
//! bank, and sends a write and a read to an address that is nobody's,
//! which the bridge answers `DecErr` itself. The bridge and both banks
//! are lowered, and the build simulates the three netlists against
//! this run under nvc and under Verilator.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{
    join2, now, Clock, DefaultClock, Mem, Running, Rx, Tx, Unit,
};
use txhdl::types::U;
use txhdl::{lower, Trace};
use txhdl_parts::bus::axi::{axi, AxiHost, BurstKind, Link, Rd, Resp, Wr};
use txhdl_parts::bus::axi_lite::{
    axi_lite, LiteAr, LiteAw, LiteB, LiteBridge2, LiteR, LiteW,
};

/// The link: sixteen-bit addresses, thirty-two-bit words, four lanes,
/// two-bit identifiers, four of them.
type HostUnit = AxiHost<16, 32, 4, 2, 4>;

/// The address map, as the bridge's type states it: two banks of a
/// nibble each, and every other address a hole.
type Bridge = LiteBridge2<16, 32, 4, 2, 0x1000, 0xf000, 0x2000, 0xf000>;

// begin{regs}
/// Four words behind an AXI-Lite link, at the word offsets 0, 4, 8
/// and 12 of its range. A read takes an address and answers the word
/// in the same cycle; a write takes an address and a word together
/// and answers `Okay`. The bridge sends it only its own range, so it
/// checks no address, and it writes the whole word.
#[derive(Trace, Default)]
pub struct Regs {
    pub words: Mem<U<32>, 4>,
}

#[lower]
impl Unit for Regs {
    async fn run(
        &mut self,
        (aw, ar, w): (Rx<LiteAw<16>>, Rx<LiteAr<16>>, Rx<LiteW<32, 4>>),
        (b, r): (Tx<LiteB>, Tx<LiteR<32>>),
    ) {
        loop {
            DefaultClock::rising().await;
            let arh = ar.head();
            let rgo = r.ready() & ar.peek().is_some();
            let _ = ar.recv_if(r.ready());
            let awh = aw.head();
            let wh = w.head();
            let wgo = b.ready() & aw.peek().is_some() & w.peek().is_some();
            let _ = aw.recv_if(wgo);
            let _ = w.recv_if(wgo);
            let rsel = arh.addr.slice::<2, 2>();
            let wsel = awh.addr.slice::<2, 2>();
            if wgo.to_bool() {
                self.words.at(wsel).set(wh.data);
                b.send(LiteB { resp: Resp::Okay });
            }
            if rgo.to_bool() {
                r.send(LiteR {
                    data: self.words.read(rsel),
                    resp: Resp::Okay,
                });
            }
        }
    }
}
// end{regs}

/// A read of `n` whole words at `addr`: four bytes a beat, so the
/// address moves by four.
fn words(addr: u32, n: usize) -> Rd<16> {
    let mut rd = Rd::at(addr, n);
    rd.size = U::from(2u8);
    rd
}

/// A write of whole words at `addr`.
fn write(addr: u32) -> Wr<16> {
    let mut wr = Wr::at(addr);
    wr.size = U::from(2u8);
    wr
}

fn raw(ws: &[U<32>]) -> Vec<u128> {
    ws.iter().map(|w| w.raw()).collect()
}

/// Words as the run prints them, in hexadecimal.
fn hex(ws: &[U<32>]) -> String {
    let s: Vec<String> = ws.iter().map(|w| format!("{:#x}", w.raw())).collect();
    s.join(" ")
}

fn main() {
    // What `lite_bridge!` wrote for two peripherals, for the document
    // to show without a hand-typed copy.
    if std::env::args().any(|a| a == "--source") {
        print!("{}", txhdl_parts::bus::axi_lite::lite_bridge2::SOURCE);
        return;
    }
    // The AXI4 link: the host client and its tracker on one side, and
    // on the other the channels the bridge takes in place of a
    // peripheral's tracker.
    let Link {
        host,
        host_in,
        host_out,
        per_in,
        per_out,
        ..
    } = axi::<16, 32, 4, 2, 4>();
    let (aw, ar, w, _, _) = per_in;
    let (_, _, b, r) = per_out;
    // An AXI-Lite link per bank.
    let l0 = axi_lite::<16, 32, 4>();
    let l1 = axi_lite::<16, 32, 4>();
    let (aw0, ar0, w0, b0, r0) = l0.host;
    let (aw1, ar1, w1, b1, r1) = l1.host;

    let mut host_unit = HostUnit::default();
    let mut bridge = Bridge::default();
    let mut regs0 = Regs::default();
    let mut regs1 = Regs::default();

    if let Some(mut wave) = Wave::from_env() {
        wave.clock::<DefaultClock>();
        // Every channel under the name of the bridge's port on it.
        wave.add("aw", &aw);
        wave.add("ar", &ar);
        wave.add("w", &w);
        wave.add("b", &b);
        wave.add("r", &r);
        wave.add("aw0", &aw0);
        wave.add("ar0", &ar0);
        wave.add("w0", &w0);
        wave.add("b0", &b0);
        wave.add("r0", &r0);
        wave.add("aw1", &aw1);
        wave.add("ar1", &ar1);
        wave.add("w1", &w1);
        wave.add("b1", &b1);
        wave.add("r1", &r1);
        wave.add("bridge", &bridge);
        wave.add("regs0", &regs0);
        wave.add("regs1", &regs1);
        wave.start();
    }

    let client = async move {
        // Three words into the first bank in one burst, then four read
        // back in one: each beat is a transaction of its own there.
        let vs = [U::from(0x11u32), U::from(0x22u32), U::from(0x33u32)];
        let r = host.write(write(0x1004), &vs).await.done().await;
        println!("t={:>3} write 3 at 0x1004 -> {:?}", now(), r.resp);
        assert_eq!(r.resp, Resp::Okay);
        let r = host.read(words(0x1000, 4)).await.done().await;
        println!("t={:>3} read 4 at 0x1000 -> {}", now(), hex(&r.data));
        assert_eq!(raw(&r.data), vec![0, 0x11, 0x22, 0x33]);
        // A fixed burst reads one word three times.
        let mut fixed = words(0x1008, 3);
        fixed.burst = BurstKind::Fixed;
        let r = host.read(fixed).await.done().await;
        println!("t={:>3} fixed 3 at 0x1008 -> {}", now(), hex(&r.data));
        assert_eq!(raw(&r.data), vec![0x22, 0x22, 0x22]);
        // The second bank.
        let r = host.write(write(0x200c), &[U::from(7u32)]).await;
        let r = r.done().await;
        println!("t={:>3} write 1 at 0x200c -> {:?}", now(), r.resp);
        let r = host.read(words(0x200c, 1)).await.done().await;
        println!("t={:>3} read 1 at 0x200c -> {}", now(), hex(&r.data));
        assert_eq!(raw(&r.data), vec![7]);
        // A hole: the bridge answers, a read in every beat it asked for.
        let two = [U::from(1u32), U::from(2u32)];
        let r = host.write(write(0x4000), &two).await.done().await;
        println!("t={:>3} write 2 at 0x4000 -> {:?}", now(), r.resp);
        assert_eq!(r.resp, Resp::DecErr);
        let r = host.read(words(0x4000, 2)).await.done().await;
        println!(
            "t={:>3} read 2 at 0x4000 -> {:?}, {} beats",
            now(),
            r.resp,
            r.data.len()
        );
        assert_eq!((r.resp, r.data.len()), (Resp::DecErr, 2));
        println!("every burst answered, a beat at a time");
    };

    let hardware = join2(
        join2(
            host_unit.run(host_in, host_out),
            bridge.run(
                (aw, ar, w, b0, r0, b1, r1),
                (aw0, ar0, w0, aw1, ar1, w1, b, r),
            ),
        ),
        join2(
            regs0.run((l0.per.0, l0.per.1, l0.per.2), (l0.per.3, l0.per.4)),
            regs1.run((l1.per.0, l1.per.1, l1.per.2), (l1.per.3, l1.per.4)),
        ),
    );
    let mut sim = Running::new(join2(hardware, client));
    for _ in 0..100 {
        sim.cycle();
    }
    stop();
    let net = Bridge::lowered("axi_lite_bridge");
    let mut bank0 = Regs::lowered("lite_regs0");
    let mut bank1 = Regs::lowered("lite_regs1");
    for (port, k) in [("aw", "aw"), ("ar", "ar"), ("w", "w"), ("b", "b")] {
        bank0.trace_as(port, &format!("{k}0"));
        bank1.trace_as(port, &format!("{k}1"));
    }
    bank0.trace_as("r", "r0");
    bank1.trace_as("r", "r1");
    txhdl::netlist::write_netlists_from_env(&[&net, &bank0, &bank1]);
}
