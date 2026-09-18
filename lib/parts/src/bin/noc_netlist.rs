// SPDX-License-Identifier: Apache-2.0
//! The Verilog of one node of the network, and of one switch inside
//! it, on standard output: what a synthesis run needs to say how fast
//! a node can go and what it costs (issue 99).
//!
//! The parameters are the ones `//soc` builds its lattice with: a two
//! by two lattice of thirty-two-bit addresses and words, four strobe
//! lanes and two-bit identifiers. The coordinate is the middle of a
//! larger lattice, `(1, 1)`, so that no direction is optimised away by
//! a node that sits on an edge.
use txhdl_parts::bus::noc::node::Node;
use txhdl_parts::bus::noc::switch::Switch;

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "node".into());
    match which.as_str() {
        "switch" => {
            print!("{}", Switch::<1, 1, 2, 2, 32, 32, 4, 2>::verilog("switch"))
        }
        _ => print!("{}", Node::<1, 1, 2, 2, 32, 32, 4, 2>::verilog("node")),
    }
}
