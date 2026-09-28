// SPDX-License-Identifier: Apache-2.0

package main

import (
	"encoding/binary"
	"errors"
	"fmt"
)

// Bit is what the header of a Vivado `.bit` file says: the fields it
// names, and the length of the configuration data after it, which is
// what goes into the flash from offset zero.
type Bit struct {
	Design, Part, Date, Time string
	DataLen                  int
	// HeaderLen is the bytes before the data.
	HeaderLen int
}

// The file starts with a length of nine, nine fixed bytes, and a
// length of one.
var magic = []byte{0x00, 0x09, 0x0f, 0xf0, 0x0f, 0xf0, 0x0f, 0xf0, 0x0f, 0xf0, 0x00, 0x00, 0x01}

// Parse reads the header: after the fixed start, the fields `a` to
// `d`, each a key byte, a two-byte big-endian length and a string ended
// by a zero, then `e`, a four-byte big-endian length, and the data. The
// data must be all there, since a short file is a truncated bitstream.
func Parse(b []byte) (Bit, error) {
	var out Bit
	if len(b) < len(magic) || string(b[:len(magic)]) != string(magic) {
		return out, errors.New("not a .bit file: the fixed start is missing")
	}
	at := len(magic)
	fields := []*string{&out.Design, &out.Part, &out.Date, &out.Time}
	for i, f := range fields {
		key := byte('a' + i)
		if at+3 > len(b) || b[at] != key {
			return out, fmt.Errorf("field %q missing at byte %d", key, at)
		}
		n := int(binary.BigEndian.Uint16(b[at+1:]))
		at += 3
		if at+n > len(b) || n == 0 {
			return out, fmt.Errorf("field %q runs past the end", key)
		}
		*f = string(b[at : at+n-1])
		at += n
	}
	if at+5 > len(b) || b[at] != 'e' {
		return out, fmt.Errorf("field 'e' missing at byte %d", at)
	}
	out.DataLen = int(binary.BigEndian.Uint32(b[at+1:]))
	out.HeaderLen = at + 5
	if out.HeaderLen+out.DataLen != len(b) {
		return out, fmt.Errorf("the header says %d bytes of data and the file holds %d",
			out.DataLen, len(b)-out.HeaderLen)
	}
	return out, nil
}
