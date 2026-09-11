// The `interface!` procedural macro.
//
// What it does that the macro_rules! version could not:
//   - any number of roles, not exactly two;
//   - `in` as the direction keyword, because a proc macro sees raw
//     tokens and a keyword is just an identifier to it;
//   - a check that every role names every member exactly once, with a
//     compile error naming the member and the role when it does not.
//
// Written against `proc_macro` alone, without syn or quote, so it needs
// no crate registry. The grammar is small enough to parse by hand:
//
//   interface! {
//       Name { member: Type, ... }
//       role RoleA { dir member, ... }
//       role RoleB { dir member, ... }
//       ...
//   }

extern crate proc_macro;
use proc_macro::{Delimiter, Group, Ident, Punct, Span, TokenStream, TokenTree};

struct Member { name: String, ty: String }
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
fn parse_members(g: Group) -> Result<Vec<Member>, TokenStream> {
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
        out.push(Member { name: name.to_string(), ty });
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

fn check(iface: &Ident, members: &[Member], roles: &[Role]) -> Result<(), TokenStream> {
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

fn emit(iface: &Ident, members: &[Member], roles: &[Role]) -> TokenStream {
    let mut s = String::new();
    s.push_str(&format!("pub struct {iface};\n"));

    for r in roles {
        s.push_str(&format!("pub struct {} {{\n", r.name));
        for (dir, mname) in &r.ends {
            let ty = &members.iter().find(|m| &m.name == mname).unwrap().ty;
            let end = if dir == "out" { "Driver" } else { "Reader" };
            s.push_str(&format!("    pub {mname}: <{ty} as Member>::{end},\n"));
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
        s.push_str(&format!("        let {} = <{} as Member>::new().split();\n", m.name, m.ty));
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

// Silence unused-import warnings for items only some paths use.
#[allow(dead_code)]
fn _unused(_: Punct) {}
