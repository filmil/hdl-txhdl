// SPDX-License-Identifier: Apache-2.0
//! The two halves of the Ethernet MAC as the echo design on the board
//! instantiates them: the Verilog of `eth_rx` and `eth_tx`, on standard
//! output.
use txhdl_parts::eth::{EthRx, EthTx};

fn main() {
    print!("{}\n{}", EthRx::verilog("eth_rx"), EthTx::verilog("eth_tx"));
}
