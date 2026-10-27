// SPDX-License-Identifier: Apache-2.0
//! The Ethernet host link as the FireSim top on the AX7A200B
//! instantiates it: the Verilog of `host_bridge`, HostLink and
//! LiteMaster joined, on standard output (issue 1931).
use txhdl_parts::remote::host::HostBridge;

/// The design's device number on the wire; the driver's `+device=`.
const DEVICE: usize = 5;

fn main() {
    print!("{}", HostBridge::<DEVICE>::verilog("host_bridge"));
}
