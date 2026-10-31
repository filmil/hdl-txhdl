// SPDX-License-Identifier: Apache-2.0
//
// AskWindow (eth_window.h) against a model of LiteMaster on a wire with
// latency. The new bridge performs an ask only when its tag follows the
// last one performed, answers any other from the last answer, names the
// ask each answer answers, and counts failures in a write's answer. The
// old bridge, from before issue 2019, performs every ask but a repeat of
// the last tag and names nothing. The wire takes time each way and can
// lose frames, so answers are in flight when the window times out, and
// stale copies come back. Every write must reach the memory once, in
// order.
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

// Steps of the model's clock per millisecond of the window's timeout.
constexpr uint64_t STEPS_PER_MS = 10;

struct Board {
  std::mt19937 rng{1};
  bool old_bridge = false;
  uint64_t latency = 3;   // steps each way; a timeout is 10
  double lose_ask = 0;    // the chance an ask is lost on its way in
  double lose_answer = 0; // the chance an answer is lost on its way out
  bool silent = false;    // answers nothing at all
  bool have_last = false;
  uint8_t last_tag = 0;
  Answer last{};
  uint32_t fails = 0;
  std::map<uint32_t, uint32_t> memory;
  std::vector<std::pair<uint32_t, uint32_t>> writes; // as performed
  uint64_t now = 0;
  std::deque<std::pair<uint64_t, std::vector<uint8_t>>> asks; // in flight
  std::deque<std::pair<uint64_t, Answer>> answers;            // in flight

  bool chance(double p) {
    return std::uniform_real_distribution<double>(0, 1)(rng) < p;
  }

  void send(const uint8_t *f) {
    if (!chance(lose_ask)) {
      asks.push_back({now + latency, std::vector<uint8_t>(f, f + MIN_FRAME)});
    }
  }

  void take(const std::vector<uint8_t> &f) {
    const uint8_t tag = f[16];
    const bool write = f[17] & 1;
    const uint32_t addr = (uint32_t(f[18]) << 24) | (uint32_t(f[19]) << 16) |
                          (uint32_t(f[20]) << 8) | f[21];
    const uint32_t data = (uint32_t(f[22]) << 24) | (uint32_t(f[23]) << 16) |
                          (uint32_t(f[24]) << 8) | f[25];
    const bool perform =
        old_bridge ? !(have_last && tag == last_tag)
                   : !have_last || tag == uint8_t(last_tag + 1);
    if (perform) {
      // The memory refuses at and above 0x100000.
      const bool err = addr >= 0x100000;
      fails += err ? 1 : 0;
      uint32_t word = old_bridge ? 0 : fails;
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
      last = Answer{5, tag, err, word, 0};
    }
    if (have_last && !silent && !chance(lose_answer)) {
      Answer a = last;
      a.asked = old_bridge ? 0 : tag;
      answers.push_back({now + latency, a});
    }
  }

  // Runs the clock until an answer arrives or `ms` runs out.
  bool receive(int ms, Answer *a) {
    const uint64_t until = now + uint64_t(ms) * STEPS_PER_MS;
    for (;;) {
      while (!asks.empty() && asks.front().first <= now) {
        const auto f = asks.front().second;
        asks.pop_front();
        take(f);
      }
      if (!answers.empty() && answers.front().first <= now) {
        *a = answers.front().second;
        answers.pop_front();
        return true;
      }
      if (now >= until) {
        return false;
      }
      now++;
    }
  }
};

struct Stopped : std::runtime_error {
  using std::runtime_error::runtime_error;
};

AskWindow window_on(Board &b, int window, int retries = 100) {
  const uint8_t mac[6] = {0x02, 0, 0, 0, 0, 0x10};
  return AskWindow(
      mac, 5, window, 1, retries, [&b](const uint8_t *f) { b.send(f); },
      [&b](int ms, Answer *a) { return b.receive(ms, a); },
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

bool stops(const std::function<void()> &f) {
  try {
    f();
  } catch (const Stopped &) {
    return true;
  }
  return false;
}

} // namespace

int main() {
  {
    // A clean wire: every write once, in order, nothing sent again.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(200, 0, 10);
    expect(w.echoes(), "the new bridge names its asks");
    expect(load_and_check(b, w, 1000), "clean: every write once, in order");
    expect(w.resends == 0, "clean: nothing sent again");
  }
  {
    // A wire slower than the timeout: answers are in flight each time
    // the window is sent again, and their stale copies come back. They
    // must not set off another round, so the asks sent again stay
    // within a few per write.
    Board b;
    b.latency = 8;
    AskWindow w = window_on(b, 32);
    w.start(7, 0, 100);
    expect(load_and_check(b, w, 2000), "slow wire: every write once");
    char what[96];
    snprintf(what, sizeof(what), "slow wire: %llu asks sent again for 2000",
             static_cast<unsigned long long>(w.resends));
    expect(w.resends < 4000, what);
  }
  {
    // The same at the first ask: copies of it answered late are passed
    // over, and the first write is not sent again for them.
    Board b;
    b.latency = 8;
    AskWindow w = window_on(b, 8);
    w.start(9, 0, 100);
    w.write(0, 1);
    expect(b.writes.size() == 1, "the first write once");
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
    // A board that kept tag 77 from an earlier run: the first read is
    // answered from it, and the count goes on from 77.
    Board b;
    b.have_last = true;
    b.last_tag = 77;
    b.last = Answer{5, 77, false, 0, 0};
    AskWindow w = window_on(b, 32);
    const Answer a = w.start(3, 0, 10);
    expect(a.tag == 77, "the first answer names the board's last tag");
    expect(load_and_check(b, w, 300), "after an earlier run");
  }
  {
    // An answer from before this run, waiting on the wire, is passed
    // over by the first ask.
    Board b;
    b.answers.push_back({0, Answer{5, 40, false, 0, 41}});
    AskWindow w = window_on(b, 32);
    const Answer a = w.start(3, 0, 10);
    expect(a.tag == 3, "a stale answer is not the first");
    expect(load_and_check(b, w, 100), "after a stale answer");
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
    // A window outside 1 to 128 is kept inside it.
    Board b;
    AskWindow w = window_on(b, 1000);
    w.start(5, 0, 10);
    expect(load_and_check(b, w, 1000), "a window of 1000 is kept to 128");
  }
  for (int seed = 1; seed <= 5; seed++) {
    // The old bridge: no names, so each write waits, and the loads are
    // still right with frames lost.
    Board b;
    b.rng.seed(seed);
    b.old_bridge = true;
    b.lose_ask = 0.1;
    b.lose_answer = 0.1;
    AskWindow w = window_on(b, 32);
    w.start(9, 0, 1000);
    expect(!w.echoes(), "the old bridge names nothing");
    char what[64];
    snprintf(what, sizeof(what), "old bridge, seed %d: every write once", seed);
    expect(load_and_check(b, w, 500), what);
  }
  {
    // A refused write whose own answer is lost still stops the run, at
    // the next answer.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    w.write(0x10, 1); // the count of failures is learned here
    b.lose_answer = 1;
    expect(stops([&] {
             w.write(0x200000, 1); // refused, and its answer lost
             b.lose_answer = 0;
             w.write(0x14, 2);
             w.flush();
           }),
           "a refused write is seen in a later answer");
  }
  {
    // On the old bridge, whose write answers carry no count, a refused
    // write after the first still stops the run.
    Board b;
    b.old_bridge = true;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    w.write(0x10, 1);
    expect(stops([&] { w.write(0x200000, 1); }),
           "old bridge: a refused write after the first");
  }
  {
    // A refused first write stops the run.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    expect(stops([&] { w.write(0x200000, 1); }), "a refused first write");
  }
  {
    // A board that counted failures before this run: no false stop.
    Board b;
    b.fails = 5;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    expect(!stops([&] { (void)load_and_check(b, w, 100); }),
           "earlier failures are not this run's");
  }
  {
    // A refused read, and a refused first read, stop the run.
    Board b;
    AskWindow w = window_on(b, 32);
    w.start(1, 0, 10);
    expect(stops([&] { (void)w.read(0x300000); }), "a refused read");
    Board c;
    AskWindow v = window_on(c, 32);
    expect(stops([&] { v.start(1, 0x300000, 10); }), "a refused first read");
  }
  {
    // A board that never answers stops the run at the first ask, and
    // one that goes quiet in a load stops it after the retries.
    Board b;
    b.silent = true;
    AskWindow w = window_on(b, 32);
    expect(stops([&] { w.start(1, 0, 10); }), "no answer at all");
    Board c;
    AskWindow v = window_on(c, 32, 5);
    v.start(1, 0, 10);
    expect(stops([&] {
             v.write(0, 1);
             c.silent = true;
             for (uint32_t i = 1; i < 100; i++) {
               v.write(4 * i, i);
             }
             v.flush();
           }),
           "a board gone quiet in a load");
  }
  if (failures) {
    return 1;
  }
  printf("eth_window_test: passed\n");
  return 0;
}
