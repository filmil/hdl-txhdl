// SPDX-License-Identifier: Apache-2.0

// Package main speaks the protocol of the Remote peripheral: one
// Ethernet frame carries one AXI-Lite transaction out of a design,
// and one carries the answer back.
//
// The frame is the one lib/parts/src/remote/eth.rs builds, and the
// constants below are its constants. A frame is 27 bytes before the
// padding the MAC adds, and a receiver must accept the padded 60 as
// well, since that is what arrives on a wire.
package main

import (
	"encoding/binary"
	"errors"
	"fmt"
)

const (
	// EtherType is what IEEE leaves for local experimental use, so no
	// registered protocol can be mistaken for this one.
	EtherType = 0x88b5
	// KindAsk marks a frame carrying a transaction out of a design.
	KindAsk = 0x01
	// KindAnswer marks a frame carrying the answer to one.
	KindAnswer = 0x81
	// FrameLen is what the peripheral sends, before the MAC's padding.
	FrameLen = 27
)

// An Ask is one AXI-Lite transaction, as it left the design.
type Ask struct {
	Device uint8
	Tag    uint8
	Write  bool
	Addr   uint32
	Data   uint32
	Strb   uint8
}

// An Answer is what a program says about one Ask.
type Answer struct {
	Device uint8
	Tag    uint8
	Err    bool
	Data   uint32
}

var errNotOurs = errors.New("not this protocol")

// ParseAsk reads a frame that carries a transaction. It returns
// errNotOurs for a frame of another protocol or another kind, which a
// program on a shared wire will see plenty of.
func ParseAsk(f []byte) (Ask, error) {
	if len(f) < FrameLen {
		return Ask{}, fmt.Errorf("frame of %d bytes, want %d or more",
			len(f), FrameLen)
	}
	if binary.BigEndian.Uint16(f[12:14]) != EtherType {
		return Ask{}, errNotOurs
	}
	if f[14] != KindAsk {
		return Ask{}, errNotOurs
	}
	return Ask{
		Device: f[15],
		Tag:    f[16],
		Write:  f[17]&1 == 1,
		Addr:   binary.BigEndian.Uint32(f[18:22]),
		Data:   binary.BigEndian.Uint32(f[22:26]),
		Strb:   f[26],
	}, nil
}

// Frame builds the frame that carries an answer. It is addressed to
// every port, as the ask is: the peripheral has no address of its own
// and the program need not learn one.
func (a Answer) Frame() []byte {
	f := make([]byte, FrameLen)
	for i := 0; i < 6; i++ {
		f[i] = 0xff
	}
	// The source: locally administered, the device in the last byte,
	// so two programs answering two devices differ on the wire.
	f[6] = 0x02
	f[11] = a.Device
	binary.BigEndian.PutUint16(f[12:14], EtherType)
	f[14] = KindAnswer
	f[15] = a.Device
	f[16] = a.Tag
	if a.Err {
		f[17] = 1
	}
	binary.BigEndian.PutUint32(f[22:26], a.Data)
	return f
}

// Frame builds the frame that carries an ask, which is what a test
// needs and what the peripheral does in hardware.
func (a Ask) Frame() []byte {
	f := make([]byte, FrameLen)
	for i := 0; i < 6; i++ {
		f[i] = 0xff
	}
	f[6] = 0x02
	f[11] = a.Device
	binary.BigEndian.PutUint16(f[12:14], EtherType)
	f[14] = KindAsk
	f[15] = a.Device
	f[16] = a.Tag
	if a.Write {
		f[17] = 1
	}
	binary.BigEndian.PutUint32(f[18:22], a.Addr)
	binary.BigEndian.PutUint32(f[22:26], a.Data)
	f[26] = a.Strb
	return f
}
