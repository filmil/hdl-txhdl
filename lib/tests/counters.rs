// SPDX-License-Identifier: Apache-2.0
//! Two processes of one unit, each lowered to a state machine with a
//! counted loop that waits (#1893): each loop's counter is a register
//! of its own, `for0` and `for1`, declared once each.
use txhdl::comp::{join2, Clock, DefaultClock, Reg, Unit};
use txhdl::types::U;
use txhdl::{lower, Trace};

/// Counts up `a` every fourth edge and `b` every sixth, each with a
/// counted loop between.
#[derive(Trace, Default)]
pub struct Twice {
    pub a: Reg<U<4>>,
    pub b: Reg<U<4>>,
}

#[lower]
impl Unit for Twice {
    async fn run(&mut self, _i: (), _o: ()) {
        join2(
            async {
                loop {
                    DefaultClock::rising().await;
                    for _ in 0..3 {
                        DefaultClock::rising().await;
                    }
                    self.a.set(self.a + 1);
                }
            },
            async {
                loop {
                    DefaultClock::rising().await;
                    for _ in 0..5 {
                        DefaultClock::rising().await;
                    }
                    self.b.set(self.b + 1);
                }
            },
        )
        .await;
    }
}

/// How many lines of `text` contain `what`.
fn lines_with(text: &str, what: &str) -> usize {
    text.lines().filter(|l| l.contains(what)).count()
}

#[test]
fn each_process_has_its_own_counter() {
    let v = Twice::lowered("twice").verilog();
    // Each declared once, as wide as its own loop needs: 0..3 counts
    // to 2 in two bits, 0..5 to 4 in three. Before #1893 both were
    // `for0`, declared twice.
    assert_eq!(lines_with(&v, "reg [1:0] for0 ="), 1, "{v}");
    assert_eq!(lines_with(&v, "reg [2:0] for1 ="), 1, "{v}");
    assert_eq!(lines_with(&v, " for0 ="), 1, "{v}");
    assert_eq!(lines_with(&v, " for1 ="), 1, "{v}");
    // The second machine counts with its own: it compares `for1` with
    // its last value, 4, and never `for0`.
    assert!(v.contains("(for1 != 3'h4)"), "{v}");
    assert!(!v.contains("(for0 != 3'h4)"), "{v}");
    assert!(v.contains("(for0 != 2'h2)"), "{v}");
    let h = Twice::lowered("twice").vhdl();
    assert_eq!(lines_with(&h, "signal for0 "), 1, "{h}");
    assert_eq!(lines_with(&h, "signal for1 "), 1, "{h}");
}
