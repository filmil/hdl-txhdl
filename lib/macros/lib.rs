// SPDX-License-Identifier: Apache-2.0
//! The macros of the runtime: two derives and `interface!`. Written
//! against `proc_macro` alone, without syn or quote, because the
//! grammars are small and a crate registry would be the larger cost.
extern crate proc_macro;
use proc_macro::{Delimiter, Group, Ident, Span, TokenStream, TokenTree};

fn type_name(input: TokenStream) -> String {
    let mut seen_kw = false;
    for tt in input {
        if let TokenTree::Ident(id) = tt {
            let s = id.to_string();
            if seen_kw {
                return s;
            }
            if s == "struct" || s == "enum" || s == "union" {
                seen_kw = true;
            }
        }
    }
    panic!("expected a struct, enum or union")
}

fn marker(input: TokenStream, trait_path: &str) -> TokenStream {
    let name = type_name(input);
    format!("impl {trait_path} for {name} {{}}").parse().unwrap()
}

#[proc_macro_derive(Transaction)]
pub fn derive_transaction(input: TokenStream) -> TokenStream {
    marker(input, "::txhdl::types::Transaction")
}

#[proc_macro_derive(Bus)]
pub fn derive_bus(input: TokenStream) -> TokenStream {
    marker(input, "::txhdl::comp::Bus")
}

// ---------------------------------------------------------------------
// interface!

struct Field { name: String, ty: String }
struct Role { name: String, ends: Vec<(String, String)> } // (dir, member)

fn err(span: Span, msg: &str) -> TokenStream {
    let msg = msg.replace('"', "\\\"");
    let mut ts: TokenStream = format!("compile_error!(\"{msg}\");").parse().unwrap();
    // Attach the span so the error points at the declaration.
    ts = ts.into_iter().map(|mut tt| { tt.set_span(span); tt }).collect();
    ts
}

fn expect_ident(it: &mut impl Iterator<Item = TokenTree>, what: &str) -> Result<Ident, TokenStream> {
    match it.next() {
        Some(TokenTree::Ident(i)) => Ok(i),
        Some(other) => Err(err(other.span(), &format!("expected {what}"))),
        None => Err(err(Span::call_site(), &format!("expected {what}, found end of input"))),
    }
}

fn expect_brace(it: &mut impl Iterator<Item = TokenTree>, what: &str) -> Result<Group, TokenStream> {
    match it.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => Ok(g),
        Some(other) => Err(err(other.span(), &format!("expected {{ ... }} for {what}"))),
        None => Err(err(Span::call_site(), &format!("expected {{ ... }} for {what}, found end of input"))),
    }
}

/// `name: Type, name: Type, ...`. A type may contain `<` and `>`, and a
/// comma inside them does not end the member, so angle depth is tracked.
fn parse_members(g: Group) -> Result<Vec<Field>, TokenStream> {
    let mut out = Vec::new();
    let mut it = g.stream().into_iter().peekable();
    while it.peek().is_some() {
        let name = expect_ident(&mut it, "a member name")?;
        match it.next() {
            Some(TokenTree::Punct(p)) if p.as_char() == ':' => {}
            Some(other) => return Err(err(other.span(), "expected `:` after member name")),
            None => return Err(err(name.span(), "expected `:` after member name")),
        }
        let mut ty = String::new();
        let mut depth = 0i32;
        loop {
            match it.peek() {
                None => break,
                Some(TokenTree::Punct(p)) if p.as_char() == ',' && depth == 0 => { it.next(); break; }
                Some(TokenTree::Punct(p)) if p.as_char() == '<' => { depth += 1; }
                Some(TokenTree::Punct(p)) if p.as_char() == '>' => { depth -= 1; }
                _ => {}
            }
            ty.push_str(&it.next().unwrap().to_string());
        }
        if ty.is_empty() {
            return Err(err(name.span(), "member has no type"));
        }
        out.push(Field { name: name.to_string(), ty });
    }
    Ok(out)
}

/// `dir member, dir member, ...` where dir is `in` or `out`.
fn parse_role(name: Ident, g: Group) -> Result<Role, TokenStream> {
    let mut ends = Vec::new();
    let mut it = g.stream().into_iter().peekable();
    while it.peek().is_some() {
        let dir = expect_ident(&mut it, "`in` or `out`")?;
        let d = dir.to_string();
        if d != "in" && d != "out" {
            return Err(err(dir.span(), &format!("direction must be `in` or `out`, found `{d}`")));
        }
        let m = expect_ident(&mut it, "a member name after the direction")?;
        ends.push((d, m.to_string()));
        if let Some(TokenTree::Punct(p)) = it.peek() {
            if p.as_char() == ',' { it.next(); }
        }
    }
    Ok(Role { name: name.to_string(), ends })
}

fn check(iface: &Ident, members: &[Field], roles: &[Role]) -> Result<(), TokenStream> {
    if roles.is_empty() {
        return Err(err(iface.span(), "an interface needs at least one role"));
    }
    for r in roles {
        for m in members {
            let n = r.ends.iter().filter(|(_, x)| x == &m.name).count();
            if n == 0 {
                return Err(err(iface.span(), &format!(
                    "role `{}` does not say which end of `{}` it takes", r.name, m.name)));
            }
            if n > 1 {
                return Err(err(iface.span(), &format!(
                    "role `{}` names `{}` {} times", r.name, m.name, n)));
            }
        }
        for (_, x) in &r.ends {
            if !members.iter().any(|m| &m.name == x) {
                return Err(err(iface.span(), &format!(
                    "role `{}` names `{}`, which is not a member of `{}`", r.name, x, iface)));
            }
        }
    }
    // One driver per member, across every role. The type system would
    // catch this too, as a move error on the second `Driver`, but this
    // message names both roles and the member; E0382 names a temporary.
    for m in members {
        let drivers: Vec<&str> = roles.iter()
            .filter(|r| r.ends.iter().any(|(d, x)| d == "out" && x == &m.name))
            .map(|r| r.name.as_str())
            .collect();
        if drivers.len() > 1 {
            return Err(err(iface.span(), &format!(
                "member `{}` is driven by more than one role: {}", m.name, drivers.join(", "))));
        }
    }
    Ok(())
}

fn emit(iface: &Ident, members: &[Field], roles: &[Role]) -> TokenStream {
    let mut s = String::new();
    s.push_str(&format!("pub struct {iface};\n"));

    for r in roles {
        s.push_str(&format!("pub struct {} {{\n", r.name));
        for (dir, mname) in &r.ends {
            let ty = &members.iter().find(|m| &m.name == mname).unwrap().ty;
            let end = if dir == "out" { "Driver" } else { "Reader" };
            s.push_str(&format!("    pub {mname}: <{ty} as ::txhdl::comp::Member>::{end},\n"));
        }
        s.push_str("}\n");
    }

    // The constructor: split every member once, then hand each role the
    // end it asked for. A reader is cloned, because readers fan out and
    // `Reader` is Clone for that reason. A driver is moved, so a second
    // role driving the same member is a use of a moved value even if the
    // check above were removed.
    let tuple: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    s.push_str(&format!("impl {iface} {{\n    pub fn new() -> ({}) {{\n", tuple.join(", ")));
    for m in members {
        // Fully qualified call syntax, so the caller need not have `Member`
        // in scope for the generated code to resolve.
        s.push_str(&format!(
            "        let {} = ::txhdl::comp::Member::split(<{} as ::txhdl::comp::Member>::new());\n",
            m.name, m.ty));
    }
    s.push_str("        (\n");
    for r in roles {
        s.push_str(&format!("            {} {{ ", r.name));
        for (dir, mname) in &r.ends {
            if dir == "out" {
                s.push_str(&format!("{mname}: {mname}.0, "));
            } else {
                s.push_str(&format!("{mname}: {mname}.1.clone(), "));
            }
        }
        s.push_str("},\n");
    }
    s.push_str("        )\n    }\n}\n");
    s.parse().unwrap()
}

#[proc_macro]
pub fn interface(input: TokenStream) -> TokenStream {
    let mut it = input.into_iter();
    let iface = match expect_ident(&mut it, "an interface name") { Ok(i) => i, Err(e) => return e };
    let body = match expect_brace(&mut it, "the interface members") { Ok(g) => g, Err(e) => return e };
    let members = match parse_members(body) { Ok(m) => m, Err(e) => return e };

    let mut roles = Vec::new();
    loop {
        match it.next() {
            None => break,
            Some(TokenTree::Ident(kw)) if kw.to_string() == "role" => {
                let name = match expect_ident(&mut it, "a role name") { Ok(i) => i, Err(e) => return e };
                let g = match expect_brace(&mut it, "the role's ends") { Ok(g) => g, Err(e) => return e };
                match parse_role(name, g) { Ok(r) => roles.push(r), Err(e) => return e }
            }
            Some(other) => return err(other.span(), "expected `role`"),
        }
    }

    if let Err(e) = check(&iface, &members, &roles) { return e; }
    emit(&iface, &members, &roles)
}


// ---------------------------------------------------------------------
// when!

/// Split a token stream on the first `<=` at depth zero. `<=` arrives as
/// `<` with joint spacing followed by `=`.
fn split_becomes(ts: TokenStream) -> Option<(TokenStream, TokenStream)> {
    let toks: Vec<TokenTree> = ts.into_iter().collect();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            TokenTree::Punct(p) if p.as_char() == '<' && p.spacing() == proc_macro::Spacing::Joint => {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '=' && depth == 0 {
                        let lhs: TokenStream = toks[..i].iter().cloned().collect();
                        let rhs: TokenStream = toks[i + 2..].iter().cloned().collect();
                        return Some((lhs, rhs));
                    }
                }
            }
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a brace group's contents on `;` at depth zero.
fn statements(g: &Group) -> Vec<TokenStream> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for tt in g.stream() {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ';' => {
                if !cur.is_empty() { out.push(cur.drain(..).collect()) }
            }
            _ => cur.push(tt),
        }
    }
    if !cur.is_empty() { out.push(cur.into_iter().collect()) }
    out
}

/// `when!(cond => { lhs <= rhs; ... } else { lhs <= rhs; ... })`
///
/// Predicated register drives that read as a conditional. Both arms exist
/// in the hardware at once and the condition selects, which is why the
/// name is `when` and not `if`. The arrow points into the register: the
/// register becomes the value.
///
/// Procedural rather than `macro_rules!` because an `expr` fragment may
/// be followed only by `=>`, `,` or `;`, so `lhs <= rhs` cannot be
/// matched declaratively at all.
#[proc_macro]
pub fn when(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = input.into_iter().collect();

    // The condition runs up to `=>`.
    let mut i = 0;
    let mut cond = Vec::new();
    loop {
        match toks.get(i) {
            None => return err(Span::call_site(), "expected `=>` after the condition"),
            Some(TokenTree::Punct(p)) if p.as_char() == '=' && p.spacing() == proc_macro::Spacing::Joint => {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '>' { i += 2; break }
                }
                cond.push(toks[i].clone()); i += 1;
            }
            Some(t) => { cond.push(t.clone()); i += 1 }
        }
    }
    let cond: TokenStream = cond.into_iter().collect();

    let then = match toks.get(i) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => { i += 1; g.clone() }
        _ => return err(Span::call_site(), "expected `{ ... }` after `=>`"),
    };
    let otherwise = match (toks.get(i), toks.get(i + 1)) {
        (Some(TokenTree::Ident(kw)), Some(TokenTree::Group(g)))
            if kw.to_string() == "else" && g.delimiter() == Delimiter::Brace => Some(g.clone()),
        (None, _) => None,
        (Some(t), _) => return err(t.span(), "expected `else { ... }` or the end"),
    };

    let mut out = String::from("{ let __c: ::txhdl::types::Bit = ");
    out.push_str(&cond.to_string());
    out.push_str(";\n");
    for (arm, pred) in [(Some(&then), "__c"), (otherwise.as_ref(), "__c.not()")] {
        let Some(g) = arm else { continue };
        for st in statements(g) {
            let span = st.clone().into_iter().next().map(|t| t.span()).unwrap_or(g.span());
            let Some((lhs, rhs)) = split_becomes(st) else {
                return err(span, "expected `register <= value`");
            };
            out.push_str(&format!("({}).set_if({pred}, {});\n", lhs, rhs));
        }
    }
    out.push('}');
    out.parse().unwrap()
}


// ---------------------------------------------------------------------
// case!

/// `case!(value => { pattern => { lhs <= rhs; ... }, ... })`
///
/// `when!` with many arms. Each arm is a Rust pattern, guards and `_`
/// included, and the first arm that matches wins, which is `match`'s
/// rule. Every arm exists in the hardware at once, the same as
/// `when!`; the ordering lowers to a priority chain of predicated
/// drives, and the predicate of each arm is its pattern and the failure
/// of every arm above it.
///
/// The scrutinee is a value, so a register is read first and the value
/// is what is matched. Arm bodies take the same `lhs <= rhs` statements
/// as `when!`, and nothing else.
#[proc_macro]
pub fn case(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = input.into_iter().collect();

    // The value runs up to `=>`.
    let (value, i) = match up_to_arrow(&toks, 0) {
        Some(x) => x,
        None => return err(Span::call_site(), "expected `=>` after the value"),
    };
    let arms = match toks.get(i) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace && toks.len() == i + 1 => g.clone(),
        _ => return err(Span::call_site(), "expected `{ pattern => { ... }, ... }` after `=>`"),
    };

    let mut out = String::from("{ let __s = ");
    out.push_str(&value.to_string());
    out.push_str(";\nlet mut __done = false;\n");

    // Arms: `pattern => { ... }` separated by `,`.
    let atoks: Vec<TokenTree> = arms.stream().into_iter().collect();
    let mut j = 0;
    let mut n = 0;
    while j < atoks.len() {
        let (pat, k) = match up_to_arrow(&atoks, j) {
            Some(x) => x,
            None => return err(atoks[j].span(), "expected `pattern => { ... }`"),
        };
        let body = match atoks.get(k) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g.clone(),
            Some(t) => return err(t.span(), "expected `{ ... }` after `=>`"),
            None => return err(arms.span(), "expected `{ ... }` after `=>`"),
        };
        j = k + 1;
        if let Some(TokenTree::Punct(p)) = atoks.get(j) {
            if p.as_char() == ',' { j += 1 }
        }
        n += 1;
        out.push_str(&format!(
            "{{ let __c: ::txhdl::types::Bit = ::txhdl::types::Bit::from_bool(!__done && matches!(__s, {}));\n",
            pat
        ));
        for st in statements(&body) {
            let span = st.clone().into_iter().next().map(|t| t.span()).unwrap_or(body.span());
            let Some((lhs, rhs)) = split_becomes(st) else {
                return err(span, "expected `register <= value`");
            };
            out.push_str(&format!("({}).set_if(__c, {});\n", lhs, rhs));
        }
        out.push_str("__done = __done || __c.to_bool(); }\n");
    }
    if n == 0 { return err(arms.span(), "expected at least one arm") }
    out.push_str("let _ = __done; }");
    out.parse().unwrap()
}

/// Tokens from `from` up to the first `=>`, and the index after it.
fn up_to_arrow(toks: &[TokenTree], from: usize) -> Option<(TokenStream, usize)> {
    let mut i = from;
    let mut acc = Vec::new();
    while i < toks.len() {
        if let TokenTree::Punct(p) = &toks[i] {
            if p.as_char() == '=' && p.spacing() == proc_macro::Spacing::Joint {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '>' { return Some((acc.into_iter().collect(), i + 2)) }
                }
            }
        }
        acc.push(toks[i].clone());
        i += 1;
    }
    None
}
