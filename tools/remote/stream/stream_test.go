// SPDX-License-Identifier: Apache-2.0

package stream

import (
	"bytes"
	"io"
	"testing"
)

// A frame written is the frame read, and two in a row stay two.
func TestAFrameComesBackWhole(t *testing.T) {
	var b bytes.Buffer
	one := bytes.Repeat([]byte{0xa5}, MinLen)
	two := bytes.Repeat([]byte{0x5a}, 60)
	if err := Write(&b, one); err != nil {
		t.Fatal(err)
	}
	if err := Write(&b, two); err != nil {
		t.Fatal(err)
	}
	got, err := Read(&b)
	if err != nil || !bytes.Equal(got, one) {
		t.Fatalf("first: %v %x", err, got)
	}
	got, err = Read(&b)
	if err != nil || !bytes.Equal(got, two) {
		t.Fatalf("second: %v %x", err, got)
	}
	if _, err := Read(&b); err != io.EOF {
		t.Fatalf("after the last: %v, want EOF", err)
	}
}

// A length no frame has is refused before any of it is read, so a
// stream that has lost its place is found out at once.
func TestALengthNoFrameHasIsRefused(t *testing.T) {
	for _, head := range [][]byte{{0, 3}, {0xff, 0xff}} {
		if _, err := Read(bytes.NewReader(head)); err == nil {
			t.Fatalf("%x accepted", head)
		}
	}
	if err := Write(io.Discard, make([]byte, MaxLen+1)); err == nil {
		t.Fatal("a frame longer than any wire carries was written")
	}
}
