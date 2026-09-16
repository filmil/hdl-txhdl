-- SPDX-License-Identifier: Apache-2.0
-- The same module as blinker.v, as an entity, so that the VHDL a
-- lowered netlist writes has something to bind its component to.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity blinker is
  generic (PERIOD : integer := 4);
  port (
    clk : in std_logic;
    enable : in std_logic;
    count : out std_logic_vector(7 downto 0);
    pins : inout std_logic_vector(7 downto 0)
  );
end entity;

architecture rtl of blinker is
  signal n : unsigned(7 downto 0) := (others => '0');
  signal tick : unsigned(7 downto 0) := (others => '0');
begin
  count <= std_logic_vector(n);
  pins <= std_logic_vector(n) when enable = '1' else (others => 'Z');
  process (clk)
  begin
    if rising_edge(clk) then
      if enable = '1' then
        if tick = PERIOD - 1 then
          tick <= (others => '0');
          n <= n + 1;
        else
          tick <= tick + 1;
        end if;
      end if;
    end if;
  end process;
end architecture;
