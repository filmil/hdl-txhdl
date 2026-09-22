// SPDX-License-Identifier: Apache-2.0

//go:build linux

package main

import (
	"fmt"
	"net"
	"syscall"
)

// A packetWire is a raw socket on one interface, bound to the
// peripheral's type, so that the kernel delivers those frames and no
// others. Opening one needs `CAP_NET_RAW`: root, or the binary given
// the capability with `setcap cap_net_raw+ep`.
type packetWire struct {
	fd      int
	ifindex int
}

// htons is the byte order the socket layer wants a protocol in.
func htons(v uint16) uint16 {
	return v<<8 | v>>8
}

// openWire binds a raw socket to the named interface.
func openWire(name string) (*packetWire, error) {
	ifi, err := net.InterfaceByName(name)
	if err != nil {
		return nil, err
	}
	proto := htons(EtherType)
	fd, err := syscall.Socket(syscall.AF_PACKET, syscall.SOCK_RAW, int(proto))
	if err != nil {
		return nil, fmt.Errorf("raw socket: %w (cap_net_raw needed)", err)
	}
	sa := &syscall.SockaddrLinklayer{Protocol: proto, Ifindex: ifi.Index}
	if err := syscall.Bind(fd, sa); err != nil {
		syscall.Close(fd)
		return nil, fmt.Errorf("bind to %s: %w", name, err)
	}
	return &packetWire{fd: fd, ifindex: ifi.Index}, nil
}

// Recv waits for one frame. The buffer is the longest frame the wire
// carries, so a frame arrives whole or not at all.
func (p *packetWire) Recv() ([]byte, error) {
	buf := make([]byte, 2048)
	n, _, err := syscall.Recvfrom(p.fd, buf, 0)
	if err != nil {
		return nil, err
	}
	return buf[:n], nil
}

// Send puts one frame on the wire as it is: the header is the frame's
// own, and the kernel pads it to the minimum the wire needs.
func (p *packetWire) Send(f []byte) error {
	if len(f) < 14 {
		return fmt.Errorf("a frame of %d bytes has no header", len(f))
	}
	var dst [8]byte
	copy(dst[:], f[0:6])
	sa := &syscall.SockaddrLinklayer{Ifindex: p.ifindex, Halen: 6, Addr: dst}
	return syscall.Sendto(p.fd, f, 0, sa)
}

// Close ends the wire, which also ends a Recv waiting on it.
func (p *packetWire) Close() error {
	return syscall.Close(p.fd)
}
