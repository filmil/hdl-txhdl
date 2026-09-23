// SPDX-License-Identifier: Apache-2.0
// The board's endpoint against a root port (issue 178): the top of the
// PCIe design, `pcie_top`, on two lanes to the behavioural root port
// AMD ships in the XDMA core's example design, `xilinx_pcie3_uscale_rp`,
// with the host that lives inside it. The host trains the link,
// scans and programs the BARs, and runs the test named on the
// simulator's command line, `tests.vh` beside this file, which reads
// and writes BAR1 through to `BarRegs` and sets `ok`. The module is
// named `board`, as the example's is, because every task of the host
// reaches its root port by that name: `board.RP.tx_usrapp` and the
// rest.
`timescale 1ps/1ps
module board;
  // The slot's reference clock, 100 MHz, to each side.
  localparam REF_CLK_HALF_CYCLE = 5000;
  // 512 bytes, the endpoint's maximum payload.
  localparam [2:0] PF0_DEV_CAP_MAX_PAYLOAD_SIZE = 3'b010;
  // The width of the root port's streams, which its host reads here.
  parameter C_DATA_WIDTH = 64;

  reg sys_rst_n;
  wire ep_sys_clk_p, ep_sys_clk_n, rp_sys_clk_p, rp_sys_clk_n;
  wire [1:0] ep_pci_exp_txn, ep_pci_exp_txp;
  wire [1:0] rp_pci_exp_txn, rp_pci_exp_txp;
  // The verdict, which the test sets when every read came back right.
  reg ok = 1'b0;

  sys_clk_gen_ds #(.halfcycle(REF_CLK_HALF_CYCLE), .offset(0)) CLK_GEN_RP (
      .sys_clk_p(rp_sys_clk_p), .sys_clk_n(rp_sys_clk_n)
  );
  sys_clk_gen_ds #(.halfcycle(REF_CLK_HALF_CYCLE), .offset(0)) CLK_GEN_EP (
      .sys_clk_p(ep_sys_clk_p), .sys_clk_n(ep_sys_clk_n)
  );

  // The reset, held as long as the example holds it.
  initial begin
    $display("[%t] : System Reset Is Asserted...", $realtime);
    sys_rst_n = 1'b0;
    repeat (500) @(posedge rp_sys_clk_p);
    $display("[%t] : System Reset Is De-asserted...", $realtime);
    sys_rst_n = 1'b1;
  end

  endpoint EP (
      .sys_clk_p(ep_sys_clk_p), .sys_clk_n(ep_sys_clk_n),
      .sys_rst_n(sys_rst_n),
      .pci_exp_txp(ep_pci_exp_txp), .pci_exp_txn(ep_pci_exp_txn),
      .pci_exp_rxp(rp_pci_exp_txp), .pci_exp_rxn(rp_pci_exp_txn)
  );

  xilinx_pcie3_uscale_rp #(
      .PF0_DEV_CAP_MAX_PAYLOAD_SIZE(PF0_DEV_CAP_MAX_PAYLOAD_SIZE)
  ) RP (
      .sys_clk_p(rp_sys_clk_p), .sys_clk_n(rp_sys_clk_n),
      .sys_rst_n(sys_rst_n),
      .pci_exp_txp(rp_pci_exp_txp), .pci_exp_txn(rp_pci_exp_txn),
      .pci_exp_rxp(ep_pci_exp_txp), .pci_exp_rxn(ep_pci_exp_txn)
  );
endmodule
