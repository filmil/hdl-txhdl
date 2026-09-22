// SPDX-License-Identifier: Apache-2.0

package main

import (
	"bytes"
	"errors"
	"net"
	"testing"
	"time"

	"github.com/filmil/txhdl/tools/remote/stream"
)

// A wire in memory: what arrives on it is fed through `in`, and what
// is sent on it comes out of `out`.
type fakeWire struct {
	in  chan []byte
	out chan []byte
}

var errWireClosed = errors.New("the wire is closed")

func (w *fakeWire) Recv() ([]byte, error) {
	f, ok := <-w.in
	if !ok {
		return nil, errWireClosed
	}
	return f, nil
}

func (w *fakeWire) Send(f []byte) error {
	w.out <- f
	return nil
}

// A frame of the protocol, of the peripheral's length, and one of
// another protocol.
func frame(kind, device byte) []byte {
	f := make([]byte, 27)
	for i := 0; i < 6; i++ {
		f[i] = 0xff
	}
	f[6] = 0x02
	f[11] = device
	f[12], f[13] = 0x88, 0xb5
	f[14], f[15] = kind, device
	return f
}

func foreign() []byte {
	f := make([]byte, 60)
	f[12], f[13] = 0x08, 0x00 // IPv4
	return f
}

// The peripheral's frames go from the wire to the stream, with their
// length in front; another protocol's do not; and an answer written to
// the stream leaves on the wire as it was.
func TestFramesGoBothWaysAndOnlyOurs(t *testing.T) {
	w := &fakeWire{in: make(chan []byte, 4), out: make(chan []byte, 4)}
	near, far := net.Pipe()
	done := make(chan error, 1)
	go func() { done <- Pump(w, near, false) }()

	w.in <- foreign()
	ask := frame(0x01, 3)
	w.in <- ask
	got, err := stream.Read(far)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, ask) {
		t.Fatalf("on the stream %x, want %x", got, ask)
	}

	ans := frame(0x81, 3)
	if err := stream.Write(far, ans); err != nil {
		t.Fatal(err)
	}
	select {
	case sent := <-w.out:
		if !bytes.Equal(sent, ans) {
			t.Fatalf("on the wire %x, want %x", sent, ans)
		}
	case <-time.After(time.Second):
		t.Fatal("the answer never reached the wire")
	}

	// Closing the wire ends the pump with the wire's own error.
	close(w.in)
	select {
	case err := <-done:
		if err != errWireClosed {
			t.Fatalf("the pump ended with %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("the pump did not end")
	}
	far.Close()
}

// The stream ending ends the pump too, so a program that hangs up
// frees the interface for the next one.
func TestTheStreamEndingEndsThePump(t *testing.T) {
	w := &fakeWire{in: make(chan []byte), out: make(chan []byte, 1)}
	near, far := net.Pipe()
	done := make(chan error, 1)
	go func() { done <- Pump(w, near, false) }()
	far.Close()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("the pump ended without an error")
		}
	case <-time.After(time.Second):
		t.Fatal("the pump did not end")
	}
	close(w.in)
}
