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
