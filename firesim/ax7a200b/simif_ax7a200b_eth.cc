// SPDX-License-Identifier: Apache-2.0
//
// FireSim's host link to the AX7A200B over Ethernet (issue 1931).
//
// FireSim's driver reaches the FPGA through its platform file: 32-bit
// register reads and writes on the simulator's AXI4-Lite control port,
// and, for designs that have them, CPU-managed streams. The published
// platforms carry those over PCIe. This one carries the registers over
// raw Ethernet frames to the board's HostLink and LiteMaster
// (lib/parts/src/remote/host.rs): one ask per frame, one answer back.
// Streams are not carried yet, which FireSim's GCD example doesn't
// need; a design that uses them aborts at its first stream access.
//
// The frame is RemoteLink's: type 0x88b5, kind 0x02 asking and 0x82
// answering, the device number, a tag the answer repeats, the write
// flag (on an answer, the error flag), the address and the word, most
// significant byte first, and the strobe.
//
// A lost frame is sent again with the same tag, and the board answers
// a repeat from what it kept rather than performing it twice. So the
// first ask is a harmless read under a random tag: whatever the board
// last kept, every tag after it differs from it.
//
// Plusargs:
//   +iface=<name>        the interface cabled to the board (required)
//   +device=<n>          the design's device number (default 5)
//   +timeout_ms=<n>      how long to wait for an answer (default 20)
//   +retries=<n>         how many times to send an ask (default 50)
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

#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <random>
#include <string>
#include <vector>

#include "core/simif.h"

namespace {

constexpr uint16_t ETHERTYPE = 0x88b5;
constexpr uint8_t KIND_HOST_ASK = 0x02;
constexpr uint8_t KIND_HOST_ANSWER = 0x82;
constexpr size_t FRAME_LEN = 27;
constexpr size_t MIN_FRAME = 60;

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

[[noreturn]] void die(const char *what) {
  fprintf(stderr, "simif_ax7a200b_eth: %s: %s\n", what, strerror(errno));
  exit(1);
}

} // namespace

class simif_ax7a200b_eth_t final : public simif_t {
public:
  simif_ax7a200b_eth_t(const TargetConfig &config,
                       const std::vector<std::string> &args);
  ~simif_ax7a200b_eth_t() override;

  void write(size_t addr, uint32_t data) override;
  uint32_t read(size_t addr) override;

private:
  // One transaction, sent and resent until answered. Returns the word
  // the answer carries.
  uint32_t transact(bool write, uint32_t addr, uint32_t data);

  int fd = -1;
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
  if (iface.empty()) {
    fprintf(stderr, "simif_ax7a200b_eth: give +iface=<interface>\n");
    exit(1);
  }
  device = std::stoi(plusarg(args, "device", "5"));
  timeout_ms = std::stoi(plusarg(args, "timeout_ms", "20"));
  retries = std::stoi(plusarg(args, "retries", "50"));

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
  struct sockaddr_ll sll = {};
  sll.sll_family = AF_PACKET;
  sll.sll_protocol = htons(ETHERTYPE);
  sll.sll_ifindex = ifindex;
  if (bind(fd, reinterpret_cast<struct sockaddr *>(&sll), sizeof(sll)) < 0) {
    die("bind");
  }
  // A random first tag, spent on a read nothing depends on, so that no
  // later tag can repeat the one the board last kept. Address 0 is the
  // simulation master's INIT_DONE, a status register: reading it does
  // nothing (FireSim-generated.const.h, SIMULATIONMASTER_struct).
  tag = static_cast<uint8_t>(std::random_device{}());
  (void)transact(false, 0, 0);
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
}

uint32_t simif_ax7a200b_eth_t::transact(bool write, uint32_t addr,
                                        uint32_t data) {
  tag = static_cast<uint8_t>(tag + 1);
  uint8_t f[MIN_FRAME] = {};
  memset(f, 0xff, 6); // to the board: the cable has no one else on it
  memcpy(f + 6, mac, 6);
  f[12] = ETHERTYPE >> 8;
  f[13] = ETHERTYPE & 0xff;
  f[14] = KIND_HOST_ASK;
  f[15] = device;
  f[16] = tag;
  f[17] = write ? 1 : 0;
  for (int i = 0; i < 4; i++) {
    f[18 + i] = static_cast<uint8_t>(addr >> (24 - 8 * i));
    f[22 + i] = static_cast<uint8_t>(data >> (24 - 8 * i));
  }
  f[26] = write ? 0xf : 0;

  struct sockaddr_ll to = {};
  to.sll_family = AF_PACKET;
  to.sll_ifindex = ifindex;
  to.sll_halen = 6;
  memset(to.sll_addr, 0xff, 6);

  for (int attempt = 0; attempt < retries; attempt++) {
    if (attempt) {
      resends++;
    }
    if (sendto(fd, f, sizeof(f), 0, reinterpret_cast<struct sockaddr *>(&to),
               sizeof(to)) < 0) {
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
      struct pollfd p = {fd, POLLIN, 0};
      if (poll(&p, 1, static_cast<int>(left)) <= 0) {
        continue;
      }
      uint8_t in[1600];
      struct sockaddr_ll from = {};
      socklen_t fl = sizeof(from);
      ssize_t n = recvfrom(fd, in, sizeof(in), 0,
                           reinterpret_cast<struct sockaddr *>(&from), &fl);
      if (n < static_cast<ssize_t>(FRAME_LEN) ||
          from.sll_pkttype == PACKET_OUTGOING) {
        continue;
      }
      if (in[14] != KIND_HOST_ANSWER || in[15] != device || in[16] != tag) {
        continue; // another kind, another device, or a late answer
      }
      if (in[17] & 1) {
        fprintf(stderr,
                "simif_ax7a200b_eth: the bus refused a %s at 0x%x\n",
                write ? "write" : "read",
                addr);
        exit(1);
      }
      return (uint32_t(in[22]) << 24) | (uint32_t(in[23]) << 16) |
             (uint32_t(in[24]) << 8) | uint32_t(in[25]);
    }
  }
  fprintf(stderr,
          "simif_ax7a200b_eth: no answer to a %s at 0x%x after %d tries\n",
          write ? "write" : "read",
          addr,
          retries);
  exit(1);
}

void simif_ax7a200b_eth_t::write(size_t addr, uint32_t data) {
  (void)transact(true, static_cast<uint32_t>(addr), data);
}

uint32_t simif_ax7a200b_eth_t::read(size_t addr) {
  return transact(false, static_cast<uint32_t>(addr), 0);
}

std::unique_ptr<simif_t>
create_simif(const TargetConfig &config, int argc, char **argv) {
  std::vector<std::string> args(argv + 1, argv + argc);
  return std::make_unique<simif_ax7a200b_eth_t>(config, args);
}
