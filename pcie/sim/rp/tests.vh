// SPDX-License-Identifier: Apache-2.0
// The test the root port's host runs against the board's endpoint
// (issue 178). The host, `pci_exp_usrapp_tx`, includes this file as
// one arm of its choice of test, once it has trained the link,
// programmed the BARs and looked for an XDMA register BAR, which the
// bridge has none of. The arm reads BAR1, the bypass window, through
// to `BarRegs`: the
// two words of the identifier, a word written to the scratch register
// and read back, and the count of writes, which that one write made
// one. It sets the board's `ok` when the link is up and every word is
// the one the design keeps.
else if (testname == "bridge")
begin
    $display("[%t] : bridge: BAR1 is at %h, link_up is %b", $realtime,
             BAR_INIT_P_BAR[1][31:0], board.EP.top.link_up);
    if (board.EP.top.link_up !== 1'b1) testError = 1'b1;

    // The identifier, 0x5478_4844_4c00_0001, a word at a time.
    P_READ_DATA = 32'hffff_ffff;
    fork
        TSK_TX_MEMORY_READ_32(DEFAULT_TAG, DEFAULT_TC, 11'd1,
                              BAR_INIT_P_BAR[1][31:0] + 32'h0, 4'h0, 4'hF);
        TSK_WAIT_FOR_READ_DATA;
    join
    DEFAULT_TAG = DEFAULT_TAG + 1;
    $display("[%t] : bridge: word 0 reads %h", $realtime, P_READ_DATA);
    if (P_READ_DATA !== 32'h4c00_0001) testError = 1'b1;
    TSK_TX_CLK_EAT(10);

    P_READ_DATA = 32'hffff_ffff;
    fork
        TSK_TX_MEMORY_READ_32(DEFAULT_TAG, DEFAULT_TC, 11'd1,
                              BAR_INIT_P_BAR[1][31:0] + 32'h4, 4'h0, 4'hF);
        TSK_WAIT_FOR_READ_DATA;
    join
    DEFAULT_TAG = DEFAULT_TAG + 1;
    $display("[%t] : bridge: word 1 reads %h", $realtime, P_READ_DATA);
    if (P_READ_DATA !== 32'h5478_4844) testError = 1'b1;
    TSK_TX_CLK_EAT(10);

    // The scratch word, written and read back. The bytes go on the
    // wire lowest address first.
    DATA_STORE[0] = 8'h78;
    DATA_STORE[1] = 8'h56;
    DATA_STORE[2] = 8'h34;
    DATA_STORE[3] = 8'h12;
    TSK_TX_MEMORY_WRITE_32(DEFAULT_TAG, DEFAULT_TC, 11'd1,
                           BAR_INIT_P_BAR[1][31:0] + 32'h8, 4'h0, 4'hF, 1'b0);
    TSK_TX_CLK_EAT(100);
    DEFAULT_TAG = DEFAULT_TAG + 1;

    P_READ_DATA = 32'hffff_ffff;
    fork
        TSK_TX_MEMORY_READ_32(DEFAULT_TAG, DEFAULT_TC, 11'd1,
                              BAR_INIT_P_BAR[1][31:0] + 32'h8, 4'h0, 4'hF);
        TSK_WAIT_FOR_READ_DATA;
    join
    DEFAULT_TAG = DEFAULT_TAG + 1;
    $display("[%t] : bridge: scratch reads %h", $realtime, P_READ_DATA);
    if (P_READ_DATA !== 32'h1234_5678) testError = 1'b1;
    TSK_TX_CLK_EAT(10);

    // The count of writes: the one above.
    P_READ_DATA = 32'hffff_ffff;
    fork
        TSK_TX_MEMORY_READ_32(DEFAULT_TAG, DEFAULT_TC, 11'd1,
                              BAR_INIT_P_BAR[1][31:0] + 32'h18, 4'h0, 4'hF);
        TSK_WAIT_FOR_READ_DATA;
    join
    DEFAULT_TAG = DEFAULT_TAG + 1;
    $display("[%t] : bridge: writes reads %h", $realtime, P_READ_DATA);
    if (P_READ_DATA !== 32'h0000_0001) testError = 1'b1;

    board.ok = ~testError;
    $display("[%t] : bridge: %s", $realtime, testError ? "FAILED" : "PASSED");
    $finish;
end
