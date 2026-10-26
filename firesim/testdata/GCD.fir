FIRRTL version 1.2.0
circuit GCD :
  extmodule RationalClockBridge :
    output clocks : Clock[1]
    defname = RationalClockBridge

  module GCDInner :
    input clock : Clock
    input reset : Reset
    output io : { flip a : UInt<16>, flip b : UInt<16>, flip e : UInt<1>, z : UInt<16>, v : UInt<1>} @[sim/src/main/scala/midasexamples/GCD.scala 20:14]

    reg x : UInt, clock with :
      reset => (UInt<1>("h0"), x) @[sim/src/main/scala/midasexamples/GCD.scala 21:15]
    reg y : UInt, clock with :
      reset => (UInt<1>("h0"), y) @[sim/src/main/scala/midasexamples/GCD.scala 22:15]
    node _T = gt(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 23:10]
    when _T : @[sim/src/main/scala/midasexamples/GCD.scala 23:15]
      node _x_T = sub(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 23:24]
      node _x_T_1 = tail(_x_T, 1) @[sim/src/main/scala/midasexamples/GCD.scala 23:24]
      x <= _x_T_1 @[sim/src/main/scala/midasexamples/GCD.scala 23:19]
    node _T_1 = leq(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 24:10]
    when _T_1 : @[sim/src/main/scala/midasexamples/GCD.scala 24:16]
      node _y_T = sub(y, x) @[sim/src/main/scala/midasexamples/GCD.scala 24:25]
      node _y_T_1 = tail(_y_T, 1) @[sim/src/main/scala/midasexamples/GCD.scala 24:25]
      y <= _y_T_1 @[sim/src/main/scala/midasexamples/GCD.scala 24:20]
    when io.e : @[sim/src/main/scala/midasexamples/GCD.scala 25:14]
      x <= io.a @[sim/src/main/scala/midasexamples/GCD.scala 25:18]
      y <= io.b @[sim/src/main/scala/midasexamples/GCD.scala 25:29]
    io.z <= x @[sim/src/main/scala/midasexamples/GCD.scala 26:8]
    node _io_v_T = eq(y, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 27:13]
    io.v <= _io_v_T @[sim/src/main/scala/midasexamples/GCD.scala 27:8]
    node _T_2 = asUInt(reset) @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
    node _T_3 = eq(_T_2, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
    when _T_3 : @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
      printf(clock, UInt<1>("h1"), "X: %d, Y:%d\n", x, y) : printf @[sim/src/main/scala/midasexamples/GCD.scala 30:9]


  module GCDInner_1 :
    input clock : Clock
    input reset : Reset
    output io : { flip a : UInt<16>, flip b : UInt<16>, flip e : UInt<1>, z : UInt<16>, v : UInt<1>} @[sim/src/main/scala/midasexamples/GCD.scala 20:14]

    reg x : UInt, clock with :
      reset => (UInt<1>("h0"), x) @[sim/src/main/scala/midasexamples/GCD.scala 21:15]
    reg y : UInt, clock with :
      reset => (UInt<1>("h0"), y) @[sim/src/main/scala/midasexamples/GCD.scala 22:15]
    node _T = gt(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 23:10]
    when _T : @[sim/src/main/scala/midasexamples/GCD.scala 23:15]
      node _x_T = sub(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 23:24]
      node _x_T_1 = tail(_x_T, 1) @[sim/src/main/scala/midasexamples/GCD.scala 23:24]
      x <= _x_T_1 @[sim/src/main/scala/midasexamples/GCD.scala 23:19]
    node _T_1 = leq(x, y) @[sim/src/main/scala/midasexamples/GCD.scala 24:10]
    when _T_1 : @[sim/src/main/scala/midasexamples/GCD.scala 24:16]
      node _y_T = sub(y, x) @[sim/src/main/scala/midasexamples/GCD.scala 24:25]
      node _y_T_1 = tail(_y_T, 1) @[sim/src/main/scala/midasexamples/GCD.scala 24:25]
      y <= _y_T_1 @[sim/src/main/scala/midasexamples/GCD.scala 24:20]
    when io.e : @[sim/src/main/scala/midasexamples/GCD.scala 25:14]
      x <= io.a @[sim/src/main/scala/midasexamples/GCD.scala 25:18]
      y <= io.b @[sim/src/main/scala/midasexamples/GCD.scala 25:29]
    io.z <= x @[sim/src/main/scala/midasexamples/GCD.scala 26:8]
    node _io_v_T = eq(y, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 27:13]
    io.v <= _io_v_T @[sim/src/main/scala/midasexamples/GCD.scala 27:8]
    node _T_2 = asUInt(reset) @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
    node _T_3 = eq(_T_2, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
    when _T_3 : @[sim/src/main/scala/midasexamples/GCD.scala 30:9]
      printf(clock, UInt<1>("h1"), "X: %d, Y:%d\n", x, y) : printf @[sim/src/main/scala/midasexamples/GCD.scala 30:9]


  module GCDDUT :
    input clock : Clock
    input reset : Reset
    output io : { flip a : UInt<16>, flip b : UInt<16>, flip e : UInt<1>, z : UInt<16>, v : UInt<1>} @[sim/src/main/scala/midasexamples/GCD.scala 35:18]

    inst inner1 of GCDInner @[sim/src/main/scala/midasexamples/GCD.scala 36:22]
    inner1.clock <= clock
    inner1.reset <= reset
    inst inner2 of GCDInner_1 @[sim/src/main/scala/midasexamples/GCD.scala 38:22]
    inner2.clock <= clock
    inner2.reset <= reset
    reg select : UInt<1>, clock with :
      reset => (reset, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 41:23]
    node _select_T = eq(select, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 42:13]
    select <= _select_T @[sim/src/main/scala/midasexamples/GCD.scala 42:10]
    reg done1 : UInt<1>, clock with :
      reset => (reset, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 44:24]
    reg result1 : UInt, clock with :
      reset => (UInt<1>("h0"), result1) @[sim/src/main/scala/midasexamples/GCD.scala 45:20]
    when io.v : @[sim/src/main/scala/midasexamples/GCD.scala 47:14]
      done1 <= UInt<1>("h0") @[sim/src/main/scala/midasexamples/GCD.scala 48:11]
    else :
      when inner1.io.v : @[sim/src/main/scala/midasexamples/GCD.scala 49:27]
        done1 <= UInt<1>("h1") @[sim/src/main/scala/midasexamples/GCD.scala 50:13]
        result1 <= inner1.io.z @[sim/src/main/scala/midasexamples/GCD.scala 51:13]
    reg done2 : UInt<1>, clock with :
      reset => (reset, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 54:24]
    reg result2 : UInt, clock with :
      reset => (UInt<1>("h0"), result2) @[sim/src/main/scala/midasexamples/GCD.scala 55:20]
    when io.v : @[sim/src/main/scala/midasexamples/GCD.scala 57:14]
      done2 <= UInt<1>("h0") @[sim/src/main/scala/midasexamples/GCD.scala 58:11]
    else :
      when inner2.io.v : @[sim/src/main/scala/midasexamples/GCD.scala 59:27]
        done2 <= UInt<1>("h1") @[sim/src/main/scala/midasexamples/GCD.scala 60:13]
        result2 <= inner2.io.z @[sim/src/main/scala/midasexamples/GCD.scala 61:13]
    inner1.io.a <= io.a @[sim/src/main/scala/midasexamples/GCD.scala 64:15]
    inner1.io.b <= io.b @[sim/src/main/scala/midasexamples/GCD.scala 65:15]
    inner1.io.e <= io.e @[sim/src/main/scala/midasexamples/GCD.scala 66:15]
    inner2.io.a <= io.b @[sim/src/main/scala/midasexamples/GCD.scala 68:15]
    inner2.io.b <= io.a @[sim/src/main/scala/midasexamples/GCD.scala 69:15]
    inner2.io.e <= io.e @[sim/src/main/scala/midasexamples/GCD.scala 70:15]
    node _io_z_T = mux(select, result1, result2) @[sim/src/main/scala/midasexamples/GCD.scala 72:14]
    io.z <= _io_z_T @[sim/src/main/scala/midasexamples/GCD.scala 72:8]
    node _io_v_T = and(done1, done2) @[sim/src/main/scala/midasexamples/GCD.scala 73:17]
    io.v <= _io_v_T @[sim/src/main/scala/midasexamples/GCD.scala 73:8]
    node _T = eq(done1, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 75:10]
    node _T_1 = eq(done2, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 75:20]
    node _T_2 = or(_T, _T_1) @[sim/src/main/scala/midasexamples/GCD.scala 75:17]
    node _T_3 = eq(result1, result2) @[sim/src/main/scala/midasexamples/GCD.scala 75:39]
    node _T_4 = or(_T_2, _T_3) @[sim/src/main/scala/midasexamples/GCD.scala 75:27]
    node _T_5 = asUInt(reset) @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
    node _T_6 = eq(_T_5, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
    when _T_6 : @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
      node _T_7 = eq(_T_4, UInt<1>("h0")) @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
      when _T_7 : @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
        printf(clock, UInt<1>("h1"), "Assertion failed: Outputs do not match!\n    at GCD.scala:75 assert(!done1 || !done2 || (result1 === result2), \"Outputs do not match!\")\n") : printf @[sim/src/main/scala/midasexamples/GCD.scala 75:9]
      assert(clock, _T_4, UInt<1>("h1"), "") : assert @[sim/src/main/scala/midasexamples/GCD.scala 75:9]


  extmodule PeekPokeBridge :
    input io : { flip a : UInt<16>, flip b : UInt<16>, flip e : UInt<1>, z : UInt<16>, v : UInt<1>}
    output reset : UInt<1>
    input clock : Clock
    defname = PeekPokeBridge

  module GCD :

    inst RationalClockBridge of RationalClockBridge @[sim/firesim-lib/src/main/scala/bridges/ClockBridge.scala 82:11]
    wire reset : UInt<1> @[sim/firesim-lib/src/main/scala/testutils/PeekPokeHarness.scala 15:23]
    reset <= UInt<1>("h0") @[sim/firesim-lib/src/main/scala/testutils/PeekPokeHarness.scala 15:23]
    inst dut of GCDDUT @[sim/firesim-lib/src/main/scala/testutils/PeekPokeHarness.scala 18:21]
    dut.clock <= RationalClockBridge.clocks[0]
    dut.reset <= reset
    wire directionedReset : UInt<1> @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 80:32]
    inst peekPokeBridge of PeekPokeBridge @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 82:34]
    directionedReset <= peekPokeBridge.reset @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    peekPokeBridge.io.v <= dut.io.v @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    peekPokeBridge.io.z <= dut.io.z @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    dut.io.e <= peekPokeBridge.io.e @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    dut.io.b <= peekPokeBridge.io.b @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    dut.io.a <= peekPokeBridge.io.a @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 83:58]
    peekPokeBridge.clock <= RationalClockBridge.clocks[0] @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 84:29]
    reset <= directionedReset @[sim/firesim-lib/src/main/scala/bridges/PeekPokeBridge.scala 85:11]
