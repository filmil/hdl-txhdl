// SPDX-License-Identifier: Apache-2.0
//
// Register writes in flight on the control path (issue 2019).
//
// LiteMaster (lib/parts/src/remote/host.rs) performs an ask only when
// its tag follows the last one it performed, and answers any other ask
// from the last answer it kept. So the host can send many writes before
// it waits:
//
// - an answer says that every ask up to its tag is done;
// - an answer that names the last tag again, while later asks are out,
//   says that one of them was lost, and the window is sent again from
//   the first ask not yet answered;
// - if nothing comes back in time, the window is sent again as well.
//
// The answer to a write carries the number of failed transactions so
// far, so a write whose own answer was lost still has its failure seen
// in a later one. A read's answer carries its word instead, so a read
// waits for every write before it to be answered, and goes alone.
//
// The window keeps no socket: the caller gives it a function that
// sends a frame and one that waits for an answer, so a test can stand a
// model of the board behind it.
#pragma once

#include <cstdint>
#include <cstdio>
#include <deque>
#include <functional>
#include <string>

#include "eth_frame.h"

namespace eth_frame {

class AskWindow {
public:
  // Sends one frame of MIN_FRAME bytes.
  using Send = std::function<void(const uint8_t *frame)>;
  // Waits up to `timeout_ms` for an answer from this device; false if
  // none came.
  using Receive = std::function<bool(int timeout_ms, Answer *a)>;
  // Stops the run, saying why. It does not return.
  using Fail = std::function<void(const std::string &why)>;

  AskWindow(const uint8_t mac[6], uint8_t device, int window, int timeout_ms,
            int retries, Send send, Receive receive, Fail fail)
      : device_(device), window_(window), timeout_ms_(timeout_ms),
        retries_(retries), send_(std::move(send)),
        receive_(std::move(receive)), fail_(std::move(fail)) {
    for (int i = 0; i < 6; i++) {
      mac_[i] = mac[i];
    }
  }

  // The first ask, a read of `addr` under `tag`, waiting up to
  // `first_timeout_ms` in all. The board answers it, or answers from
  // what it kept under its last tag; either way the count of tags goes
  // on from the tag of the answer. Returns that answer.
  Answer start(uint8_t tag, uint32_t addr, int first_timeout_ms) {
    uint8_t f[MIN_FRAME];
    encode_ask(f, mac_, device_, tag, false, addr, 0);
    const int tries =
        (first_timeout_ms + timeout_ms_ - 1) / timeout_ms_;
    Answer a;
    for (int i = 0; i < tries; i++) {
      if (i) {
        resends++;
      }
      send_(f);
      if (receive_(timeout_ms_, &a)) {
        // Far enough from zero that the count never goes below it.
        done_ = (uint64_t(1) << 32) | a.tag;
        next_ = done_ + 1;
        return a;
      }
    }
    fail_("no answer to the first read at " + hex(addr));
    return a;
  }

  // Sends a write and returns before its answer, once the window has
  // room for it.
  void write(uint32_t addr, uint32_t data) {
    const size_t room = fails_known_ ? size_t(window_) : 1;
    while (out_.size() >= room) {
      pump();
    }
    send_ask(true, addr, data);
    if (!fails_known_) {
      flush();
    }
  }

  // Waits for every write before it, then reads `addr`.
  uint32_t read(uint32_t addr) {
    flush();
    send_ask(false, addr, 0);
    flush();
    return read_data_;
  }

  // Waits until every ask sent has been answered.
  void flush() {
    while (!out_.empty()) {
      pump();
    }
  }

  // How many asks were sent again.
  uint64_t resends = 0;

private:
  struct Out {
    uint64_t seq;
    bool write;
    uint32_t addr;
    uint32_t data;
  };

  static std::string hex(uint32_t v) {
    char b[16];
    snprintf(b, sizeof(b), "0x%x", v);
    return b;
  }

  void send_frame(const Out &o) {
    uint8_t f[MIN_FRAME];
    encode_ask(f, mac_, device_, static_cast<uint8_t>(o.seq), o.write,
               o.addr, o.data);
    send_(f);
  }

  void send_ask(bool write, uint32_t addr, uint32_t data) {
    out_.push_back(Out{next_++, write, addr, data});
    send_frame(out_.back());
  }

  void again() {
    for (const Out &o : out_) {
      resends++;
      send_frame(o);
    }
  }

  // Waits for one answer, or sends the window again when none comes.
  void pump() {
    Answer a;
    if (!receive_(timeout_ms_, &a)) {
      if (++tries_ >= retries_) {
        const Out &o = out_.front();
        fail_(std::string("no answer to a ") + (o.write ? "write" : "read") +
              " at " + hex(o.addr) + " after " + std::to_string(retries_) +
              " tries");
      }
      again();
      sent_again_at_ = done_;
      return;
    }
    // The answer's tag is that of an ask from the last one done to the
    // last one sent; the window is shorter than the 256 tags, so only
    // one of them has it.
    const uint64_t s = done_ + uint8_t(a.tag - uint8_t(done_));
    if (s >= next_) {
      return; // a late answer from before the count started
    }
    if (s == done_) {
      // The board's last is still the last done: an ask after it was
      // lost. Send the window again, once until something is done.
      if (sent_again_at_ != done_) {
        sent_again_at_ = done_;
        again();
      }
      return;
    }
    const Out o = out_[s - done_ - 1];
    while (!out_.empty() && out_.front().seq <= s) {
      out_.pop_front();
    }
    done_ = s;
    tries_ = 0;
    if (o.write) {
      if (!fails_known_) {
        fails_ = a.data - (a.err ? 1 : 0);
        fails_known_ = true;
      }
      if (a.data != fails_) {
        fail_("the bus refused a write at or before " + hex(o.addr));
      }
    } else {
      if (a.err) {
        fail_("the bus refused a read at " + hex(o.addr));
      }
      read_data_ = a.data;
    }
  }

  uint8_t mac_[6];
  uint8_t device_;
  int window_;
  int timeout_ms_;
  int retries_;
  Send send_;
  Receive receive_;
  Fail fail_;

  // Asks sent and not yet answered, oldest first.
  std::deque<Out> out_;
  // The last ask answered, and the next to send, counted from the
  // board's tag; a tag is the low byte.
  uint64_t done_ = 0;
  uint64_t next_ = 1;
  // Where the window was last sent again, so that a run of answers to
  // asks out of turn sends it once.
  uint64_t sent_again_at_ = ~uint64_t(0);
  // Timeouts in a row with nothing done.
  int tries_ = 0;
  // The board's count of failed transactions, once a write's answer
  // has said it; until then each write waits for its answer.
  bool fails_known_ = false;
  uint32_t fails_ = 0;
  uint32_t read_data_ = 0;
};

} // namespace eth_frame
