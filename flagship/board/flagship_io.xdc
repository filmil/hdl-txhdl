# SPDX-License-Identifier: Apache-2.0
# The flagship's Ethernet and HDMI pins, timed (issue 861). Without this,
# check_timing counted five inputs with no input delay and 38 outputs
# with no output delay, and timing was met only because those paths were
# not timed at all. The board's other pins are timed or false-pathed by
# cpu/vreteno/board/ax7a200_io.xdc (#847).

# ---------------------------------------------------------------------
# RGMII, to the JL2121 PHY, at gigabit: 125 MHz, a nibble on each edge.
#
# The JL2121's own data sheet could not be found on 2026-10-02, so the
# figures below are RGMII 2.0's and are placeholders, to be confirmed
# against the JL2121's sheet. The board's traces are short and were not
# measured; 100 ps of mismatch is allowed for in each.

# Receive. The PHY's clock reaches the pins centred in its data: RGMII
# 2.0 with the delay in the source, data valid TsetupT = 1.2 ns before
# and TholdT = 1.2 ns after each edge, less the 100 ps, so 1.1 ns.
#
# That is not what #231 concluded, and the board's own record is why.
# The design that received nothing clocked the IDDRs through a plain
# BUFG, about 3.4 ns late, which samples where a centred source changes
# too, so it never showed that the PHY adds no delay. Every point of the
# sweep that followed fits a centred source: through the MMCM, whose
# compensation removes the insertion delay, shifts of 45 to 112.5
# degrees received and 180 did not. Against RGMII's edge-aligned figures
# instead, data within 500 ps of the edge, the shift that passes is
# 170 degrees, where the board received nothing.
#
# Each nibble is stated against the edge it is centred on: it begins
# between 2.9 and 1.1 ns before that edge. eth_rgmii shifts the clock
# in an MMCM by RX_CLOCK_PHASE, set in flagship.v, and the IDDRs take
# each nibble on the shifted copy of its edge, with the nibble before it
# on the edge before. With Vivado's routed delays, setup slack is
# phase - 1.58 ns and hold slack 1.96 ns - phase, so 90 degrees, 2 ns,
# misses hold by 40 ps and 78.75 degrees, 1.75 ns, meets both.
set rgmii_valid 1.1
set rx_in [get_ports {eth_rxd[*] eth_rxctl}]
set_input_delay -clock [get_clocks eth_rxck] \
    -max [expr {-$rgmii_valid}] $rx_in
set_input_delay -clock [get_clocks eth_rxck] \
    -min [expr {$rgmii_valid - 4.0}] $rx_in
set_input_delay -clock [get_clocks eth_rxck] -clock_fall \
    -max [expr {-$rgmii_valid}] -add_delay $rx_in
set_input_delay -clock [get_clocks eth_rxck] -clock_fall \
    -min [expr {$rgmii_valid - 4.0}] -add_delay $rx_in

# Transmit. eth_rgmii sends the clock through an ODDR on the same clock
# as the data, so the two leave edge-aligned and the PHY delays the
# clock, which #231 measured: with a quarter cycle added here as well
# the PHY sampled where the data changes. The source's figure is RGMII
# 2.0's TskewT, data within 500 ps of the clock edge either way, plus the
# 100 ps. The forwarded clock is created at the pin, and the outputs are
# AMD's skew-based template for an edge-aligned DDR output, whose false
# paths keep each edge's data checked against the edge it goes with.
set rgmii_skew 0.6
set eth_src [get_clocks -of_objects [get_pins rgmii/txck_oddr/C]]
create_generated_clock -name eth_txck_fwd \
    -source [get_pins rgmii/txck_oddr/C] -multiply_by 1 \
    [get_ports eth_txck]
set tx_out [get_ports {eth_txd[*] eth_txctl}]
set_output_delay -clock eth_txck_fwd -max [expr {4.0 - $rgmii_skew}] $tx_out
set_output_delay -clock eth_txck_fwd -min $rgmii_skew $tx_out
set_output_delay -clock eth_txck_fwd -clock_fall \
    -max [expr {4.0 - $rgmii_skew}] -add_delay $tx_out
set_output_delay -clock eth_txck_fwd -clock_fall \
    -min $rgmii_skew -add_delay $tx_out
set_false_path -setup -rise_from $eth_src -rise_to [get_clocks eth_txck_fwd]
set_false_path -setup -fall_from $eth_src -fall_to [get_clocks eth_txck_fwd]
set_false_path -hold -rise_from $eth_src -fall_to [get_clocks eth_txck_fwd]
set_false_path -hold -fall_from $eth_src -rise_to [get_clocks eth_txck_fwd]

# ---------------------------------------------------------------------
# HDMI, to the SiI9134 encoder, at the 25.2 MHz pixel clock.
#
# The pixel, the syncs and the enable leave on the pixel clock's rise,
# and the encoder's input clock, IDCK, is the pixel clock turned over by
# an ODDR. The encoder latches on the edge its EDGE bit chooses. The
# figures are the SiI9134 data sheet's (SiI-DS-0193-D, single-edge
# clocking): setup 1.0 ns and hold 0.5 ns to IDCK rising, EDGE = 1;
# setup 1.0 ns and hold 0.8 ns to IDCK falling, EDGE = 0. The design
# writes 0x35 to the system control register, 0x08, and no document in
# the tree states which bit of it is EDGE, so both edges are checked
# here: whichever the chip uses, its window is met.
create_generated_clock -name hdmi_idck \
    -source [get_pins clk_oddr/C] -multiply_by 1 -invert \
    [get_ports hdmi_clk]
set vid_out [get_ports {hdmi_d[*] hdmi_de hdmi_hs hdmi_vs}]
set_output_delay -clock hdmi_idck -max 1.0 $vid_out
set_output_delay -clock hdmi_idck -min -0.5 $vid_out
set_output_delay -clock hdmi_idck -clock_fall -max 1.0 -add_delay $vid_out
set_output_delay -clock hdmi_idck -clock_fall -min -0.8 -add_delay $vid_out

# The encoder's configuration bus and its resets are levels and an I2C
# bus at a few hundred kilohertz, driven and read on the pixel clock
# with many cycles to every change; nothing times them against a clock.
set_false_path -to [get_ports {hdmi_scl hdmi_sda hdmi_nreset hdmi_nreset_alt}]
set_false_path -from [get_ports {hdmi_sda}]
