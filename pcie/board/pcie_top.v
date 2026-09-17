// SPDX-License-Identifier: Apache-2.0
// PCIe on the Alinx AX7A200B: the XDMA endpoint in AXI bridge mode, and
// `pcie_bar`, which `#[lower]` writes for `pcie::bar::PcieBar`, behind
// its BAR0. Written by hand, since it holds what a lowered unit cannot
// say: the transceivers' reference clock buffer, the endpoint core, and
// its clock and reset handed to the lowered design.
//
// The lowered design runs on the endpoint's user clock, so nothing
// crosses a clock. Its reset is high while the endpoint's is low. The
// endpoint's own reset comes from the board's reset key: the slot's
// PERST# is not in the board's manual (issue 179).
//
// The LEDs are lit when driven low: LED1 the link up, LED2 the design
// out of reset, and LED3 and LED4 the two bits a host writes at 0x10.
`default_nettype none
module pcie_top (
    input  wire       pcie_clk_p,
    input  wire       pcie_clk_n,
    input  wire [1:0] pcie_rx_p,
    input  wire [1:0] pcie_rx_n,
    output wire [1:0] pcie_tx_p,
    output wire [1:0] pcie_tx_n,
    input  wire       reset_n,
    output wire       led1,
    output wire       led2,
    output wire       led3,
    output wire       led4
);
  wire refclk;
  IBUFDS_GTE2 refclk_buf (
      .I(pcie_clk_p), .IB(pcie_clk_n), .CEB(1'b0), .O(refclk), .ODIV2()
  );

  wire link_up, aclk, aresetn;
  wire [3:0] awid, arid, bid, rid;
  wire [63:0] awaddr, araddr, wdata, rdata;
  wire [7:0] awlen, arlen, wstrb;
  wire [2:0] awsize, arsize, awprot, arprot;
  wire [1:0] awburst, arburst, bresp, rresp;
  wire [3:0] awcache, arcache;
  wire awlock, arlock, awvalid, awready, wlast, wvalid, wready;
  wire bvalid, bready, arvalid, arready, rlast, rvalid, rready;
  wire [1:0] leds;

  xdma_x2 endpoint (
      .sys_clk(refclk), .sys_rst_n(reset_n), .user_lnk_up(link_up),
      .pci_exp_txp(pcie_tx_p), .pci_exp_txn(pcie_tx_n),
      .pci_exp_rxp(pcie_rx_p), .pci_exp_rxn(pcie_rx_n),
      .axi_aclk(aclk), .axi_aresetn(aresetn),
      .usr_irq_req(1'b0), .usr_irq_ack(), .msi_enable(),
      .msi_vector_width(),
      .m_axi_awid(awid), .m_axi_awaddr(awaddr), .m_axi_awlen(awlen),
      .m_axi_awsize(awsize), .m_axi_awburst(awburst),
      .m_axi_awprot(awprot), .m_axi_awvalid(awvalid),
      .m_axi_awlock(awlock), .m_axi_awcache(awcache),
      .m_axi_awready(awready),
      .m_axi_wdata(wdata), .m_axi_wstrb(wstrb), .m_axi_wlast(wlast),
      .m_axi_wvalid(wvalid), .m_axi_wready(wready),
      .m_axi_bid(bid), .m_axi_bresp(bresp), .m_axi_bvalid(bvalid),
      .m_axi_bready(bready),
      .m_axi_arid(arid), .m_axi_araddr(araddr), .m_axi_arlen(arlen),
      .m_axi_arsize(arsize), .m_axi_arburst(arburst),
      .m_axi_arprot(arprot), .m_axi_arvalid(arvalid),
      .m_axi_arlock(arlock), .m_axi_arcache(arcache),
      .m_axi_arready(arready),
      .m_axi_rid(rid), .m_axi_rdata(rdata), .m_axi_rresp(rresp),
      .m_axi_rlast(rlast), .m_axi_rvalid(rvalid), .m_axi_rready(rready),
      .cfg_mgmt_addr(19'd0), .cfg_mgmt_write(1'b0),
      .cfg_mgmt_write_data(32'd0), .cfg_mgmt_byte_enable(4'd0),
      .cfg_mgmt_read(1'b0), .cfg_mgmt_read_data(),
      .cfg_mgmt_read_write_done(), .cfg_mgmt_type1_cfg_reg_access(1'b0)
  );

  pcie_bar bar (
      .clk(aclk), .rst(~aresetn),
      .awid(awid), .awaddr(awaddr[31:0]), .awlen(awlen), .awsize(awsize),
      .awburst(awburst), .awlock(awlock), .awcache(awcache),
      .awprot(awprot), .awvalid(awvalid), .awready(awready),
      .wdata(wdata), .wstrb(wstrb), .wlast(wlast), .wvalid(wvalid),
      .wready(wready),
      .bid(bid), .bresp(bresp), .bvalid(bvalid), .bready(bready),
      .arid(arid), .araddr(araddr[31:0]), .arlen(arlen), .arsize(arsize),
      .arburst(arburst), .arlock(arlock), .arcache(arcache),
      .arprot(arprot), .arvalid(arvalid), .arready(arready),
      .rid(rid), .rdata(rdata), .rresp(rresp), .rlast(rlast),
      .rvalid(rvalid), .rready(rready),
      .leds(leds)
  );

  assign led1 = ~link_up;
  assign led2 = ~aresetn;
  assign led3 = ~leds[0];
  assign led4 = ~leds[1];
endmodule
`default_nettype wire
