// SPDX-License-Identifier: Apache-2.0
//! A process of several waits, lowered to a state machine (issue 501).
//!
//! A loop body that waits more than once is cut at its waits into
//! segments, one per wait, and the segments are the states of a
//! machine: a hidden register, `at_wait`, holds the number of the
//! wait the process is at, and each segment's statements happen at
//! the edge when the register holds its number and the wait's own
//! condition holds. The register then moves to the next wait, and
//! after the last it moves back to the first, which is the loop.
//!
//! What a segment may hold is what a loop of one wait may hold after
//! its wait, with the differences a state brings:
//!
//! * a register is driven inside the state's arm, so it changes only
//!   at the edge that leaves the state;
//! * a `send` offers while the process is in the state and its wait's
//!   condition holds, so `valid` is the state's condition, and a
//!   channel sent from several states offers the state's data;
//! * a receive takes in its state alone, so `ready` is the state's
//!   condition;
//! * an output port is a wire and has no memory, so it is set after
//!   every wait, and the wire is the state's expression, selected on
//!   the register. That reproduces the run only when no wait can
//!   stall, so an output is refused in a process that waits on a
//!   condition or on a channel; such a process drives a register, or
//!   sends;
//! * a `let` is a wire too, and a wire computed in one state is not
//!   what it was by the next, so a name bound in one segment is not
//!   read in a later one; what crosses a wait is held in a register.
//!
//! A `for` whose body waits is unrolled first, a state per turn with
//! the variable a number in each, so its bounds are written out; a
//! `for` that does not wait is left to the state's own lowering.
//!
//! Every wait is on one clock and one edge, since the machine is one
//! clocked block.
use super::{ebin, ename, err, is_ident, lower_stmts, stmts_of, Cx};
use proc_macro::{Delimiter, Group, Literal, Span, TokenStream, TokenTree};

/// The name of the hidden register of the `k`th process of several
/// waits in a unit, counted from one.
pub(crate) fn reg_name(k: usize) -> String {
    if k == 1 {
        "at_wait".to_string()
    } else {
        format!("at_wait{k}")
    }
}

/// A binary literal of `w` bits holding `v`.
fn bits(w: usize, v: usize) -> String {
    format!("NlE::Bits({w}, \"{:0>w$b}\".to_string())", v, w = w)
}

/// The text of a statement, as `lower_stmts` reads it: the tokens
/// written out with nothing between them.
fn text(ts: &[TokenTree]) -> String {
    ts.iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join("")
}

/// Whether a statement is a wait: it ends in `.await`.
pub(crate) fn is_wait(ts: &[TokenTree]) -> bool {
    text(ts).ends_with(".await")
}

/// The expression a drive of `net` assigns, if `s` is such a drive:
/// the text between the target and the closing parenthesis.
fn drive_of<'a>(s: &'a str, net: &str) -> Option<&'a str> {
    let head = format!("NlS::Drive(NlT::Name(\"{net}\".to_string()), ");
    s.strip_prefix(&head).and_then(|r| r.strip_suffix(')'))
}

/// The net a drive assigns, if `s` is a drive of a plain name.
fn driven(s: &str) -> Option<&str> {
    let r = s.strip_prefix("NlS::Drive(NlT::Name(\"")?;
    let end = r.find('"')?;
    Some(&r[..end])
}

/// The first identifier among `ts`, at any depth, that is one of
/// `names`, unless it follows a `.`, which makes it a field or a
/// method rather than a name of the body.
fn first_use(ts: &[TokenTree], names: &[String]) -> Option<(String, Span)> {
    let mut prev_dot = false;
    for t in ts {
        match t {
            TokenTree::Ident(i) if !prev_dot => {
                let s = i.to_string();
                if names.contains(&s) {
                    return Some((s, i.span()));
                }
            }
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                if let Some(u) = first_use(&inner, names) {
                    return Some(u);
                }
            }
            _ => {}
        }
        prev_dot = matches!(t, TokenTree::Punct(p) if p.as_char() == '.');
    }
    None
}

/// The names a `let` statement binds: one, or the members of a tuple.
fn bound_by(ts: &[TokenTree]) -> Vec<String> {
    if ts.len() < 2
        || !matches!(&ts[0], TokenTree::Ident(i) if i.to_string() == "let")
    {
        return Vec::new();
    }
    match &ts[1] {
        TokenTree::Ident(i) => vec![i.to_string()],
        TokenTree::Group(g) => g
            .stream()
            .into_iter()
            .filter_map(|t| match t {
                TokenTree::Ident(i) => Some(i.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The statements of a loop body of several waits, as the Rust source
/// of the netlist's statements, and the width of the hidden register
/// called `reg`. `sts` are the body's statements, the first a wait;
/// `pnames` the unit's ports; `span` where the loop is, for what is
/// refused about the whole of it.
pub(crate) fn lower(
    cx: &mut Cx,
    sts: &[Vec<TokenTree>],
    reg: &str,
    pnames: &[String],
    span: Span,
) -> Result<(Vec<String>, usize), TokenStream> {
    // A `for` that waits is unrolled first; then the segments, each
    // starting at a wait.
    let sts = unrolled(sts)?;
    let mut segs: Vec<Vec<&Vec<TokenTree>>> = Vec::new();
    for st in &sts {
        if is_wait(st) {
            segs.push(vec![st]);
        } else if let Some(last) = segs.last_mut() {
            last.push(st);
        } else {
            return Err(err(
                st[0].span(),
                "a loop must start by waiting for an edge",
            ));
        }
    }
    let n = segs.len();
    let width =
        std::cmp::max(1, usize::BITS - (n - 1).leading_zeros()) as usize;
    let at = |k: usize| ebin("==", &ename(reg), &bits(width, k));
    // Whether a port's name is a wire of the netlist: an output, or a
    // channel's data, valid or ready.
    let is_port_net = |net: &str| -> bool {
        pnames.iter().any(|p| p == net)
            || ["_data", "_valid", "_ready"].iter().any(|end| {
                net.strip_suffix(end)
                    .is_some_and(|p| pnames.iter().any(|q| q == p))
            })
    };
    // The combinational drives of every state: the net, the state,
    // its condition, and the expression.
    let mut comb: Vec<(String, usize, String, String)> = Vec::new();
    let mut arms: Vec<String> = Vec::new();
    // Names bound by earlier segments' `let`s, which a later one may
    // not read.
    let mut earlier: Vec<String> = Vec::new();
    let mut clock: Option<(String, bool)> = None;
    let mut guarded = false;
    for (k, seg) in segs.iter().enumerate() {
        let start = cx.subst.len();
        cx.guard = None;
        cx.clock.clear();
        cx.falling = false;
        cx.hoisted.clear();
        // The wait: its clock, its edge, and its condition, which
        // joins the state's.
        let head = lower_stmts(cx, seg[0], None)?;
        let mut cond = at(k);
        let mut seq: Vec<String> = Vec::new();
        for s in head {
            if let Some(g) = s.strip_prefix("NlS::Guard(") {
                let g = g.strip_suffix(')').unwrap_or(g);
                cond = ebin("&&", &cond, g);
                guarded = true;
            } else if let Some(net) = driven(&s) {
                if is_port_net(net) {
                    let e = drive_of(&s, net).unwrap_or("").to_string();
                    comb.push((net.to_string(), k, cond.clone(), e));
                } else {
                    seq.push(s);
                }
            } else {
                seq.push(s);
            }
        }
        // A wait on a channel names no clock and takes the process's;
        // the waits that name one name the same.
        match &clock {
            _ if cx.clock.is_empty() => {}
            None => clock = Some((cx.clock.clone(), cx.falling)),
            Some((c, f)) if *c != cx.clock || *f != cx.falling => {
                return Err(err(
                    seg[0][0].span(),
                    "a process of several waits waits on one clock, at \
                     one edge",
                ));
            }
            Some(_) => {}
        }
        cx.guard = Some(cond.clone());
        // The statements of the state: each lowered as it would be
        // after a single wait, then sorted into the state's arm or
        // the wires.
        for st in &seg[1..] {
            let fresh = bound_by(st);
            earlier.retain(|e| !fresh.contains(e));
            let from = if fresh.is_empty() { 0 } else { 2 };
            if let Some((name, sp)) = first_use(&st[from..], &earlier) {
                return Err(err(
                    sp,
                    &format!(
                        "`{name}` is bound before an earlier wait, and a \
                         `let` is a wire that does not hold: a value that \
                         crosses a wait is kept in a register"
                    ),
                ));
            }
            let mut out = lower_stmts(cx, st, None)?;
            out.append(&mut cx.hoisted);
            for s in out {
                match driven(&s) {
                    Some(net) if is_port_net(net) => {
                        let e = drive_of(&s, net).unwrap_or("").to_string();
                        comb.push((net.to_string(), k, cond.clone(), e));
                    }
                    _ => seq.push(s),
                }
            }
        }
        for (name, _) in cx.subst.drain(start..) {
            if !earlier.contains(&name) {
                earlier.push(name);
            }
        }
        seq.push(format!(
            "NlS::Drive(NlT::Name(\"{reg}\".to_string()), {})",
            bits(width, (k + 1) % n)
        ));
        arms.push(format!(
            "NlS::If(vec![({cond}, vec![{}])], vec![])",
            seq.join(",\n")
        ));
    }
    cx.guard = None;
    // The clock is the process's; none named is refused where a loop of
    // one wait is.
    if let Some((c, f)) = clock {
        cx.clock = c;
        cx.falling = f;
    } else {
        cx.clock.clear();
    }
    // The wires, one drive per net: a handshake is the union of its
    // states' conditions; data, and an output, the state's expression
    // selected on the register.
    let mut stmts: Vec<String> = Vec::new();
    let mut nets: Vec<String> = Vec::new();
    for (net, _, _, _) in &comb {
        if !nets.contains(net) {
            nets.push(net.clone());
        }
    }
    let one = "NlE::Bits(1, \"1\".to_string())";
    for net in nets {
        let mine: Vec<&(String, usize, String, String)> =
            comb.iter().filter(|(x, _, _, _)| *x == net).collect();
        let e = if net.ends_with("_valid") || net.ends_with("_ready") {
            let mut acc: Option<String> = None;
            for (_, _, cond, e) in &mine {
                // A handshake lowered after a wait carries the guard,
                // which is the state's condition here; one asserted
                // outright takes the condition.
                let term = if e == one {
                    cond.clone()
                } else if e.contains(cond.as_str()) {
                    e.clone()
                } else {
                    ebin("&&", cond, e)
                };
                acc = Some(match acc {
                    None => term,
                    Some(a) => ebin("||", &a, &term),
                });
            }
            acc.unwrap_or_default()
        } else {
            let is_out = !net.ends_with("_data");
            if is_out {
                if guarded {
                    return Err(err(
                        span,
                        &format!(
                            "`{net}` is an output, a wire, and a wire cannot \
                             hold what one state set while the next waits: \
                             a process that waits on a condition or a \
                             channel drives a register, or sends"
                        ),
                    ));
                }
                for k in 0..n {
                    let sets =
                        mine.iter().filter(|(_, s, _, _)| *s == k).count();
                    if sets != 1 {
                        return Err(err(
                            span,
                            &format!(
                                "`{net}` is set {sets} times after wait {k}: \
                                 an output of a process of several waits is \
                                 a wire, and is set once after every wait"
                            ),
                        ));
                    }
                }
            }
            let mut sorted: Vec<&&(String, usize, String, String)> =
                mine.iter().collect();
            sorted.sort_by_key(|(_, k, _, _)| *k);
            let (_, _, _, last) = sorted[sorted.len() - 1];
            let mut acc = last.clone();
            for (_, k, _, e) in sorted.iter().rev().skip(1) {
                acc = format!(
                    "NlE::Cond(Box::new({}), Box::new({e}), Box::new({acc}))",
                    at(*k)
                );
            }
            acc
        };
        stmts
            .push(format!("NlS::Drive(NlT::Name(\"{net}\".to_string()), {e})"));
    }
    stmts.extend(arms);
    Ok((stmts, width))
}

/// `for v in lo..hi { .. }`: the variable, the range's tokens and the
/// body's tokens, if `st` is such a statement.
fn for_parts(
    st: &[TokenTree],
) -> Option<(String, Vec<TokenTree>, Vec<TokenTree>)> {
    if st.len() < 5 || !is_ident(&st[0], "for") || !is_ident(&st[2], "in") {
        return None;
    }
    let TokenTree::Ident(v) = &st[1] else {
        return None;
    };
    let Some(TokenTree::Group(g)) = st.last() else {
        return None;
    };
    if g.delimiter() != Delimiter::Brace {
        return None;
    }
    let body: Vec<TokenTree> = g.stream().into_iter().collect();
    Some((v.to_string(), st[3..st.len() - 1].to_vec(), body))
}

/// How many waits the statements hold, those inside a `for` counted
/// with the rest. A wait under `if` is refused where the arm is
/// lowered, so it is not looked for.
pub(crate) fn waits_in(sts: &[Vec<TokenTree>]) -> usize {
    sts.iter()
        .map(|st| match for_parts(st) {
            _ if is_wait(st) => 1,
            Some((_, _, body)) => waits_in(&stmts_of(&body)),
            None => 0,
        })
        .sum()
}

/// The bounds of a range written as numbers, `lo..hi` or `lo..=hi`.
fn bounds(range: &[TokenTree]) -> Option<(usize, usize)> {
    let t: String = range.iter().map(|t| t.to_string()).collect();
    let (lo, hi, closed) = match t.split_once("..=") {
        Some((a, b)) => (a, b, true),
        None => {
            let (a, b) = t.split_once("..")?;
            (a, b, false)
        }
    };
    let num = |s: &str| -> Option<usize> {
        let s = s.trim();
        let digits = s.trim_end_matches(|c: char| c.is_ascii_alphabetic());
        digits.parse().ok()
    };
    let (lo, hi) = (num(lo)?, num(hi)?);
    Some((lo, if closed { hi + 1 } else { hi }))
}

/// `ts` with the name `var` replaced by the number `k`, at any depth;
/// a name after `.` is a field or a method and is left alone.
fn substituted(ts: &[TokenTree], var: &str, k: usize) -> Vec<TokenTree> {
    let mut out = Vec::with_capacity(ts.len());
    let mut prev_dot = false;
    for t in ts {
        let is_dot = matches!(t, TokenTree::Punct(p) if p.as_char() == '.');
        out.push(match t {
            TokenTree::Ident(i) if !prev_dot && i.to_string() == var => {
                let mut l = Literal::usize_unsuffixed(k);
                l.set_span(i.span());
                TokenTree::Literal(l)
            }
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                let mut n = Group::new(
                    g.delimiter(),
                    substituted(&inner, var, k).into_iter().collect(),
                );
                n.set_span(g.span());
                TokenTree::Group(n)
            }
            t => t.clone(),
        });
        prev_dot = is_dot;
    }
    out
}

/// The statements with every `for` that waits unrolled: the body once
/// per value of the range, the variable a number in each, nested
/// ones unrolled inside. A `for` that does not wait is left to the
/// state's lowering, which unrolls it when `lowered` runs and so
/// takes a bound the macro cannot see; a `for` that waits is a state
/// per turn, and the macro must count them, so its bounds are written
/// out.
fn unrolled(
    sts: &[Vec<TokenTree>],
) -> Result<Vec<Vec<TokenTree>>, TokenStream> {
    let mut out = Vec::new();
    for st in sts {
        let Some((var, range, body)) = for_parts(st) else {
            out.push(st.clone());
            continue;
        };
        let inner = stmts_of(&body);
        if waits_in(&inner) == 0 {
            out.push(st.clone());
            continue;
        }
        let Some((lo, hi)) = bounds(&range) else {
            return Err(err(
                st[0].span(),
                "a `for` that waits is a state per turn, counted by the \
                 macro, so its bounds are numbers written out: `for i in \
                 0..8`",
            ));
        };
        for k in lo..hi {
            let turn: Vec<Vec<TokenTree>> =
                inner.iter().map(|s| substituted(s, &var, k)).collect();
            out.extend(unrolled(&turn)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{bits, drive_of, driven, reg_name};

    #[test]
    fn the_register_is_numbered_from_the_second_process() {
        assert_eq!(reg_name(1), "at_wait");
        assert_eq!(reg_name(2), "at_wait2");
    }

    #[test]
    fn a_state_number_is_a_literal_of_the_register_width() {
        assert_eq!(bits(1, 0), "NlE::Bits(1, \"0\".to_string())");
        assert_eq!(bits(2, 3), "NlE::Bits(2, \"11\".to_string())");
        assert_eq!(bits(3, 2), "NlE::Bits(3, \"010\".to_string())");
    }

    #[test]
    fn a_drive_is_read_back_by_its_net() {
        let s = "NlS::Drive(NlT::Name(\"sums_valid\".to_string()), \
                 NlE::Name(\"go\".to_string()))";
        assert_eq!(driven(s), Some("sums_valid"));
        let e = "NlE::Name(\"go\".to_string())";
        assert_eq!(drive_of(s, "sums_valid"), Some(e));
        assert_eq!(drive_of(s, "held"), None);
        assert_eq!(driven("NlS::Guard(x)"), None);
        assert_eq!(driven("NlS::Drive(NlT::Word(m, a), e)"), None);
    }
}
