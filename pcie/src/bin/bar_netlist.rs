// SPDX-License-Identifier: Apache-2.0
//! The Verilog of everything behind BAR0, on standard output, for the
//! board's top to instantiate as `pcie_bar`.
use pcie::bar::PcieBar;

fn main() {
    print!("{}", PcieBar::verilog("pcie_bar"));
}
