// SPDX-License-Identifier: Apache-2.0

// The shim beside the board: raw Ethernet frames on one side, the
// stream `//tools/remote` speaks on the other.
//
// A Remote peripheral in a design sends its transactions as frames on
// the board's Ethernet port, and the program that answers them runs
// on a developer's machine, reached through an ssh tunnel that
// carries a byte stream and nothing else. This program runs on the
// machine the board is wired to, reads the frames off that interface,
// writes each to the stream with its length in front, and puts the
// answers that come back the other way onto the wire, as they are.
// It knows the protocol's type and nothing else about it: which
// frames are the peripheral's is the type in bytes 12 and 13, and
// what is in them is the business of the two ends.
//
//	# on the machine with the board, as root or with cap_net_raw:
//	shim -iface enp3s0 -listen :9797
//	# on a developer's machine, through a tunnel to that port:
//	bazel run //tools/remote -- -addr localhost:9797 -device 3
//
// The pump between the two sides is written against an interface, so
// that it is tested here with a wire in memory; the raw socket that
// is the wire on Linux is in `packet_linux.go`.
package main

import (
	"encoding/binary"
	"io"
	"log"

	"github.com/filmil/txhdl/tools/remote/stream"
)

// EtherType is the Remote peripheral's, the one IEEE leaves for local
// experimental use; it is the same constant as in `//tools/remote`
// and in `lib/parts/src/remote/eth.rs`.
const EtherType = 0x88b5

// A Wire carries frames: one whole frame per call, each way.
type Wire interface {
	// Recv waits for the next frame and returns it whole.
	Recv() ([]byte, error)
	// Send puts one frame on the wire, as given.
	Send(f []byte) error
}

// ours says whether a frame carries the peripheral's protocol. A
// wire bound to the type delivers nothing else, but a wire in a test
// may, and a frame too short to have a type is nobody's.
func ours(f []byte) bool {
	return len(f) >= 14 && binary.BigEndian.Uint16(f[12:14]) == EtherType
}

// Pump moves frames both ways until either side fails, and returns
// that failure. The two directions run at once, since an answer may
// be on its way while the next ask arrives. Whoever called it closes
// both sides afterwards, which is what ends the other direction.
func Pump(w Wire, s io.ReadWriter, verbose bool) error {
	errs := make(chan error, 2)
	go func() {
		for {
			f, err := w.Recv()
			if err != nil {
				errs <- err
				return
			}
			if !ours(f) {
				continue
			}
			if verbose {
				log.Printf("wire -> stream, %d bytes, kind %#02x device %d",
					len(f), f[14], f[15])
			}
			if err := stream.Write(s, f); err != nil {
				errs <- err
				return
			}
		}
	}()
	go func() {
		for {
			f, err := stream.Read(s)
			if err != nil {
				errs <- err
				return
			}
			if verbose {
				log.Printf("stream -> wire, %d bytes, kind %#02x device %d",
					len(f), f[14], f[15])
			}
			if err := w.Send(f); err != nil {
				errs <- err
				return
			}
		}
	}()
	return <-errs
}
