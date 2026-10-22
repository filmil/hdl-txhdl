-- SPDX-License-Identifier: Apache-2.0
-- offer_tb.v in VHDL: TxHDL's two channel buffers driven by a sender
-- that does not wait for ready, as AXI requires of a source (#1871).
-- Each word must arrive once, in order.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity offer_lane is
  generic (UNREG : boolean; SEED : unsigned(15 downto 0));
  port (
    clk : in std_logic;
    rst : in std_logic;
    sent : out natural;
    got : out natural
  );
end entity;

architecture sim of offer_lane is
  signal word : unsigned(7 downto 0) := (others => '0');
  signal tv : std_logic := '0';
  signal tr, rv, rr : std_logic;
  signal rd : unsigned(7 downto 0);
  -- The same shift register as offer_tb.v: bit 0 and bit 3 decide
  -- whether the sender offers after a word, bits 1 and 2 whether the
  -- receiver is ready.
  signal lfsr : unsigned(15 downto 0) := SEED;
  signal nsent, ngot : natural := 0;
begin
  rr <= lfsr(1) or lfsr(2);
  sent <= nsent;
  got <= ngot;
  u : if UNREG generate
    ch : entity work.pair_u_txhdl_chan_u
      generic map (W => 8)
      port map (clk => clk, rst => rst,
                tx_data => word, tx_valid => tv, tx_ready => tr,
                rx_data => rd, rx_valid => rv, rx_ready => rr);
  end generate;
  r : if not UNREG generate
    ch : entity work.pair_u_txhdl_chan
      generic map (W => 8)
      port map (clk => clk, rst => rst,
                tx_data => word, tx_valid => tv, tx_ready => tr,
                rx_data => rd, rx_valid => rv, rx_ready => rr);
  end generate;
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        word <= (others => '0');
        tv <= '0';
        nsent <= 0;
        ngot <= 0;
        lfsr <= SEED;
      else
        lfsr <= lfsr(14 downto 0) &
                (lfsr(15) xor lfsr(13) xor lfsr(12) xor lfsr(10));
        -- The sender: an offer stays up, with the same word, until
        -- taken.
        if tv = '1' and tr = '1' then
          word <= word + 1;
          nsent <= nsent + 1;
          tv <= lfsr(0) or lfsr(3);
        elsif tv = '0' then
          tv <= lfsr(0) or lfsr(3);
        end if;
        -- The receiver: each word it takes is the next one.
        if rv = '1' and rr = '1' then
          assert to_integer(rd) = ngot mod 256
            report "offer: word " & integer'image(ngot) & " was " &
                   integer'image(to_integer(rd))
            severity failure;
          ngot <= ngot + 1;
        end if;
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity offer_tb is
end entity;

architecture sim of offer_tb is
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal rsent, rgot, usent, ugot : natural;
begin
  reg_lane : entity work.offer_lane
    generic map (UNREG => false, SEED => x"ace1")
    port map (clk => clk, rst => rst, sent => rsent, got => rgot);
  unreg_lane : entity work.offer_lane
    generic map (UNREG => true, SEED => x"beef")
    port map (clk => clk, rst => rst, sent => usent, got => ugot);
  process
  begin
    for cycle in 0 to 3999 loop
      wait for 5 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      if cycle = 2 then
        rst <= '0';
      end if;
    end loop;
    wait for 1 ns;
    -- Nothing lost: what was taken from the sender and not yet by the
    -- receiver is in the buffer, two words at most.
    assert rsent - rgot <= 2 and usent - ugot <= 2
      report "offer: sent " & integer'image(rsent) & " and " &
             integer'image(usent) & ", got " & integer'image(rgot) &
             " and " & integer'image(ugot)
      severity failure;
    -- About 2800 words pass in each; fewer than 2500 is a buffer that
    -- stalls.
    assert rgot >= 2500 and ugot >= 2500
      report "offer: only " & integer'image(rgot) & " and " &
             integer'image(ugot) & " words passed"
      severity failure;
    report "offer: " & integer'image(rgot) & " and " &
           integer'image(ugot) & " words passed, each once, in order";
    wait;
  end process;
end architecture;
