// SPDX-License-Identifier: Apache-2.0
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"fmt"
	"strings"
)

// Image is a flash image read from Intel HEX, the `.mcs` that
// Vivado's write_cfgmem writes: every byte it gives an address.
type Image map[uint32]byte

// ParseMCS reads an Intel HEX file: data records (type 00), the
// extended linear address (type 04) that sets the upper half of the
// addresses after it, and the end (type 01). A record whose checksum
// is wrong, or of another type, is an error.
func ParseMCS(text []byte) (Image, error) {
	img := Image{}
	var upper uint32
	sc := bufio.NewScanner(bytes.NewReader(text))
	n := 0
	for sc.Scan() {
		n++
		line := strings.TrimSpace(sc.Text())
		if line == "" {
			continue
		}
		if !strings.HasPrefix(line, ":") {
			return nil, fmt.Errorf("line %d: no ':'", n)
		}
		rec, err := hex.DecodeString(line[1:])
		if err != nil || len(rec) < 5 {
			return nil, fmt.Errorf("line %d: not a record", n)
		}
		count := int(rec[0])
		if len(rec) != count+5 {
			return nil, fmt.Errorf("line %d: %d bytes for a count of %d", n, len(rec)-5, count)
		}
		var sum byte
		for _, b := range rec {
			sum += b
		}
		if sum != 0 {
			return nil, fmt.Errorf("line %d: checksum", n)
		}
		addr := uint32(rec[1])<<8 | uint32(rec[2])
		data := rec[4 : 4+count]
		switch rec[3] {
		case 0x00:
			for i, b := range data {
				img[upper|addr+uint32(i)] = b
			}
		case 0x01:
			return img, nil
		case 0x04:
			if count != 2 {
				return nil, fmt.Errorf("line %d: an address of %d bytes", n, count)
			}
			upper = (uint32(data[0])<<8 | uint32(data[1])) << 16
		default:
			return nil, fmt.Errorf("line %d: record type %02x", n, rec[3])
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	return nil, fmt.Errorf("no end record")
}

// Holds says where, if anywhere, the image differs from `want` placed
// at `at`: the first address whose byte is missing or another.
func (img Image) Holds(want []byte, at uint32) (uint32, bool) {
	for i, b := range want {
		a := at + uint32(i)
		got, ok := img[a]
		if !ok || got != b {
			return a, false
		}
	}
	return 0, true
}

// Below is the highest address the image gives a byte under `limit`,
// and whether it gives any: the end of what sits before the programs.
func (img Image) Below(limit uint32) (uint32, bool) {
	var hi uint32
	found := false
	for a := range img {
		if a < limit && (!found || a > hi) {
			hi, found = a, true
		}
	}
	return hi, found
}
