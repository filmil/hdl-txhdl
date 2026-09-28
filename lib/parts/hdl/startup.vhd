-- SPDX-License-Identifier: Apache-2.0
-- The same module as startup.v, as an entity, for the VHDL a lowered
-- netlist writes to bind its component to. It is the simulation model
-- only: the primitive is instantiated from the Verilog, which is what
-- a board synthesises.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity startup is
  generic (EOS_AFTER : integer := 8);
  port (
    clk : in std_logic;
    usrcclko : in std_logic;
    eos : out std_logic;
    cclk : out std_logic
  );
end entity;

architecture model of startup is
  signal count : unsigned(3 downto 0) := (others => '0');
  signal eos_r : std_logic := '0';
  signal prev : std_logic := '0';
  signal seen : unsigned(1 downto 0) := (others => '0');
  signal cclk_r : std_logic := '0';
begin
  eos <= eos_r;
  cclk <= cclk_r;
  process (clk)
  begin
    if rising_edge(clk) then
      if count /= EOS_AFTER then
        count <= count + 1;
      else
        eos_r <= '1';
      end if;
      prev <= usrcclko;
      if eos_r = '1' and usrcclko = '1' and prev = '0' and seen /= 3 then
        seen <= seen + 1;
      end if;
      if eos_r = '1' and seen = 3 then
        cclk_r <= usrcclko;
      else
        cclk_r <= '0';
      end if;
    end if;
  end process;
end architecture;
