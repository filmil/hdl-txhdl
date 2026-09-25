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
//! * an output port keeps what a state set until a state sets it
//!   again, as the run's wire does, so it is held in a register the
//!   unit does not declare, `<port>_held`, set in the state's arm; the
//!   port is the state's expression while the state is about to leave
//!   and the register otherwise, which is the run's wire seen a cycle
//!   ahead, as every wire of the netlist is;
//! * a `let` is a wire too, and a wire computed in one state is not
//!   what it was by the next, so a name bound in one segment is not
//!   read in a later one; what crosses a wait is held in a register.
//!
//! A `for` whose body waits is unrolled first, a state per turn with
//! the variable a number in each, so its bounds are written out; a
//! `for` that does not wait is left to the state's own lowering.
//!
//! A wait under `if` is a state with a way around it: the state before
//! the `if` goes to the wait inside when the condition holds at its
//! edge and past the `if` otherwise, so its next value is a choice.
//! What sits in an arm before its wait happens in the state before
//! the `if`, under the arm's condition; what follows the `if` happens
//! in every state the `if` can end in, and is lowered once for all of
//! them, under the union of their conditions.
//!
//! Every wait is on one clock and one edge, since the machine is one
//! clocked block.
use super::{ebin, ename, err, is_ident, lower_stmts, stmts_of, tr, Cx};
use proc_macro::{
    Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream,
    TokenTree,
};

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

/// A path: the conditions of the `if` arms a statement sits under,
/// each as the tokens of a condition and whether it is the failure of
/// that condition, which is what the arms below it and the way past
/// an `if` with no `else` are under.
type Path = Vec<(Vec<TokenTree>, bool)>;

/// Whether a path is the way past every `if` on it: only failures, so
/// that the state's default next covers it once the branches taken
/// on the conditions come first.
fn fallthrough(p: &Path) -> bool {
    p.iter().all(|(_, failed)| *failed)
}

/// One state of the machine: the wait that opens it, the statements
/// that happen in it alone, each under its path, the branches an `if`
/// opened from here, each to the state its wait made, and the state
/// after them all.
struct State {
    wait: Vec<TokenTree>,
    items: Vec<(Path, Vec<TokenTree>)>,
    branches: Vec<(Path, usize)>,
    next: Option<usize>,
}

/// Statements that several states share: what follows an `if` whose
/// arms end in different states, or that has no `else`. Each member
/// is a state with the path it is under there, and the statements
/// are lowered once, under the union of the members' conditions,
/// rather than once per member (issue 596).
struct Tail {
    members: Vec<(usize, Path)>,
    items: Vec<Vec<TokenTree>>,
}

/// The states and the tails in the order the body makes them, which
/// is the order they are lowered in, so that a name bound in one is
/// known to the next.
enum Piece {
    State(usize),
    Tail(usize),
}

/// The conjunction of a path, `(a) && !(b)`, as tokens.
fn conj(path: &Path) -> Vec<TokenTree> {
    let mut out = Vec::new();
    for (i, (c, failed)) in path.iter().enumerate() {
        let span = c.first().map(|t| t.span()).unwrap_or_else(Span::call_site);
        if i > 0 {
            let mut a = Punct::new('&', Spacing::Joint);
            a.set_span(span);
            let mut b = Punct::new('&', Spacing::Alone);
            b.set_span(span);
            out.push(TokenTree::Punct(a));
            out.push(TokenTree::Punct(b));
        }
        if *failed {
            let mut bang = Punct::new('!', Spacing::Alone);
            bang.set_span(span);
            out.push(TokenTree::Punct(bang));
        }
        let mut g =
            Group::new(Delimiter::Parenthesis, c.iter().cloned().collect());
        g.set_span(span);
        out.push(TokenTree::Group(g));
    }
    out
}

/// The arms of `if c { .. } else if d { .. } else { .. }`: each arm's
/// condition, none for the `else`, and its body's tokens; nothing if
/// `st` is not an `if`.
fn if_parts(
    st: &[TokenTree],
) -> Option<Vec<(Option<Vec<TokenTree>>, Vec<TokenTree>)>> {
    if st.is_empty() || !is_ident(&st[0], "if") {
        return None;
    }
    let brace = |t: &TokenTree| {
        matches!(t, TokenTree::Group(g)
                if g.delimiter() == Delimiter::Brace)
    };
    let mut arms = Vec::new();
    let mut i = 0;
    loop {
        let b = st[i + 1..].iter().position(brace)?;
        if b == 0 {
            return None;
        }
        let cond = st[i + 1..i + 1 + b].to_vec();
        let TokenTree::Group(g) = &st[i + 1 + b] else {
            return None;
        };
        arms.push((Some(cond), g.stream().into_iter().collect()));
        i += 2 + b;
        match (st.get(i), st.get(i + 1)) {
            (Some(e), Some(f)) if is_ident(e, "else") && is_ident(f, "if") => {
                i += 1;
            }
            (Some(e), Some(TokenTree::Group(g)))
                if is_ident(e, "else") && brace(&st[i + 1]) =>
            {
                arms.push((None, g.stream().into_iter().collect()));
                return Some(arms);
            }
            (None, _) => return Some(arms),
            _ => return None,
        }
    }
}

/// The statements as text, one after another, for a body rebuilt
/// from its statements.
fn joined(sts: &[Vec<TokenTree>]) -> TokenStream {
    let mut out: Vec<TokenTree> = Vec::new();
    for st in sts {
        out.extend(st.iter().cloned());
        let span = st.last().map(|t| t.span()).unwrap_or_else(Span::call_site);
        let mut semi = Punct::new(';', Spacing::Alone);
        semi.set_span(span);
        out.push(TokenTree::Punct(semi));
    }
    out.into_iter().collect()
}

/// Whether two sets of open states are the same states under the
/// same paths.
fn same_members(a: &[(usize, Path)], b: &[(usize, Path)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((s, p), (t, q))| s == t && same_path(p, q))
}

/// What the walk builds: the states, the tails, and their order.
struct Plan {
    states: Vec<State>,
    tails: Vec<Tail>,
    order: Vec<Piece>,
}

/// The states of a body, walked in order. `open` are the states the
/// walk is in, each with the path it is under there: a statement goes
/// into every one of them, and a wait closes them all into a new
/// state. An `if` walks each arm from the same open states with the
/// arm's condition on the path, and what follows the `if` goes into
/// every state an arm ended in, and into the states the `if` was
/// entered from when it has no `else`. A statement that goes into
/// several states at once goes into a tail they share.
fn walk(
    plan: &mut Plan,
    sts: &[Vec<TokenTree>],
    open: Vec<(usize, Path)>,
) -> Result<Vec<(usize, Path)>, TokenStream> {
    let mut open = open;
    for st in sts {
        if is_wait(st) {
            let n = plan.states.len();
            plan.states.push(State {
                wait: st.clone(),
                items: Vec::new(),
                branches: Vec::new(),
                next: None,
            });
            plan.order.push(Piece::State(n));
            for (s, p) in &open {
                if fallthrough(p) {
                    plan.states[*s].next = Some(n);
                } else {
                    plan.states[*s].branches.push((p.clone(), n));
                }
            }
            open = vec![(n, Vec::new())];
            continue;
        }
        if open.is_empty() {
            return Err(err(
                st[0].span(),
                "a loop must start by waiting for an edge",
            ));
        }
        if let Some(arms) = if_parts(st) {
            if waits_in(std::slice::from_ref(st)) > 0 {
                let mut after: Vec<(usize, Path)> = Vec::new();
                let mut nots: Path = Vec::new();
                let mut has_else = false;
                for (cond, body) in arms {
                    let mut p = nots.clone();
                    match &cond {
                        Some(c) => {
                            p.push((c.clone(), false));
                            nots.push((c.clone(), true));
                        }
                        None => has_else = true,
                    }
                    let from: Vec<(usize, Path)> = open
                        .iter()
                        .map(|(s, q)| {
                            let mut q = q.clone();
                            q.extend(p.iter().cloned());
                            (*s, q)
                        })
                        .collect();
                    after.extend(walk(plan, &stmts_of(&body), from)?);
                }
                if !has_else {
                    for (s, q) in &open {
                        let mut q = q.clone();
                        q.extend(nots.iter().cloned());
                        after.push((*s, q));
                    }
                }
                open = after;
                continue;
            }
        }
        if let [(s, p)] = open.as_slice() {
            plan.states[*s].items.push((p.clone(), st.clone()));
            continue;
        }
        // Shared: the tail the last piece made, if it is for these
        // states, or a new one.
        let same = match plan.order.last() {
            Some(Piece::Tail(t)) => {
                same_members(&plan.tails[*t].members, &open)
            }
            _ => false,
        };
        if !same {
            plan.order.push(Piece::Tail(plan.tails.len()));
            plan.tails.push(Tail {
                members: open.clone(),
                items: Vec::new(),
            });
        }
        plan.tails.last_mut().unwrap().items.push(st.clone());
    }
    Ok(open)
}

/// What the lowering of the pieces gathers as it goes: the
/// combinational drives (the net, the piece, its condition, the
/// expression), the output ports held in a register, and the names
/// bound by earlier pieces' `let`s, which a later one may not read.
struct Gathered<'a> {
    comb: Vec<(String, usize, String, String)>,
    held: Vec<String>,
    earlier: Vec<String>,
    pnames: &'a [String],
}

impl Gathered<'_> {
    /// Whether a name is an output port of the unit.
    fn is_output(&self, net: &str) -> bool {
        self.pnames.iter().any(|p| p == net)
    }
    /// Whether a name is a wire of the netlist a port owns: an output,
    /// or a channel's data, valid or ready.
    fn is_port_net(&self, net: &str) -> bool {
        self.is_output(net)
            || ["_data", "_valid", "_ready"].iter().any(|end| {
                net.strip_suffix(end)
                    .is_some_and(|p| self.pnames.iter().any(|q| q == p))
            })
    }
    /// A lowered statement sorted into the piece's arm or the wires: an
    /// output goes to its held register and to the wires, a channel's
    /// net to the wires, anything else to the arm.
    fn sort(
        &mut self,
        s: String,
        piece: usize,
        cond: &str,
        seq: &mut Vec<String>,
    ) {
        match driven(&s) {
            Some(net) if self.is_output(net) => {
                let e = drive_of(&s, net).unwrap_or("").to_string();
                seq.push(format!(
                    "NlS::Drive(NlT::Name(\"{net}_held\".to_string()), {e})"
                ));
                if !self.held.contains(&net.to_string()) {
                    self.held.push(net.to_string());
                }
                self.comb
                    .push((net.to_string(), piece, cond.to_string(), e));
            }
            Some(net) if self.is_port_net(net) => {
                let e = drive_of(&s, net).unwrap_or("").to_string();
                self.comb
                    .push((net.to_string(), piece, cond.to_string(), e));
            }
            _ => seq.push(s),
        }
    }
}

/// The statements of a piece, each lowered as it would be after a
/// single wait, then sorted into the piece's arm or the wires. Those
/// under a path are lowered as one `if` on it, consecutive ones
/// together, so a `let` among them is read by the next.
fn lower_items(
    cx: &mut Cx,
    g: &mut Gathered,
    items: &[(Path, Vec<TokenTree>)],
    piece: usize,
    cond: &str,
    seq: &mut Vec<String>,
) -> Result<(), TokenStream> {
    let mut i = 0;
    while i < items.len() {
        let path = &items[i].0;
        let mut j = i;
        while j < items.len() && same_path(&items[j].0, path) {
            j += 1;
        }
        let group: Vec<Vec<TokenTree>> =
            items[i..j].iter().map(|(_, st)| st.clone()).collect();
        i = j;
        for st in &group {
            let fresh = bound_by(st);
            g.earlier.retain(|e| !fresh.contains(e));
            let from = if fresh.is_empty() { 0 } else { 2 };
            if let Some((name, sp)) = first_use(&st[from..], &g.earlier) {
                return Err(err(
                    sp,
                    &format!(
                        "`{name}` is bound before an earlier wait, and a \
                         `let` is a wire that does not hold: a value that \
                         crosses a wait is kept in a register"
                    ),
                ));
            }
        }
        let mut out = if path.is_empty() {
            let mut out = Vec::new();
            for st in &group {
                out.extend(lower_stmts(cx, st, None)?);
                out.append(&mut cx.hoisted);
            }
            out
        } else {
            let first = group[0][0].span();
            let mut kw = Ident::new("if", first);
            kw.set_span(first);
            let mut st: Vec<TokenTree> = vec![TokenTree::Ident(kw)];
            st.extend(conj(path));
            let mut body = Group::new(Delimiter::Brace, joined(&group));
            body.set_span(first);
            st.push(TokenTree::Group(body));
            let mut out = lower_stmts(cx, &st, None)?;
            out.append(&mut cx.hoisted);
            out
        };
        for s in out.drain(..) {
            g.sort(s, piece, cond, seq);
        }
    }
    Ok(())
}

/// The statements of a loop body of several waits, as the Rust source
/// of the netlist's statements, the width of the hidden register
/// called `reg`, and the registers the outputs are held in. `sts` are
/// the body's statements, the first a wait; `pnames` the unit's
/// ports; `span` where the loop is, for what is refused about the
/// whole of it.
pub(crate) fn lower(
    cx: &mut Cx,
    sts: &[Vec<TokenTree>],
    reg: &str,
    pnames: &[String],
    span: Span,
) -> Result<(Vec<String>, usize, Vec<(String, String)>), TokenStream> {
    // A `for` that waits is unrolled first; then the states, one per
    // wait, and what follows the last wait goes back to the first.
    let sts = unrolled(sts)?;
    let mut plan = Plan {
        states: Vec::new(),
        tails: Vec::new(),
        order: Vec::new(),
    };
    let open = walk(&mut plan, &sts, Vec::new())?;
    for (s, p) in open {
        if fallthrough(&p) {
            plan.states[s].next = Some(0);
        } else {
            plan.states[s].branches.push((p, 0));
        }
    }
    let n = plan.states.len();
    let width =
        std::cmp::max(1, usize::BITS - (n - 1).leading_zeros()) as usize;
    let at = |k: usize| ebin("==", &ename(reg), &bits(width, k));
    let mut g = Gathered {
        comb: Vec::new(),
        held: Vec::new(),
        earlier: Vec::new(),
        pnames,
    };
    let mut arms: Vec<String> = Vec::new();
    // Each state's condition, for the tails that share it.
    let mut conds: Vec<String> = vec![String::new(); n];
    let mut clock: Option<(String, bool)> = None;
    for (piece, what) in plan.order.iter().enumerate() {
        let start = cx.subst.len();
        cx.hoisted.clear();
        let mut seq: Vec<String> = Vec::new();
        let cond = match what {
            Piece::State(k) => {
                let k = *k;
                let state = &plan.states[k];
                cx.guard = None;
                cx.clock.clear();
                cx.falling = false;
                // The wait: its clock, its edge, and its condition,
                // which joins the state's.
                let head = lower_stmts(cx, &state.wait, None)?;
                let mut cond = at(k);
                let mut rest: Vec<String> = Vec::new();
                for s in head {
                    if let Some(gd) = s.strip_prefix("NlS::Guard(") {
                        let gd = gd.strip_suffix(')').unwrap_or(gd);
                        cond = ebin("&&", &cond, gd);
                    } else {
                        rest.push(s);
                    }
                }
                for s in rest {
                    g.sort(s, piece, &cond, &mut seq);
                }
                // A wait on a channel names no clock and takes the
                // process's; the waits that name one name the same.
                match &clock {
                    _ if cx.clock.is_empty() => {}
                    None => clock = Some((cx.clock.clone(), cx.falling)),
                    Some((c, f)) if *c != cx.clock || *f != cx.falling => {
                        return Err(err(
                            state.wait[0].span(),
                            "a process of several waits waits on one \
                             clock, at one edge",
                        ));
                    }
                    Some(_) => {}
                }
                conds[k] = cond.clone();
                cx.guard = Some(cond.clone());
                lower_items(cx, &mut g, &state.items, piece, &cond, &mut seq)?;
                // Where the state goes: the state after it, unless a
                // branch taken from here holds; the branches in the
                // order written, the first that holds winning.
                let mut next = bits(width, state.next.unwrap_or(0));
                for (path, to) in state.branches.iter().rev() {
                    let c = match tr(&conj(path), &cx.subst) {
                        Ok(c) => c,
                        Err(m) => return Err(err(state.wait[0].span(), &m)),
                    };
                    next = format!(
                        "NlE::Cond(Box::new({c}), Box::new({}), \
                         Box::new({next}))",
                        bits(width, *to)
                    );
                }
                seq.push(format!(
                    "NlS::Drive(NlT::Name(\"{reg}\".to_string()), {next})"
                ));
                cond
            }
            Piece::Tail(t) => {
                // A tail happens in any of its members: the union of
                // their conditions, each with the path it is under.
                let tail = &plan.tails[*t];
                let mut cond: Option<String> = None;
                for (s, path) in &tail.members {
                    let mut c = conds[*s].clone();
                    if !path.is_empty() {
                        let p = match tr(&conj(path), &cx.subst) {
                            Ok(p) => p,
                            Err(m) => return Err(err(span, &m)),
                        };
                        c = ebin("&&", &c, &p);
                    }
                    cond = Some(match cond {
                        None => c,
                        Some(a) => ebin("||", &a, &c),
                    });
                }
                let cond = cond.unwrap_or_default();
                cx.guard = Some(cond.clone());
                let items: Vec<(Path, Vec<TokenTree>)> = tail
                    .items
                    .iter()
                    .map(|st| (Vec::new(), st.clone()))
                    .collect();
                lower_items(cx, &mut g, &items, piece, &cond, &mut seq)?;
                cond
            }
        };
        for (name, _) in cx.subst.drain(start..) {
            if !g.earlier.contains(&name) {
                g.earlier.push(name);
            }
        }
        if !seq.is_empty() {
            arms.push(format!(
                "NlS::If(vec![({cond}, vec![{}])], vec![])",
                seq.join(",\n")
            ));
        }
    }
    let Gathered { comb, held, .. } = g;
    let is_output = |net: &str| pnames.iter().any(|p| p == net);
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
        } else if is_output(&net) {
            // An output is what the state about to leave sets, while
            // its wait's condition holds, and otherwise what it was
            // last set to, which its register holds. That is the run's
            // wire seen a cycle ahead, as the netlist's wires are: the
            // run sets it at the edge from what stood before the edge.
            let mut sorted: Vec<&&(String, usize, String, String)> =
                mine.iter().collect();
            sorted.sort_by_key(|(_, k, _, _)| *k);
            let mut acc = ename(&format!("{net}_held"));
            for (_, _, cond, e) in sorted.iter().rev() {
                acc = format!(
                    "NlE::Cond(Box::new({cond}), Box::new({e}), \
                     Box::new({acc}))"
                );
            }
            acc
        } else {
            // A channel sent from several states offers the state's
            // data, and where the data is the same in every state it
            // is one expression rather than a choice among copies.
            let mut sorted: Vec<&&(String, usize, String, String)> =
                mine.iter().collect();
            sorted.sort_by_key(|(_, k, _, _)| *k);
            let (_, _, _, last) = sorted[sorted.len() - 1];
            let mut acc = last.clone();
            if sorted.iter().any(|(_, _, _, e)| e != last) {
                for (_, _, cond, e) in sorted.iter().rev().skip(1) {
                    acc = format!(
                        "NlE::Cond(Box::new({cond}), Box::new({e}), \
                         Box::new({acc}))"
                    );
                }
            }
            acc
        };
        stmts
            .push(format!("NlS::Drive(NlT::Name(\"{net}\".to_string()), {e})"));
    }
    // A held output's register is as wide as the port, which the
    // generated code asks the value's type for.
    let mut regs: Vec<(String, String)> = Vec::new();
    for net in &held {
        let ty = super::PTYPES.with(|p| {
            p.borrow()
                .iter()
                .find(|(n, _)| n == net)
                .map(|(_, t)| t.clone())
        });
        let Some(ty) = ty else {
            return Err(err(
                span,
                &format!("the type of the output `{net}` is not known"),
            ));
        };
        regs.push((
            format!("{net}_held"),
            format!("<{ty} as ::txhdl::types::Value>::WIDTH"),
        ));
    }
    stmts.extend(arms);
    Ok((stmts, width, regs))
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

/// How many waits the statements hold, those inside a `for` and under
/// `if` counted with the rest.
pub(crate) fn waits_in(sts: &[Vec<TokenTree>]) -> usize {
    sts.iter()
        .map(|st| {
            if is_wait(st) {
                return 1;
            }
            if let Some((_, _, body)) = for_parts(st) {
                return waits_in(&stmts_of(&body));
            }
            match if_parts(st) {
                Some(arms) => {
                    arms.iter().map(|(_, body)| waits_in(&stmts_of(body))).sum()
                }
                None => 0,
            }
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
        // An `if` whose arms hold a `for` that waits: the arms are
        // unrolled in place, the chain kept.
        if if_parts(st).is_some() && waits_in(std::slice::from_ref(st)) > 0 {
            let mut rebuilt: Vec<TokenTree> = Vec::new();
            for t in st {
                rebuilt.push(match t {
                    TokenTree::Group(g)
                        if g.delimiter() == Delimiter::Brace =>
                    {
                        let inner: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let body = unrolled(&stmts_of(&inner))?;
                        let mut n = Group::new(Delimiter::Brace, joined(&body));
                        n.set_span(g.span());
                        TokenTree::Group(n)
                    }
                    t => t.clone(),
                });
            }
            out.push(rebuilt);
            continue;
        }
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

/// Whether two paths are the same conditions, token for token.
fn same_path(a: &Path, b: &Path) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((x, f), (y, g))| f == g && text(x) == text(y))
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
