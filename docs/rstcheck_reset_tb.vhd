-- SPDX-License-Identifier: Apache-2.0
-- The decade counter of ex_rstcheck held in its own reset for three
-- cycles, then counting past a wrap, under nvc (issue 633). The check
-- is the netlist's own VHDL assert, of severity failure, so a check
-- stated in reset stops the run. Under reset the counter shows a
-- blank, which its check would reject if the netlist stated it there.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity rstcheck_reset_tb is
end entity;

architecture sim of rstcheck_reset_tb is
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal tick : std_logic := '1';
  signal shown : unsigned(7 downto 0);
begin
  uut : entity work.decade port map (
    clk => clk, rst => rst, tick => tick, shown => shown);

  clock : process
  begin
    for i in 1 to 30 loop
      clk <= '1'; wait for 1 ns;
      clk <= '0'; wait for 1 ns;
    end loop;
    wait;
  end process;

  stim : process
  begin
    wait for 5.5 ns;
    rst <= '0';
    wait for 50 ns;
    assert shown <= 9 report "a blank shown out of reset" severity failure;
    report "no check failed across the reset";
    std.env.finish;
  end process;
end architecture;
