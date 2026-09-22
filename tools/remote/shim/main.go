// SPDX-License-Identifier: Apache-2.0

package main

import (
	"flag"
	"fmt"
	"io"
	"log"
	"net"
	"os"
)

func main() {
	iface := flag.String("iface", "", "the interface the board is wired to")
	addr := flag.String("addr", "", "the program to connect to, host:port")
	listen := flag.String("listen", "", "or an address to listen on")
	verbose := flag.Bool("v", false, "print every frame")
	flag.Parse()
	if *iface == "" {
		fmt.Fprintln(os.Stderr, "-iface is required")
		os.Exit(2)
	}
	if (*addr == "") == (*listen == "") {
		fmt.Fprintln(os.Stderr, "give exactly one of -addr and -listen")
		os.Exit(2)
	}
	// One connection at a time, each with a wire of its own, so that
	// a program that reconnects finds the interface fresh and a frame
	// received between connections is not held for the next one.
	serve := func(c net.Conn) {
		defer c.Close()
		w, err := openWire(*iface)
		if err != nil {
			log.Printf("%v", err)
			return
		}
		defer w.Close()
		log.Printf("pumping %s <-> %s", *iface, c.RemoteAddr())
		if err := Pump(w, c, *verbose); err != nil && err != io.EOF {
			log.Printf("the connection ended: %v", err)
		}
	}
	if *listen != "" {
		l, err := net.Listen("tcp", *listen)
		if err != nil {
			log.Fatal(err)
		}
		log.Printf("interface %s, waiting on %s", *iface, *listen)
		for {
			c, err := l.Accept()
			if err != nil {
				log.Fatal(err)
			}
			serve(c)
		}
	}
	c, err := net.Dial("tcp", *addr)
	if err != nil {
		log.Fatal(err)
	}
	serve(c)
}
