// SPDX-License-Identifier: Apache-2.0
//
// The frames the host and the board speak. The control frames are
// HostBridge's (issue 1931), as lib/parts/src/remote/host.rs lays them
// out: RemoteLink's offsets, type 0x88b5, kind 0x02 asking and 0x82
// answering. The stream frames are StreamBridge's (issue 1984), as
// lib/parts/src/remote/stream.rs lays them out: type 0x88b6, kind 0x03
// asking and 0x83 answering, one 16-byte beat each.
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

constexpr uint16_t ETHERTYPE_STREAM = 0x88b6;
constexpr uint8_t KIND_STREAM_ASK = 0x03;
constexpr uint8_t KIND_STREAM_ANSWER = 0x83;
// The bytes a beat's ask or answer uses, before the padding to sixty.
constexpr size_t STREAM_FRAME_LEN = 44;
constexpr size_t BEAT_BYTES = 16;
// Bits of byte 17 of a beat's ask.
constexpr uint8_t BEAT_WRITE = 1;
constexpr uint8_t BEAT_NOOP = 2;

// A beat's ask, padded to sixty bytes, to every port. `flags` is
// BEAT_WRITE, BEAT_NOOP or neither (a read). The beat is in memory
// order; the address and the strobe go most significant byte first.
inline void encode_beat(uint8_t f[MIN_FRAME], const uint8_t mac[6],
                        uint8_t device, uint8_t tag, uint8_t flags,
                        uint64_t addr, const uint8_t beat[BEAT_BYTES],
                        uint16_t strb) {
  memset(f, 0, MIN_FRAME);
  memset(f, 0xff, 6);
  memcpy(f + 6, mac, 6);
  f[12] = ETHERTYPE_STREAM >> 8;
  f[13] = ETHERTYPE_STREAM & 0xff;
  f[14] = KIND_STREAM_ASK;
  f[15] = device;
  f[16] = tag;
  f[17] = flags;
  for (int i = 0; i < 8; i++) {
    f[18 + i] = static_cast<uint8_t>(addr >> (56 - 8 * i));
  }
  memcpy(f + 26, beat, BEAT_BYTES);
  f[42] = static_cast<uint8_t>(strb >> 8);
  f[43] = static_cast<uint8_t>(strb & 0xff);
}

struct BeatAnswer {
  uint8_t device;
  uint8_t tag;
  bool err;
  uint8_t beat[BEAT_BYTES];
};

// Reads a beat's answer from a frame; false for anything else.
inline bool decode_beat_answer(const uint8_t *in, size_t n, BeatAnswer *a) {
  if (n < STREAM_FRAME_LEN || in[12] != (ETHERTYPE_STREAM >> 8) ||
      in[13] != (ETHERTYPE_STREAM & 0xff) || in[14] != KIND_STREAM_ANSWER) {
    return false;
  }
  a->device = in[15];
  a->tag = in[16];
  a->err = (in[17] & 1) != 0;
  memcpy(a->beat, in + 26, BEAT_BYTES);
  return true;
}

} // namespace eth_frame
