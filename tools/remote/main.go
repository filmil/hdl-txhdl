// SPDX-License-Identifier: Apache-2.0

// The program at the other end of a Remote peripheral.
//
// It reads frames, answers the ones addressed to the device it is
// serving, and writes the answers back. What carries the frames is
// not its business: a length-prefixed stream on a TCP connection is
// what it speaks, because that is what survives an ssh tunnel, and
// the shim that puts raw frames on and off a wire sits at the other
// end of that connection, next to the board.
//
//	# on the machine with the board, a shim bridges the wire:
//	#   socat, or a raw socket, or whatever the network allows
//	# on a developer's machine:
//	bazel run //tools/remote -- -addr localhost:9797 -device 3
//
// The device it serves here is a memory: words written are kept and
// read back, and a read of an address nothing has written is refused,
// which is what a peripheral does before it does anything else. A
// peripheral being designed replaces `serve` with what it means to
// do, which is the whole point of issue 297: the behaviour is a
// program long before it is hardware, and the bus cannot tell.
package main

import (
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"os"

	"github.com/filmil/txhdl/tools/remote/stream"
)

// A Device answers transactions. A read returns the word and whether
// the read failed; a write takes the word and says whether it failed.
type Device interface {
	Read(addr uint32) (uint32, bool)
	Write(addr uint32, data uint32, strb uint8) bool
}

// Memory is the device this program serves unless it is changed: it
// keeps what is written and refuses a read of anything else.
type Memory struct {
	words map[uint32]uint32
}

// NewMemory makes an empty one.
func NewMemory() *Memory {
	return &Memory{words: make(map[uint32]uint32)}
}

// Read returns the word at addr, and true when there is none.
func (m *Memory) Read(addr uint32) (uint32, bool) {
	w, ok := m.words[addr]
	return w, !ok
}

// Write keeps the word, taking only the bytes the strobe covers.
func (m *Memory) Write(addr, data uint32, strb uint8) bool {
	old := m.words[addr]
	var out uint32
	for i := 0; i < 4; i++ {
		shift := 8 * i
		mask := uint32(0xff) << shift
		if strb&(1<<i) != 0 {
			out |= data & mask
		} else {
			out |= old & mask
		}
	}
	m.words[addr] = out
	return false
}

// serve answers one ask with the device's behaviour.
func serve(d Device, a Ask) Answer {
	ans := Answer{Device: a.Device, Tag: a.Tag}
	if a.Write {
		ans.Err = d.Write(a.Addr, a.Data, a.Strb)
		return ans
	}
	ans.Data, ans.Err = d.Read(a.Addr)
	return ans
}

// A frame on the connection is its length in two bytes and then the
// frame, which is what `stream` does and what the shim writes.
// run serves one connection until it ends.
func run(c io.ReadWriter, device uint8, d Device, verbose bool) error {
	for {
		f, err := stream.Read(c)
		if err != nil {
			return err
		}
		ask, err := ParseAsk(f)
		if err != nil {
			// A frame of another protocol, or this device's own ask
			// coming back on a looped wire. Both are ordinary.
			continue
		}
		if ask.Device != device {
			continue
		}
		ans := serve(d, ask)
		if verbose {
			kind := "read "
			if ask.Write {
				kind = "write"
			}
			log.Printf("%s %#08x tag %d -> %#08x err %v",
				kind, ask.Addr, ask.Tag, ans.Data, ans.Err)
		}
		if err := stream.Write(c, ans.Frame()); err != nil {
			return err
		}
	}
}

func main() {
	addr := flag.String("addr", "", "the shim to connect to, host:port")
	listen := flag.String("listen", "", "or an address to listen on")
	device := flag.Int("device", 0, "the device number this answers")
	verbose := flag.Bool("v", false, "print every transaction")
	flag.Parse()
	if (*addr == "") == (*listen == "") {
		fmt.Fprintln(os.Stderr, "give exactly one of -addr and -listen")
		os.Exit(2)
	}
	d := NewMemory()
	if *listen != "" {
		l, err := net.Listen("tcp", *listen)
		if err != nil {
			log.Fatal(err)
		}
		log.Printf("device %d, waiting on %s", *device, *listen)
		for {
			c, err := l.Accept()
			if err != nil {
				log.Fatal(err)
			}
			if err := run(c, uint8(*device), d, *verbose); err != nil &&
				err != io.EOF {
				log.Printf("the connection ended: %v", err)
			}
			c.Close()
		}
	}
	c, err := net.Dial("tcp", *addr)
	if err != nil {
		log.Fatal(err)
	}
	defer c.Close()
	log.Printf("device %d, on %s", *device, *addr)
	if err := run(c, uint8(*device), d, *verbose); err != nil &&
		err != io.EOF {
		log.Fatal(err)
	}
}
