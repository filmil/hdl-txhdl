// SPDX-License-Identifier: Apache-2.0
//
// The host's frames against the layout HostBridge reads and writes,
// byte for byte, at the offsets lib/parts/src/remote/host.rs's tests use.
#include "eth_frame.h"

#include <cstdio>
#include <cstdlib>

using namespace eth_frame;

static int failures = 0;

static void expect(bool ok, const char *what) {
  if (!ok) {
    fprintf(stderr, "FAILED: %s\n", what);
    failures++;
  }
}

int main() {
  const uint8_t mac[6] = {0x02, 0, 0, 0, 0, 0x10};
  uint8_t f[MIN_FRAME];
  encode_ask(f, mac, 5, 7, true, 0x12345678, 0xc0ffee);
  expect(f[0] == 0xff && f[5] == 0xff, "to every port");
  expect(f[6] == 0x02 && f[11] == 0x10, "from the host's address");
  expect(f[12] == 0x88 && f[13] == 0xb5, "type 0x88b5");
  expect(f[14] == 0x02, "kind: an ask");
  expect(f[15] == 5 && f[16] == 7, "device and tag");
  expect(f[17] == 1, "a write");
  expect(f[18] == 0x12 && f[19] == 0x34 && f[20] == 0x56 && f[21] == 0x78,
         "address, most significant byte first");
  expect(f[22] == 0x00 && f[23] == 0xc0 && f[24] == 0xff && f[25] == 0xee,
         "word, most significant byte first");
  expect(f[26] == 0xf, "a write's strobe");
  expect(f[59] == 0, "padded with zeros to sixty");

  encode_ask(f, mac, 5, 8, false, 0x40, 0);
  expect(f[17] == 0 && f[26] == 0, "a read: no write flag, no strobe");

  // An answer, as HostBridge's answer_byte writes one.
  uint8_t in[MIN_FRAME] = {};
  in[12] = 0x88;
  in[13] = 0xb5;
  in[14] = 0x82;
  in[15] = 5;
  in[16] = 8;
  in[17] = 0;
  in[22] = 0xde;
  in[23] = 0xad;
  in[24] = 0xbe;
  in[25] = 0xef;
  Answer a;
  expect(decode_answer(in, sizeof(in), &a), "an answer is read");
  expect(a.device == 5 && a.tag == 8 && !a.err && a.data == 0xdeadbeef,
         "its device, tag, flag and word");
  in[17] = 1;
  expect(decode_answer(in, sizeof(in), &a) && a.err, "the error flag");
  in[14] = 0x02;
  expect(!decode_answer(in, sizeof(in), &a), "an ask is not an answer");

  // A beat's ask, as StreamLink reads one.
  uint8_t beat[BEAT_BYTES];
  for (int i = 0; i < 16; i++) {
    beat[i] = static_cast<uint8_t>(i);
  }
  encode_beat(f, mac, 5, 9, BEAT_WRITE, 0x0102030405060708ULL, beat, 0x8001);
  expect(f[12] == 0x88 && f[13] == 0xb6, "type 0x88b6");
  expect(f[14] == 0x03 && f[15] == 5 && f[16] == 9, "kind, device, tag");
  expect(f[17] == 1, "a write");
  expect(f[18] == 0x01 && f[25] == 0x08,
         "address, most significant byte first");
  expect(f[26] == 0 && f[41] == 15, "the beat in memory order");
  expect(f[42] == 0x80 && f[43] == 0x01,
         "strobe, most significant byte first");
  expect(f[44] == 0 && f[59] == 0, "padded with zeros to sixty");
  encode_beat(f, mac, 5, 10, BEAT_NOOP, 0, beat, 0);
  expect(f[17] == 2, "a no-op");

  // A beat's answer, as StreamLink's stream_answer_byte writes one.
  uint8_t bin[MIN_FRAME] = {};
  bin[12] = 0x88;
  bin[13] = 0xb6;
  bin[14] = 0x83;
  bin[15] = 5;
  bin[16] = 10;
  for (int i = 0; i < 16; i++) {
    bin[26 + i] = static_cast<uint8_t>(0xa0 + i);
  }
  BeatAnswer ba;
  expect(decode_beat_answer(bin, sizeof(bin), &ba), "a beat's answer");
  expect(ba.device == 5 && ba.tag == 10 && !ba.err && ba.beat[0] == 0xa0 &&
             ba.beat[15] == 0xaf,
         "its device, tag, flag and beat");
  expect(!decode_answer(bin, sizeof(bin), &a), "not a control answer");
  bin[17] = 1;
  expect(decode_beat_answer(bin, sizeof(bin), &ba) && ba.err,
         "a beat's error flag");
  expect(!decode_beat_answer(bin, STREAM_FRAME_LEN - 1, &ba),
         "too short is not a beat's answer");
  bin[14] = KIND_STREAM_ASK;
  expect(!decode_beat_answer(bin, sizeof(bin), &ba),
         "a beat's ask is not its answer");
  bin[14] = KIND_STREAM_ANSWER;
  bin[13] = 0xb5;
  expect(!decode_beat_answer(bin, sizeof(bin), &ba), "the type is checked");
  in[14] = 0x82;
  in[13] = 0xb6;
  expect(!decode_answer(in, sizeof(in), &a), "another type is not one");
  in[13] = 0xb5;
  expect(!decode_answer(in, FRAME_LEN - 1, &a), "too short is not one");

  if (failures) {
    return 1;
  }
  printf("eth_frame: every check passed\n");
  return 0;
}
