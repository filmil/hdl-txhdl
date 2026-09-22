// SPDX-License-Identifier: Apache-2.0

// Package stream carries Ethernet frames over a byte stream: each
// frame is its length in two bytes, most significant first, and then
// the frame. That is all the framing a stream needs, and it is what
// survives an ssh tunnel, which is why the program that serves a
// Remote peripheral and the shim beside the board both speak it
// rather than raw frames.
package stream

import (
	"encoding/binary"
	"fmt"
	"io"
)

const (
	// MinLen is the shortest frame that is worth carrying: the
	// Remote peripheral's 27 bytes before the MAC's padding.
	MinLen = 27
	// MaxLen is the longest Ethernet frame with a VLAN tag.
	MaxLen = 1522
)

// Read takes one frame off the stream.
func Read(r io.Reader) ([]byte, error) {
	var head [2]byte
	if _, err := io.ReadFull(r, head[:]); err != nil {
		return nil, err
	}
	n := binary.BigEndian.Uint16(head[:])
	if n < MinLen || n > MaxLen {
		return nil, fmt.Errorf("a frame of %d bytes", n)
	}
	f := make([]byte, n)
	_, err := io.ReadFull(r, f)
	return f, err
}

// Write puts one frame on the stream.
func Write(w io.Writer, f []byte) error {
	if len(f) > MaxLen {
		return fmt.Errorf("a frame of %d bytes", len(f))
	}
	var head [2]byte
	binary.BigEndian.PutUint16(head[:], uint16(len(f)))
	if _, err := w.Write(head[:]); err != nil {
		return err
	}
	_, err := w.Write(f)
	return err
}
