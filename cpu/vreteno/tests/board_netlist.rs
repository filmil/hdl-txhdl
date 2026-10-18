// SPDX-License-Identifier: Apache-2.0
//! The board's netlist, which holds the memory controller as a foreign
//! module. A test crate of its own and not one of `board.rs`'s tests:
//! that crate is built at opt-level 3 for its simulations, and with
//! this test in it LLVM optimised the board's netlist generator too,
//! for 77 minutes and 9 GiB (#1792, #1807). Here it is built at the
//! default opt-level 0, which the generator needs.
use vreteno32::board::Board;

/// One module holds the rest, and the controller is an instance of its
/// wrapper that the netlist does not write, with the memory's pads
/// running out to the board's own ports.
#[test]
fn the_netlist_holds_the_controller() {
    let v = Board::<4>::verilog("board");
    assert!(v.contains("module board("), "the top");
    assert!(v.contains("module board_cpu("), "the core");
    assert!(v.contains("module board_ddr3_pins("), "the pins");
    assert!(v.contains("ddr3_axi32 "), "the controller");
    assert!(!v.contains("module ddr3_axi32"), "not written");
    assert!(v.contains("ring_osc "), "the rings");
    assert!(!v.contains("module ring_osc"), "not written either");
    assert!(v.contains("inout [31:0] dq"), "the data pads");
}
