// SPDX-License-Identifier: Apache-2.0
//
// AskWindow (eth_window.h) against a model of LiteMaster's rule: an
// ask is performed only when its tag follows the last one performed,
// any other is answered from the last answer, and a write's answer
// carries the count of failures. The model loses frames both ways, and
// every write must still reach the memory once, in order.
#include "eth_window.h"

#include <cstdio>
#include <cstdlib>
#include <deque>
#include <map>
#include <random>
#include <stdexcept>
#include <vector>

using namespace eth_frame;

static int failures = 0;

static void expect(bool ok, const char *what) {
  if (!ok) {
    fprintf(stderr, "FAILED: %s\n", what);
    failures++;
  }
}

namespace {

// LiteMaster, its memory and the wire, frame by frame.
struct Board {
  std::mt19937 rng{1};
  double lose_ask = 0;    // the chance an ask is lost on its way in
  double lose_answer = 0; // the chance an answer is lost on its way out
  bool have_last = false;
  uint8_t last_tag = 0;
  Answer last{};
  uint32_t fails = 0;
  std::map<uint32_t, uint32_t> memory;
  std::vector<std::pair<uint32_t, uint32_t>> writes; // as performed
  std::deque<Answer> wire;                           // answers on the way
  uint64_t asks_seen = 0;

  bool chance(double p) {
    return std::uniform_real_distribution<double>(0, 1)(rng) < p;
  }

  void take(const uint8_t *f) {
    if (chance(lose_ask)) {
      return;
    }
    asks_seen++;
    const uint8_t tag = f[16];
    const bool write = f[17] & 1;
    const uint32_t addr = (uint32_t(f[18]) << 24) | (uint32_t(f[19]) << 16) |
                          (uint32_t(f[20]) << 8) | f[21];
    const uint32_t data = (uint32_t(f[22]) << 24) | (uint32_t(f[23]) << 16) |
                          (uint32_t(f[24]) << 8) | f[25];
    if (!have_last || tag == uint8_t(last_tag + 1)) {
      // Performed. The memory refuses at and above 0x100000.
      const bool err = addr >= 0x100000;
      fails += err ? 1 : 0;
      uint32_t word = fails;
      if (write) {
        if (!err) {
          memory[addr] = data;
        }
        writes.push_back({addr, data});
      } else {
        word = err ? 0 : memory[addr];
      }
      have_last = true;
      last_tag = tag;
      last = Answer{5, tag, err, word};
    }
    if (have_last && !chance(lose_answer)) {
      wire.push_back(last);
    }
  }
};

struct Stopped : std::runtime_error {
  using std::runtime_error::runtime_error;
};

AskWindow window_on(Board &b, int window) {
  const uint8_t mac[6] = {0x02, 0, 0, 0, 0, 0x10};
  return AskWindow(
      mac, 5, window, 1, 100, [&b](const uint8_t *f) { b.take(f); },
      [&b](int, Answer *a) {
        if (b.wire.empty()) {
          return false; // the wait runs out at once
        }
        *a = b.wire.front();
        b.wire.pop_front();
        return true;
      },
      [](const std::string &why) { throw Stopped(why); });
}

// A load of `n` words through `w`, then a read of each tenth back.
bool load_and_check(Board &b, AskWindow &w, uint32_t n) {
  for (uint32_t i = 0; i < n; i++) {
    w.write(4 * i, 0x1000000 + i);
  }
  bool ok = true;
  for (uint32_t i = 0; i < n; i += 10) {
    ok &= w.read(4 * i) == 0x1000000 + i;
  }
  ok &= b.writes.size() == n;
  for (uint32_t i = 0; i < n && i < b.writes.size(); i++) {
    ok &= b.writes[i].first == 4 * i && b.writes[i].second == 0x1000000 + i;
  }
  return ok;
}

} // namespace

int main() {
  {
    // A clean wire: every write once, in order, and few asks over the
    // writes and reads themselves.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(200, 0, 10);
    expect(load_and_check(b, w, 1000), "clean: every write once, in order");
    expect(w.resends == 0, "clean: nothing sent again");
  }
  {
    // A board that kept tag 77 from an earlier run: the first read is
    // answered from it, and the count goes on from 77.
    Board b;
    b.have_last = true;
    b.last_tag = 77;
    b.last = Answer{5, 77, false, 0};
    AskWindow w = window_on(b, 32);
    const Answer a = w.start(3, 0, 10);
    expect(a.tag == 77, "the first answer names the board's last tag");
    expect(load_and_check(b, w, 300), "after an earlier run");
  }
  for (int seed = 1; seed <= 20; seed++) {
    // One frame in ten lost each way.
    Board b;
    b.rng.seed(seed);
    b.lose_ask = 0.1;
    b.lose_answer = 0.1;
    AskWindow w = window_on(b, 32);
    w.start(9, 0, 1000);
    char what[64];
    snprintf(what, sizeof(what), "lossy, seed %d: every write once", seed);
    expect(load_and_check(b, w, 2000), what);
  }
  {
    // The tags wrap many times over a long load.
    Board b;
    b.lose_answer = 0.05;
    AskWindow w = window_on(b, 128);
    w.start(250, 0, 10);
    expect(load_and_check(b, w, 5000), "window of 128, tags wrapping");
  }
  {
    // A refused write whose own answer is lost still stops the run, at
    // the next answer.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    w.write(0x10, 1); // the count of failures is learned here
    b.lose_answer = 1;
    w.write(0x200000, 1); // refused, and its answer lost
    b.lose_answer = 0;
    bool stopped = false;
    try {
      w.write(0x14, 2);
      w.flush();
    } catch (const Stopped &) {
      stopped = true;
    }
    expect(stopped, "a refused write is seen in a later answer");
  }
  {
    // A refused read stops the run.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    bool stopped = false;
    try {
      (void)w.read(0x300000);
    } catch (const Stopped &) {
      stopped = true;
    }
    expect(stopped, "a refused read");
  }
  {
    // A board that never answers stops the run after the tries.
    Board b;
    b.lose_answer = 1;
    AskWindow w = window_on(b, 32);
    bool stopped = false;
    try {
      w.start(1, 0, 10);
    } catch (const Stopped &) {
      stopped = true;
    }
    expect(stopped, "no answer at all");
  }
  if (failures) {
    return 1;
  }
  printf("eth_window_test: passed\n");
  return 0;
}
