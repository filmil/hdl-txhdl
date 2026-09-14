// SPDX-License-Identifier: Apache-2.0
//! A reservation station of two inputs, `Station2`, with a byte on
//! one input and a half word on the other, four lines by a two-bit
//! tag. The two sources offer their tags in different orders, one of
//! them with gaps, and the sink holds the output off for two cycles
//! at a time, so a line waits for room and a repeated tag waits for
//! its cell. Each line comes out as its tag and both values; the
//! netlist is checked against this run.
use txhdl::comp::trace::{stop, Wave};
use txhdl::comp::{chan, now, DefaultClock, Running, Unit};
use txhdl::types::U;
use txhdl_parts::station::{Line2, Station2, Tagged};

/// The station of this example: two-bit tags, four lines, a byte and
/// a half word.
type Station = Station2<2, 4, U<8>, U<16>>;

fn main() {
    let (a_tx, a_rx) = chan::<Tagged<2, U<8>>, DefaultClock>();
    let (b_tx, b_rx) = chan::<Tagged<2, U<16>>, DefaultClock>();
    let (o_tx, o_rx) = chan::<Line2<2, U<8>, U<16>>, DefaultClock>();
    let mut station = Station::default();
    let (occ0, occ1) = (station.occ0, station.occ1);
    if let Some(mut w) = Wave::from_env() {
        w.clock::<DefaultClock>();
        w.add("in0", &a_rx);
        w.add("in1", &b_rx);
        w.add("out", &o_rx);
        w.add("station", &station);
        w.start();
    }
    let mut sim = Running::new(station.run((a_rx, b_rx), o_tx));
    // The bytes come tagged 0, 1, 2, 3, then 0 and 2 again; the half
    // words come tagged 2, 0, 3, 1, 2, 0, with a gap every third
    // cycle; the sink takes on two cycles in four.
    let a_tags = [0u8, 1, 2, 3, 0, 2];
    let b_tags = [2u8, 0, 3, 1, 2, 0];
    let (mut ai, mut bi) = (0, 0);
    println!(" t a(tag) b(tag) occ0 occ1  line");
    for t in 0..22 {
        let a = if ai < a_tags.len() && a_tx.ready().to_bool() {
            let tag = a_tags[ai];
            a_tx.send(Tagged {
                tag: U::from(tag),
                value: U::from(0x10 + ai as u8),
            });
            ai += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let b = if bi < b_tags.len() && t % 3 != 2 && b_tx.ready().to_bool() {
            let tag = b_tags[bi];
            let value = U::from(0x100 * (bi as u16 + 1));
            b_tx.send(Tagged {
                tag: U::from(tag),
                value,
            });
            bi += 1;
            format!("{tag}")
        } else {
            "-".to_string()
        };
        let line = if t % 4 >= 2 { o_rx.recv() } else { None };
        sim.cycle();
        println!(
            "{:2} {:>6} {:>6} {:04b} {:04b}  {}",
            now() / 2,
            a,
            b,
            occ0.get().raw(),
            occ1.get().raw(),
            line.map_or("-".to_string(), |l| format!(
                "tag {} = ({:#x}, {:#x})",
                l.tag.raw(),
                l.v0.raw(),
                l.v1.raw()
            ))
        );
    }
    stop();
    txhdl::netlist::write_vhdl_from_env(&Station::lowered("station"));
    print!("\n{}", Station::verilog("station"));
}
