// SPDX-License-Identifier: Apache-2.0
//
// FireSim's host link to the AX7A200B over Ethernet (issue 1931).
//
// FireSim's driver reaches the FPGA through its platform file: 32-bit
// register reads and writes on the simulator's AXI4-Lite control port,
// and, for designs that have them, CPU-managed streams on a 128-bit
// AXI4 port. The published platforms carry those over PCIe. This one
// carries them over raw Ethernet frames, one ask per frame and one
// answer back:
//
// - the registers to the board's HostLink and LiteMaster
//   (lib/parts/src/remote/host.rs, issue 1931), in RemoteLink's frame:
//   type 0x88b5, kind 0x02 asking and 0x82 answering;
// - the streams to its StreamLink and BeatMaster
//   (lib/parts/src/remote/stream.rs, issue 1984), one 16-byte beat per
//   frame: type 0x88b6, kind 0x03 asking and 0x83 answering.
//
// eth_frame.h lays out both.
//
// A lost frame is sent again with the same tag, and the board answers
// a repeat from what it kept rather than performing it twice. Each path
// keeps its own tag. So the first ask on each is harmless, under a
// random tag: a read of a plain register for the control path, a
// no-op for the streams, sent at the first stream access so a design
// without streams never asks for one. Whatever the board last kept,
// every tag after it differs from it.
//
// Before all of that, the driver sends a reset frame (issue 2007).
// FireSim expects a fresh simulator for each run: a run that ends leaves
// its step unfinished, and the fourth run on one simulator would wait
// for ever on its first STEP write. The Rocket design's board resets
// the simulator and the link on the frame, and its memory calibrates
// again, so the first ask waits up to five seconds for an answer. A
// design without the reset unit ignores the frame.
//
// Plusargs:
//   +iface=<name>        the interface cabled to the board (required)
//   +device=<n>          the design's device number (default 5)
//   +timeout_ms=<n>      how long to wait for an answer (default 20)
//   +retries=<n>         how many times to send an ask (default 50)
//   +eth_log=<file>      log every register access and stream beat
//                        to <file>, one line each (issue 2007)
//
// The raw socket needs CAP_NET_RAW: root, or this binary given it with
// `setcap cap_net_raw+ep`.

#include <arpa/inet.h>
#include <net/if.h>
#include <netpacket/packet.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <random>
#include <string>
#include <thread>
#include <vector>

#include "bridges/cpu_managed_stream.h"
#include "core/simif.h"
#include "eth_frame.h"

namespace {

using namespace eth_frame;

std::string plusarg(const std::vector<std::string> &args,
                    const std::string &name,
                    const std::string &dflt) {
  const std::string key = "+" + name + "=";
  for (const auto &a : args) {
    if (a.rfind(key, 0) == 0) {
      return a.substr(key.size());
    }
  }
  return dflt;
}

// A whole-number plusarg in [lo, hi]; anything else stops the driver.
int plusint(const std::vector<std::string> &args, const std::string &name,
            int dflt, int lo, int hi) {
  const std::string v = plusarg(args, name, std::to_string(dflt));
  char *end = nullptr;
  const long n = strtol(v.c_str(), &end, 10);
  if (v.empty() || *end != '\0' || n < lo || n > hi) {
    fprintf(stderr, "simif_ax7a200b_eth: +%s=%s is not a number from %d to %d\n",
            name.c_str(), v.c_str(), lo, hi);
    exit(1);
  }
  return static_cast<int>(n);
}

[[noreturn]] void die(const char *what) {
  fprintf(stderr, "simif_ax7a200b_eth: %s: %s\n", what, strerror(errno));
  exit(1);
}

} // namespace

class simif_ax7a200b_eth_t final : public simif_t,
                                   public CPUManagedStreamIO {
public:
  simif_ax7a200b_eth_t(const TargetConfig &config,
                       const std::vector<std::string> &args);
  ~simif_ax7a200b_eth_t() override;

  void write(size_t addr, uint32_t data) override;
  uint32_t read(size_t addr) override;

  CPUManagedStreamIO &get_cpu_managed_stream_io() override { return *this; }

private:
  uint32_t mmio_read(size_t addr) override { return read(addr); }
  size_t
  cpu_managed_axi4_write(size_t addr, const char *data, size_t size) override;
  size_t cpu_managed_axi4_read(size_t addr, char *data, size_t size) override;
  uint64_t get_beat_bytes() const override { return BEAT_BYTES; }

  // Sends the reset frame, which gets no answer.
  void send_reset();
  // Opens a raw socket for one EtherType, bound to the interface.
  int open_socket(uint16_t type);
  // Sends the frame `f` on `sock` and resends it until `answered` takes
  // an answer from a frame that came in, or the tries run out.
  template <typename Answered>
  void exchange(int sock, const uint8_t *f, Answered answered,
                const char *what, uint64_t addr);
  // One control transaction. Returns the word the answer carries.
  uint32_t transact(bool write, uint32_t addr, uint32_t data);
  // One beat on the stream port. Returns the beat a read read in `out`.
  void beat(uint8_t flags, uint64_t addr, const uint8_t in[BEAT_BYTES],
            uint16_t strb, uint8_t out[BEAT_BYTES]);
  // Sends the stream path's first ask, once.
  void start_streams();
  // Seconds since the driver started, for the log.
  double since_start() const;

  FILE *log = nullptr;
  std::chrono::steady_clock::time_point start =
      std::chrono::steady_clock::now();

  int fd = -1;
  int sfd = -1;
  bool streams_ready = false;
  uint8_t stag = 0;
  int ifindex = 0;
  uint8_t mac[6] = {0x02, 0, 0, 0, 0, 0x10};
  uint8_t device = 5;
  int timeout_ms = 20;
  int retries = 50;
  uint8_t tag = 0;
  uint64_t resends = 0;
};

simif_ax7a200b_eth_t::simif_ax7a200b_eth_t(
    const TargetConfig &config, const std::vector<std::string> &args)
    : simif_t(config) {
  const std::string iface = plusarg(args, "iface", "");
  if (iface.empty() || iface.size() >= IFNAMSIZ) {
    fprintf(stderr,
            "simif_ax7a200b_eth: give +iface=<interface>, at most %d "
            "characters\n",
            IFNAMSIZ - 1);
    exit(1);
  }
  device = static_cast<uint8_t>(plusint(args, "device", 5, 0, 255));
  timeout_ms = plusint(args, "timeout_ms", 20, 1, 60000);
  retries = plusint(args, "retries", 50, 1, 1000000);
  const std::string log_name = plusarg(args, "eth_log", "");
  if (!log_name.empty()) {
    log = fopen(log_name.c_str(), "w");
    if (!log) {
      die("+eth_log");
    }
    // Line by line, so a run that hangs keeps its log up to the hang.
    setvbuf(log, nullptr, _IOLBF, 0);
  }

  fd = socket(AF_PACKET, SOCK_RAW, htons(ETHERTYPE));
  if (fd < 0) {
    die("raw socket (needs cap_net_raw)");
  }
  struct ifreq ifr = {};
  strncpy(ifr.ifr_name, iface.c_str(), IFNAMSIZ - 1);
  if (ioctl(fd, SIOCGIFINDEX, &ifr) < 0) {
    die("interface index");
  }
  ifindex = ifr.ifr_ifindex;
  if (ioctl(fd, SIOCGIFHWADDR, &ifr) == 0) {
    memcpy(mac, ifr.ifr_hwaddr.sa_data, 6);
  }
  close(fd);
  fd = open_socket(ETHERTYPE);
  sfd = open_socket(ETHERTYPE_STREAM);
  // The board answers an ask that repeats the tag of the last one it
  // performed from what it kept, without performing it. Each ask here
  // takes the tag after the one before, so only the first could match
  // a tag the board kept from an earlier run. So the first ask is a
  // read nothing depends on, under a random tag: if it matches, the
  // board answers it from what it kept, and every ask after it differs
  // from the board's last, however the count wraps. Address 0 is a
  // plain register that reading does nothing to: GCD's simulation
  // master's INIT_DONE, and the Rocket design's block device's
  // read_latency (FireSim-generated.const.h).
  //
  // After a reset the link answers once the memory has calibrated
  // again: hence the longer wait for the first answer.
  send_reset();
  tag = static_cast<uint8_t>(std::random_device{}());
  const int asks = retries;
  retries = std::max(retries, (5000 + timeout_ms - 1) / timeout_ms);
  (void)transact(false, 0, 0);
  retries = asks;
  fprintf(stdout,
          "simif_ax7a200b_eth: device %u on %s, answering\n",
          device,
          iface.c_str());
}

simif_ax7a200b_eth_t::~simif_ax7a200b_eth_t() {
  if (resends) {
    fprintf(stdout, "simif_ax7a200b_eth: %lu asks sent again\n",
            static_cast<unsigned long>(resends));
  }
  if (fd >= 0) {
    close(fd);
  }
  if (sfd >= 0) {
    close(sfd);
  }
  if (log) {
    fclose(log);
  }
}

double simif_ax7a200b_eth_t::since_start() const {
  return std::chrono::duration<double>(std::chrono::steady_clock::now() -
                                       start)
      .count();
}

void simif_ax7a200b_eth_t::send_reset() {
  uint8_t f[MIN_FRAME];
  encode_reset(f, mac, device);
  // Three times, since nothing answers it: one lost frame would leave
  // the run on the simulator the last run left. The three arrive within
  // microseconds, and a second reset while the first holds is harmless.
  struct sockaddr_ll to = {};
  to.sll_family = AF_PACKET;
  to.sll_ifindex = ifindex;
  to.sll_halen = 6;
  memset(to.sll_addr, 0xff, 6);
  for (int i = 0; i < 3; i++) {
    if (sendto(fd, f, MIN_FRAME, 0,
               reinterpret_cast<struct sockaddr *>(&to), sizeof(to)) < 0) {
      die("send");
    }
  }
  if (log) {
    fprintf(log, "%.6f reset\n", since_start());
  }
  // The reset takes the link within microseconds; an ask sent before
  // it could be answered, and the asks after it lost to the reset.
  std::this_thread::sleep_for(std::chrono::milliseconds(10));
}

int simif_ax7a200b_eth_t::open_socket(uint16_t type) {
  const int s = socket(AF_PACKET, SOCK_RAW, htons(type));
  if (s < 0) {
    die("raw socket (needs cap_net_raw)");
  }
  struct sockaddr_ll sll = {};
  sll.sll_family = AF_PACKET;
  sll.sll_protocol = htons(type);
  sll.sll_ifindex = ifindex;
  if (bind(s, reinterpret_cast<struct sockaddr *>(&sll), sizeof(sll)) < 0) {
    die("bind");
  }
  return s;
}

template <typename Answered>
void simif_ax7a200b_eth_t::exchange(int sock, const uint8_t *f,
                                    Answered answered, const char *what,
                                    uint64_t addr) {
  struct sockaddr_ll to = {};
  to.sll_family = AF_PACKET;
  to.sll_ifindex = ifindex;
  to.sll_halen = 6;
  memset(to.sll_addr, 0xff, 6);

  for (int attempt = 0; attempt < retries; attempt++) {
    if (attempt) {
      resends++;
      if (log) {
        fprintf(log, "%.6f again %s %llx\n", since_start(), what,
                static_cast<unsigned long long>(addr));
      }
    }
    if (sendto(sock, f, MIN_FRAME, 0,
               reinterpret_cast<struct sockaddr *>(&to), sizeof(to)) < 0) {
      die("send");
    }
    const auto until = std::chrono::steady_clock::now() +
                       std::chrono::milliseconds(timeout_ms);
    for (;;) {
      const auto left = std::chrono::duration_cast<std::chrono::milliseconds>(
                            until - std::chrono::steady_clock::now())
                            .count();
      if (left <= 0) {
        break;
      }
      struct pollfd p = {sock, POLLIN, 0};
      const int r = poll(&p, 1, static_cast<int>(left));
      if (r < 0 && errno != EINTR) {
        die("poll");
      }
      if (r <= 0) {
        continue; // interrupted, or the wait ran out: checked above
      }
      uint8_t in[1600];
      struct sockaddr_ll from = {};
      socklen_t fl = sizeof(from);
      ssize_t n = recvfrom(sock, in, sizeof(in), 0,
                           reinterpret_cast<struct sockaddr *>(&from), &fl);
      if (n < 0 || from.sll_pkttype == PACKET_OUTGOING) {
        continue;
      }
      if (answered(in, static_cast<size_t>(n))) {
        return;
      }
    }
  }
  if (log) {
    fprintf(log, "%.6f none %s %llx\n", since_start(), what,
            static_cast<unsigned long long>(addr));
  }
  fprintf(stderr,
          "simif_ax7a200b_eth: no answer to a %s at 0x%llx after %d tries\n",
          what,
          static_cast<unsigned long long>(addr),
          retries);
  exit(1);
}

uint32_t simif_ax7a200b_eth_t::transact(bool write, uint32_t addr,
                                        uint32_t data) {
  tag = static_cast<uint8_t>(tag + 1);
  uint8_t f[MIN_FRAME];
  encode_ask(f, mac, device, tag, write, addr, data);
  Answer a;
  exchange(
      fd,
      f,
      [&](const uint8_t *in, size_t n) {
        // Not an answer, another device's, or a late one: wait on.
        return decode_answer(in, n, &a) && a.device == device && a.tag == tag;
      },
      write ? "write" : "read",
      addr);
  if (log) {
    // Time, C for control, W or R, address, the word written or read,
    // and the answer's error flag.
    fprintf(log, "%.6f C %c %08x %08x %d\n", since_start(),
            write ? 'W' : 'R', addr, write ? data : a.data, a.err ? 1 : 0);
  }
  if (a.err) {
    fprintf(stderr,
            "simif_ax7a200b_eth: the bus refused a %s at 0x%x\n",
            write ? "write" : "read",
            addr);
    exit(1);
  }
  return a.data;
}

void simif_ax7a200b_eth_t::beat(uint8_t flags, uint64_t addr,
                                const uint8_t in[BEAT_BYTES], uint16_t strb,
                                uint8_t out[BEAT_BYTES]) {
  stag = static_cast<uint8_t>(stag + 1);
  uint8_t f[MIN_FRAME];
  encode_beat(f, mac, device, stag, flags, addr, in, strb);
  BeatAnswer a;
  const char *what = (flags & BEAT_NOOP)    ? "stream no-op"
                     : (flags & BEAT_WRITE) ? "stream write"
                                            : "stream read";
  exchange(
      sfd,
      f,
      [&](const uint8_t *frame, size_t n) {
        return decode_beat_answer(frame, n, &a) && a.device == device &&
               a.tag == stag;
      },
      what,
      addr);
  if (log) {
    // Time, S for stream, N(o-op), W or R, address, the beat written or
    // read as bytes 15 down to 0, and the answer's error flag.
    const uint8_t *b = (flags & BEAT_WRITE) ? in : a.beat;
    fprintf(log, "%.6f S %c %08llx ", since_start(),
            (flags & BEAT_NOOP) ? 'N' : (flags & BEAT_WRITE) ? 'W' : 'R',
            static_cast<unsigned long long>(addr));
    for (int i = static_cast<int>(BEAT_BYTES) - 1; i >= 0; i--) {
      fprintf(log, "%02x", b[i]);
    }
    fprintf(log, " %d\n", a.err ? 1 : 0);
  }
  if (a.err) {
    fprintf(stderr,
            "simif_ax7a200b_eth: the stream port refused a %s at 0x%llx\n",
            what,
            static_cast<unsigned long long>(addr));
    exit(1);
  }
  if (out) {
    memcpy(out, a.beat, BEAT_BYTES);
  }
}

void simif_ax7a200b_eth_t::write(size_t addr, uint32_t data) {
  (void)transact(true, static_cast<uint32_t>(addr), data);
}

uint32_t simif_ax7a200b_eth_t::read(size_t addr) {
  return transact(false, static_cast<uint32_t>(addr), 0);
}

// A beat goes to lane 0 of the port, so a transfer must start on a
// beat. FireSim's stream addresses do.
static void check_aligned(size_t addr) {
  if (addr % BEAT_BYTES != 0) {
    fprintf(stderr,
            "simif_ax7a200b_eth: a stream transfer at 0x%zx is not on a "
            "%zu-byte beat\n",
            addr,
            BEAT_BYTES);
    exit(1);
  }
}

// The first stream access starts the stream path's tags with a no-op,
// as the constructor starts the control path's with a read.
void simif_ax7a200b_eth_t::start_streams() {
  if (streams_ready) {
    return;
  }
  stag = static_cast<uint8_t>(std::random_device{}());
  const uint8_t zero[BEAT_BYTES] = {};
  beat(BEAT_NOOP, 0, zero, 0, nullptr);
  streams_ready = true;
}

size_t simif_ax7a200b_eth_t::cpu_managed_axi4_write(size_t addr,
                                                    const char *data,
                                                    size_t size) {
  start_streams();
  check_aligned(addr);
  // One beat per frame; a last beat shorter than sixteen bytes is sent
  // with only its own lanes in the strobe.
  for (size_t at = 0; at < size; at += BEAT_BYTES) {
    const size_t n = std::min(BEAT_BYTES, size - at);
    uint8_t b[BEAT_BYTES] = {};
    memcpy(b, data + at, n);
    const uint16_t strb =
        n == BEAT_BYTES ? 0xffff : static_cast<uint16_t>((1u << n) - 1);
    beat(BEAT_WRITE, addr + at, b, strb, nullptr);
  }
  return size;
}

size_t simif_ax7a200b_eth_t::cpu_managed_axi4_read(size_t addr,
                                                   char *data,
                                                   size_t size) {
  start_streams();
  check_aligned(addr);
  const uint8_t zero[BEAT_BYTES] = {};
  for (size_t at = 0; at < size; at += BEAT_BYTES) {
    uint8_t b[BEAT_BYTES];
    beat(0, addr + at, zero, 0, b);
    memcpy(data + at, b, std::min(BEAT_BYTES, size - at));
  }
  return size;
}

std::unique_ptr<simif_t>
create_simif(const TargetConfig &config, int argc, char **argv) {
  std::vector<std::string> args(argv + 1, argv + argc);
  return std::make_unique<simif_ax7a200b_eth_t>(config, args);
}
