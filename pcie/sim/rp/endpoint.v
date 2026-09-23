// SPDX-License-Identifier: Apache-2.0
// The endpoint as the root port's host expects to find it: `pcie_top`
// with the slot's pins, and beside it the names the host's DMA tasks
// reach into `board.EP` for. Those tasks compare a DMA's data against
// the endpoint's AXI write channel and never run here, the bridge
// having no DMA, but the names must resolve for the host to elaborate.
`timescale 1ps/1ps
module endpoint (
    input  wire       sys_clk_p,
    input  wire       sys_clk_n,
    input  wire       sys_rst_n,
    output wire [1:0] pci_exp_txp,
    output wire [1:0] pci_exp_txn,
    input  wire [1:0] pci_exp_rxp,
    input  wire [1:0] pci_exp_rxn
);
  wire led1, led2, led3, led4;

  pcie_top top (
      .pcie_clk_p(sys_clk_p), .pcie_clk_n(sys_clk_n),
      .pcie_rx_p(pci_exp_rxp), .pcie_rx_n(pci_exp_rxn),
      .pcie_tx_p(pci_exp_txp), .pcie_tx_n(pci_exp_txn),
      .reset_n(sys_rst_n),
      .led1(led1), .led2(led2), .led3(led3), .led4(led4)
  );

  // What the host's DMA tasks name, from the top's own nets.
  wire        user_clk     = top.aclk;
  wire [63:0] m_axi_wdata  = top.wdata;
  wire [7:0]  m_axi_wstrb  = top.wstrb;
  wire        m_axi_wvalid = top.wvalid;
  wire        m_axi_wready = top.wready;
endmodule
