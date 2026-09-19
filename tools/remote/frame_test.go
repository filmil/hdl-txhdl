// SPDX-License-Identifier: Apache-2.0

package main

import (
	"bytes"
	"io"
	"testing"
)

// The frame the hardware sends for a write, byte for byte. These are
// the same bytes `a_transaction_leaves_as_a_frame` asserts in
// lib/parts/src/remote/eth.rs: the two implementations of one
// protocol are checked against the same constants rather than against
// each other.
func TestTheFrameIsTheOneTheHardwareSends(t *testing.T) {
	want := []byte{
		0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // to every port
		0x02, 0, 0, 0, 0, 3, // from device 3
		0x88, 0xb5, // the type
		KindAsk, 3, 0x5a, 1, // kind, device, tag, a write
		0x12, 0x34, 0x56, 0x78, // the address
		0xc0, 0xff, 0xee, 0x11, // the word
		0x0f, // the strobe
	}
	got := Ask{
		Device: 3,
		Tag:    0x5a,
		Write:  true,
		Addr:   0x12345678,
		Data:   0xc0ffee11,
		Strb:   0x0f,
	}.Frame()
	if !bytes.Equal(got, want) {
		t.Fatalf("frame\n got %02x\nwant %02x", got, want)
	}
	if len(got) != FrameLen {
		t.Fatalf("%d bytes, want %d", len(got), FrameLen)
	}
}

func TestAnAskIsReadBackFromItsFrame(t *testing.T) {
	a := Ask{Device: 3, Tag: 9, Write: true, Addr: 0x1000,
		Data: 0xdeadbeef, Strb: 0xf}
	got, err := ParseAsk(a.Frame())
	if err != nil {
		t.Fatal(err)
	}
	if got != a {
		t.Fatalf("got %+v, want %+v", got, a)
	}
}

// A frame padded to sixty bytes, which is what arrives from a wire,
// parses the same as the twenty-seven the peripheral sent.
func TestPaddingIsIgnored(t *testing.T) {
	a := Ask{Device: 1, Tag: 2, Addr: 0x40}
	padded := append(a.Frame(), make([]byte, 33)...)
	got, err := ParseAsk(padded)
	if err != nil {
		t.Fatal(err)
	}
	if got != a {
		t.Fatalf("got %+v, want %+v", got, a)
	}
}

func TestOtherTrafficIsNotOurs(t *testing.T) {
	a := Ask{Device: 1, Tag: 2}
	other := a.Frame()
	other[13] = 0xb6
	if _, err := ParseAsk(other); err != errNotOurs {
		t.Fatalf("another protocol: %v", err)
	}
	answer := Answer{Device: 1, Tag: 2}.Frame()
	if _, err := ParseAsk(answer); err != errNotOurs {
		t.Fatalf("an answer is not an ask: %v", err)
	}
	if _, err := ParseAsk(a.Frame()[:20]); err == nil {
		t.Fatal("a short frame should not parse")
	}
}

// The memory keeps what is written under the strobe, and refuses a
// read of an address nothing has written.
func TestTheMemoryAnswersLikeADevice(t *testing.T) {
	m := NewMemory()
	if _, err := m.Read(0x40); !err {
		t.Fatal("an address nothing wrote should be refused")
	}
	m.Write(0x40, 0xaabbccdd, 0xf)
	if w, err := m.Read(0x40); err || w != 0xaabbccdd {
		t.Fatalf("got %#x err %v", w, err)
	}
	// Two lanes only: the other two keep what they had.
	m.Write(0x40, 0x11223344, 0x3)
	if w, _ := m.Read(0x40); w != 0xaabb3344 {
		t.Fatalf("got %#x, want 0xaabb3344", w)
	}
}

// One transaction over a connection, as the program serves it: the
// length prefix, the ask, the answer, and the answer's own prefix.
func TestATransactionIsServedOverAConnection(t *testing.T) {
	var in bytes.Buffer
	writeFrame(&in, Ask{Device: 3, Tag: 1, Write: true, Addr: 0x80,
		Data: 0x1234, Strb: 0xf}.Frame())
	writeFrame(&in, Ask{Device: 3, Tag: 2, Addr: 0x80}.Frame())
	// A frame for another device, which this program leaves alone.
	writeFrame(&in, Ask{Device: 4, Tag: 3, Addr: 0x80}.Frame())
	var out bytes.Buffer
	conn := struct {
		io.Reader
		io.Writer
	}{&in, &out}
	if err := run(conn, 3, NewMemory(), false); err != io.EOF {
		t.Fatalf("the connection should end: %v", err)
	}
	answers := out.Bytes()
	if len(answers) != 2*(2+FrameLen) {
		t.Fatalf("%d bytes of answers, want two frames", len(answers))
	}
	second := answers[2+FrameLen+2:]
	if second[16] != 2 {
		t.Fatalf("the tag of the second answer is %d", second[16])
	}
	if got := uint32(second[22])<<24 | uint32(second[23])<<16 |
		uint32(second[24])<<8 | uint32(second[25]); got != 0x1234 {
		t.Fatalf("the read answered %#x", got)
	}
	if second[17] != 0 {
		t.Fatal("the read should not have failed")
	}
}
