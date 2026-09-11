-- SPDX-License-Identifier: Apache-2.0
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity hello_tb is
end entity;

architecture sim of hello_tb is
    signal n : unsigned(2 downto 0) := "000";
begin
    process
    begin
        n <= n + 1;
        wait for 1 ns;
        assert n = "001" report "the count did not count" severity failure;
        report "hello: the simulator works";
        std.env.finish;
    end process;
end architecture;
