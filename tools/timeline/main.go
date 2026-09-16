// SPDX-License-Identifier: Apache-2.0
//
// The project's timeline, measured from its own history.
//
// Run by hand, not by the build: a Bazel action has no repository to
// read. It writes docs/timeline.tsv, which the build then draws. So
// the chart in the document is generated, and the numbers behind it
// are measured rather than typed:
//
//	go run tools/timeline/main.go > docs/timeline.tsv
//
// A task is a set of paths. Its span is the first and last commit
// that touched any of them, and its weight is how many commits did.
// Paths rather than commit messages, because a path is what a change
// actually moved; a message is what it said it moved.
package main

import (
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strings"
)

// One band of the chart: what it is called, which paths it is, and
// which earlier tasks had to exist before it could start. The
// dependencies are stated here and not measured, because a repository
// records what changed and never why it could not have changed
// sooner.
type task struct {
	name  string
	paths []string
	needs []string
}

var tasks = []task{
	{"The specification", []string{"docs/spec", "spec"}, nil},
	{"Values and units", []string{"lib/src/types.rs", "lib/src/comp.rs"}, []string{"The specification"}},
	{"Traces", []string{"lib/src/comp.rs"}, []string{"Values and units"}},
	{"The lowering", []string{"lib/macros/lib.rs", "lib/src/netlist.rs"}, []string{"Values and units"}},
	{"Examples", []string{"lib/examples"}, []string{"Values and units"}},
	{"The documents", []string{"docs"}, []string{"Examples"}},
	{"Vreteno", []string{"cpu/vreteno/src"}, []string{"The lowering"}},
	{"Foreign units", []string{"lib/src/foreign.rs", "tools/vshim"}, []string{"The lowering"}},
	{"Synthesis and the board", []string{"cpu/vreteno/board", "cpu/vreteno/vreteno.xdc"}, []string{"Vreteno"}},
	{"Parts", []string{"lib/parts/src/buffer.rs", "lib/parts/src/fifo.rs", "lib/parts/src/station.rs"}, []string{"The lowering"}},
	{"The AXI link", []string{"lib/parts/src/bus/axi.rs", "lib/parts/src/bus/mod.rs"}, []string{"Parts"}},
	{"The AXI router", []string{"lib/parts/src/bus/router.rs"}, []string{"The AXI link"}},
	// The devices were written for the core's own bus and moved onto
	// AXI later, so the band is what they are and not where they sit.
	{"The devices", []string{"cpu/vreteno/src/dmem.rs", "cpu/vreteno/src/uart.rs", "cpu/vreteno/src/timer.rs"}, []string{"Vreteno"}},
	{"A GPU", []string{"gpu"}, []string{"The AXI link"}},
	{"Rust for the core", []string{"cpu/vreteno/rust", "tools/elf2vreteno"}, []string{"Vreteno"}},
	{"C++ for the core", []string{"cpu/vreteno/cpp"}, []string{"Rust for the core"}},
}

// How many commits touched any of `paths`, per day.
func byDay(paths []string) (map[string]int, error) {
	args := append([]string{
		"log", "--format=%ad", "--date=format:%Y-%m-%d", "--",
	}, paths...)
	out, err := exec.Command("git", args...).Output()
	if err != nil {
		return nil, err
	}
	days := map[string]int{}
	for _, l := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if l != "" {
			days[l]++
		}
	}
	return days, nil
}

// Every day the project was worked on at all, in order. The chart's
// axis is these and not the calendar, because the calendar between
// them is four months of nothing and would be most of the picture.
func workdays() ([]string, map[string]int, error) {
	days, err := byDay(nil)
	if err != nil {
		return nil, nil, err
	}
	var all []string
	for d := range days {
		all = append(all, d)
	}
	sort.Strings(all)
	return all, days, nil
}

func main() {
	all, total, err := workdays()
	if err != nil {
		fmt.Fprintln(os.Stderr, "timeline:", err)
		os.Exit(1)
	}
	fmt.Println("# The project's timeline, from its own history.")
	fmt.Println("# Written by tools/timeline/main.go; do not edit.")
	fmt.Println("# The axis: every day the project was worked on.")
	for _, d := range all {
		fmt.Printf("day\t%s\t%d\n", d, total[d])
	}
	fmt.Println("# A band each: name, what it needs, then a count per")
	fmt.Println("# day it was worked on.")
	for _, t := range tasks {
		days, err := byDay(t.paths)
		if err != nil {
			fmt.Fprintf(os.Stderr, "timeline: %s: %v\n", t.name, err)
			os.Exit(1)
		}
		if len(days) == 0 {
			continue // a task whose paths never existed
		}
		var on []string
		for _, d := range all {
			if n, ok := days[d]; ok {
				on = append(on, fmt.Sprintf("%s:%d", d, n))
			}
		}
		fmt.Printf("task\t%s\t%s\t%s\n",
			t.name, strings.Join(t.needs, ";"), strings.Join(on, ","))
	}
}
