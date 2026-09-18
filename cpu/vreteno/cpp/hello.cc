// SPDX-License-Identifier: Apache-2.0
//
// Hello world, in C++, for Vreteno: compiled for the core itself by
// the toolchain the build fetches, with the standard library that
// toolchain carries for exactly this machine.
//
// What the standard library means here is worth saying plainly. The
// headers that are templates and constants are all usable, and this
// file uses them: <array>, <algorithm>, <string_view>, <cstdint>.
// What is not usable is the half of it that needs an operating
// system underneath. There is no heap, so no container that allocates;
// no file descriptors, so no <iostream>; no unwinder, so no
// exceptions. A freestanding C++ is most of the language and a good
// deal of the library, and none of the runtime.
#include <algorithm>
#include <array>
#include <cstdint>
#include <string_view>

namespace {

// The serial port: the byte to send, then the status, whose bit zero
// is high while a frame is still going out.
volatile std::uint32_t *const kUart =
    reinterpret_cast<volatile std::uint32_t *>(0x3000);

void Put(char c) {
  while (kUart[1] & 1) {
  }
  kUart[0] = static_cast<std::uint32_t>(static_cast<unsigned char>(c));
}

void Say(std::string_view s) {
  std::for_each(s.begin(), s.end(), Put);
}

// Something computed by the compiler rather than by the core, to say
// that this is C++ and not C with a different suffix. The digits of a
// number, worked out at compile time into an array that lands in the
// data memory as constants.
constexpr int kAnswer = []() {
  std::array<int, 8> a{1, 2, 3, 4, 5, 6, 7, 8};
  int total = 0;
  for (int v : a) total += v * v;
  return total;
}();

// The same, as text, also at compile time.
constexpr auto kAnswerText = []() {
  std::array<char, 4> out{};
  int n = kAnswer;
  for (int i = 2; i >= 0; --i) {
    out[static_cast<std::size_t>(i)] = static_cast<char>('0' + n % 10);
    n /= 10;
  }
  out[3] = '\0';
  return out;
}();

}  // namespace

extern "C" [[noreturn]] void VretenoMain() {
  Say("hello from c++\n");
  Say("the squares to eight sum to ");
  Say(kAnswerText.data());
  Put('\n');
  // A write of one to `mhalt` stops this core. It is how a program
  // says it is done; `ebreak` is a breakpoint and traps.
  for (;;) __asm__ volatile("csrwi 0x7c0, 1");
}

// The reset vector. Nothing sets the stack pointer on this machine and
// nothing clears the uninitialised data, so this does both before any
// compiled code runs, and it is `.text.init` so the linker puts it at
// address zero whatever the optimiser does with the rest.
extern "C" [[noreturn, gnu::naked, gnu::section(".text.init")]] void
_start() {
  __asm__ volatile(
      "la sp, __stack_top\n"
      "la t0, __bss_start\n"
      "la t1, __bss_end\n"
      "1:\n"
      "beq t0, t1, 2f\n"
      "sw zero, 0(t0)\n"
      "addi t0, t0, 4\n"
      "j 1b\n"
      "2:\n"
      "j VretenoMain\n");
}
