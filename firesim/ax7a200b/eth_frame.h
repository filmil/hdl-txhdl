// SPDX-License-Identifier: Apache-2.0
//
// The frame the host and the board's HostBridge speak (issue 1931), as
// lib/parts/src/remote/host.rs lays it out: RemoteLink's offsets, type
// 0x88b5, kind 0x02 asking and 0x82 answering.
#pragma once

#include <cstddef>
#include <cstdint>
#include <cstring>

namespace eth_frame {

constexpr uint16_t ETHERTYPE = 0x88b5;
constexpr uint8_t KIND_HOST_ASK = 0x02;
constexpr uint8_t KIND_HOST_ANSWER = 0x82;
// The bytes an ask or an answer uses, before the padding to sixty.
constexpr size_t FRAME_LEN = 27;
constexpr size_t MIN_FRAME = 60;

// An ask, padded to sixty bytes, to every port: the cable to the board
// has nothing else on it.
inline void encode_ask(uint8_t f[MIN_FRAME], const uint8_t mac[6],
                       uint8_t device, uint8_t tag, bool write,
                       uint32_t addr, uint32_t data) {
  memset(f, 0, MIN_FRAME);
  memset(f, 0xff, 6);
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
}

struct Answer {
  uint8_t device;
  uint8_t tag;
  bool err;
  uint32_t data;
};

// Reads an answer to a program from a frame; false for anything else:
// too short, another type, or another kind.
inline bool decode_answer(const uint8_t *in, size_t n, Answer *a) {
  if (n < FRAME_LEN || in[12] != (ETHERTYPE >> 8) ||
      in[13] != (ETHERTYPE & 0xff) || in[14] != KIND_HOST_ANSWER) {
    return false;
  }
  a->device = in[15];
  a->tag = in[16];
  a->err = (in[17] & 1) != 0;
  a->data = (uint32_t(in[22]) << 24) | (uint32_t(in[23]) << 16) |
            (uint32_t(in[24]) << 8) | uint32_t(in[25]);
  return true;
}

} // namespace eth_frame
