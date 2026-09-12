// SPDX-License-Identifier: Apache-2.0
//! The macros of the runtime: four derives, `interface!`, `when!` and
//! `case!`. Written
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
    format!("impl {trait_path} for {name} {{}}")
        .parse()
        .unwrap()
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
// derive(Value), derive(Trace)

/// A struct or enum item, as far as the derives need it: name, generic
/// parameters with and without their bounds, and the body.
struct Item {
    kind: String,
    name: String,
    bounds: String,
    args: String,
    body: Option<Group>,
}

fn parse_item(input: TokenStream) -> Item {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let mut i = 0;
    while i < toks.len() {
        if let TokenTree::Ident(id) = &toks[i] {
            let s = id.to_string();
            if s == "struct" || s == "enum" {
                break;
            }
        }
        i += 1;
    }
    let kind = toks[i].to_string();
    let name = toks[i + 1].to_string();
    i += 2;
    let (mut bounds, mut args) = (String::new(), String::new());
    if let Some(TokenTree::Punct(p)) = toks.get(i) {
        if p.as_char() == '<' {
            // Collect the generic parameters up to the matching `>`.
            let mut depth = 0;
            let mut params: Vec<Vec<TokenTree>> = vec![Vec::new()];
            loop {
                let t = &toks[i];
                i += 1;
                match t {
                    TokenTree::Punct(p) if p.as_char() == '<' => {
                        depth += 1;
                        if depth > 1 {
                            params.last_mut().unwrap().push(t.clone())
                        }
                    }
                    TokenTree::Punct(p) if p.as_char() == '>' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        params.last_mut().unwrap().push(t.clone())
                    }
                    TokenTree::Punct(p) if p.as_char() == ',' && depth == 1 => {
                        params.push(Vec::new())
                    }
                    _ => params.last_mut().unwrap().push(t.clone()),
                }
            }
            let mut names = Vec::new();
            let mut full = Vec::new();
            for p in params.into_iter().filter(|p| !p.is_empty()) {
                let text: String = p
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                full.push(text);
                let name = match &p[0] {
                    TokenTree::Ident(id) if id.to_string() == "const" => {
                        p[1].to_string()
                    }
                    TokenTree::Punct(q) if q.as_char() == '\'' => {
                        format!("'{}", p[1])
                    }
                    t => t.to_string(),
                };
                names.push(name);
            }
            bounds = format!("<{}>", full.join(", "));
            args = format!("<{}>", names.join(", "));
        }
    }
    let body = toks[i..].iter().find_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            Some(g.clone())
        }
        _ => None,
    });
    Item {
        kind,
        name,
        bounds,
        args,
        body,
    }
}

/// The field names of a braced struct body, in order.
fn field_names(body: &Group) -> Vec<String> {
    let toks: Vec<TokenTree> = body.stream().into_iter().collect();
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            TokenTree::Ident(id) if depth == 0 => {
                if let Some(TokenTree::Punct(p)) = toks.get(i + 1) {
                    if p.as_char() == ':' {
                        let next_is_path = matches!(
                            toks.get(i + 2),
                            Some(TokenTree::Punct(q)) if q.as_char() == ':'
                        );
                        if !next_is_path {
                            names.push(id.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    names
}

/// The field types of a braced struct body, as text, in order.
fn field_types(body: &Group) -> Vec<String> {
    let toks: Vec<TokenTree> = body.stream().into_iter().collect();
    let mut types = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            TokenTree::Punct(p) if p.as_char() == ':' && depth == 0 => {
                let path = matches!(
                    toks.get(i + 1),
                    Some(TokenTree::Punct(q)) if q.as_char() == ':'
                );
                if !path
                    && !matches!(toks.get(i - 1), Some(TokenTree::Punct(_)))
                {
                    // From here to the next `,` at depth 0 is the type.
                    let mut j = i + 1;
                    let mut d = 0i32;
                    let mut ty = Vec::new();
                    while j < toks.len() {
                        match &toks[j] {
                            TokenTree::Punct(p) if p.as_char() == '<' => d += 1,
                            TokenTree::Punct(p) if p.as_char() == '>' => d -= 1,
                            TokenTree::Punct(p)
                                if p.as_char() == ',' && d == 0 =>
                            {
                                break
                            }
                            _ => {}
                        }
                        ty.push(toks[j].to_string());
                        j += 1;
                    }
                    types.push(ty.join(" "));
                    i = j;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    types
}

/// The variant names of a fieldless enum.
fn variant_names(body: &Group) -> Vec<String> {
    let mut names = Vec::new();
    let mut skip = false;
    for t in body.stream() {
        match t {
            TokenTree::Punct(p) if p.as_char() == '#' => skip = true,
            TokenTree::Group(_) if skip => skip = false,
            TokenTree::Ident(id) => names.push(id.to_string()),
            _ => {}
        }
    }
    names
}

/// `#[derive(Value)]`: a struct of values is their concatenation, most
/// significant field first; a fieldless enum is the index of its
/// variant, as wide as it needs to be.
#[proc_macro_derive(Value)]
pub fn derive_value(input: TokenStream) -> TokenStream {
    let item = parse_item(input);
    let Some(body) = &item.body else {
        return err(Span::call_site(), "Value needs a braced struct or enum");
    };
    let out = if item.kind == "struct" {
        let names = field_names(body);
        let types = field_types(body);
        let width = types
            .iter()
            .map(|t| format!("<{t} as ::txhdl::types::Value>::WIDTH"))
            .collect::<Vec<_>>()
            .join(" + ");
        let parts = names
            .iter()
            .map(|n| {
                format!("s.push_str(&::txhdl::types::Value::vcd(self.{n}));")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let fields = names
            .iter()
            .zip(&types)
            .map(|(n, t)| {
                format!(
                    "::txhdl::types::Part {{ name: \"{n}\", \
                     width: <{t} as ::txhdl::types::Value>::WIDTH, \
                     bits: ::txhdl::types::Value::vcd(self.{n}), \
                     names: <{t} as ::txhdl::types::Value>::names() }},"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "impl{b} ::txhdl::types::Value for {n}{a} {{\n\
             const WIDTH: usize = {width};\n\
             fn vcd(self) -> String {{\n\
             let mut s = String::new(); {parts} s }}\n\
             fn parts(self) -> Vec<::txhdl::types::Part> {{\n\
             vec![{fields}] }}\n}}",
            b = item.bounds,
            n = item.name,
            a = item.args
        )
    } else {
        let variants = variant_names(body);
        let n = variants.len().max(2);
        let width = (usize::BITS - (n - 1).leading_zeros()) as usize;
        let names = variants
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let arms = variants
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{name}::{v} => {i}usize,", name = item.name))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "impl{b} ::txhdl::types::Value for {n}{a} {{\n\
             const WIDTH: usize = {width};\n\
             fn vcd(self) -> String {{ let i = match self {{ {arms} }};\n\
             format!(\"{{:0w$b}}\", i, w = {width}) }}\n\
             fn names() -> Option<&'static [&'static str]> {{\n\
             Some(&[{names}]) }}\n}}",
            b = item.bounds,
            n = item.name,
            a = item.args
        )
    };
    out.parse().unwrap()
}

/// `#[derive(Trace)]`: every field is registered under its own name.
#[proc_macro_derive(Trace)]
pub fn derive_trace(input: TokenStream) -> TokenStream {
    let item = parse_item(input);
    let Some(body) = &item.body else {
        return err(Span::call_site(), "Trace needs a braced struct");
    };
    let calls = field_names(body)
        .iter()
        .map(|n| {
            format!(
                "::txhdl::comp::trace::Traceable::trace(\
                 &self.{n}, &scope.child(\"{n}\"));"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let names = field_names(body);
    let types = field_types(body);
    let fields = names
        .iter()
        .zip(&types)
        .map(|(n, t)| {
            format!(
                "(\"{n}\", <{t} as ::txhdl::netlist::Port>::KIND, \
                 <{t} as ::txhdl::netlist::Port>::WIDTH, \
                 <{t} as ::txhdl::netlist::Port>::DEPTH),"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "impl{b} ::txhdl::comp::trace::Traceable for {n}{a} {{\n\
         fn trace(&self, scope: &::txhdl::comp::trace::Scope) {{\n\
         {calls} }}\n}}\n\
         impl{b} ::txhdl::netlist::Fields for {n}{a} {{\n\
         fn fields() -> Vec<(&'static str, \
         Option<::txhdl::comp::trace::Kind>, usize, usize)> {{ \
         vec![{fields}] }}\n}}\n\
         impl{b} ::txhdl::netlist::Port for {n}{a} {{}}",
        b = item.bounds,
        n = item.name,
        a = item.args
    )
    .parse()
    .unwrap()
}

// ---------------------------------------------------------------------
// interface!

struct Field {
    name: String,
    ty: String,
}
struct Role {
    name: String,
    ends: Vec<(String, String)>,
} // (dir, member)

fn err(span: Span, msg: &str) -> TokenStream {
    let msg = msg.replace('"', "\\\"");
    let mut ts: TokenStream =
        format!("compile_error!(\"{msg}\");").parse().unwrap();
    // Attach the span so the error points at the declaration.
    ts = ts
        .into_iter()
        .map(|mut tt| {
            tt.set_span(span);
            tt
        })
        .collect();
    ts
}

fn expect_ident(
    it: &mut impl Iterator<Item = TokenTree>,
    what: &str,
) -> Result<Ident, TokenStream> {
    match it.next() {
        Some(TokenTree::Ident(i)) => Ok(i),
        Some(other) => Err(err(other.span(), &format!("expected {what}"))),
        None => Err(err(
            Span::call_site(),
            &format!("expected {what}, found end of input"),
        )),
    }
}

fn expect_brace(
    it: &mut impl Iterator<Item = TokenTree>,
    what: &str,
) -> Result<Group, TokenStream> {
    match it.next() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => Ok(g),
        Some(other) => {
            Err(err(other.span(), &format!("expected {{ ... }} for {what}")))
        }
        None => Err(err(
            Span::call_site(),
            &format!("expected {{ ... }} for {what}, found end of input"),
        )),
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
            Some(other) => {
                return Err(err(other.span(), "expected `:` after member name"))
            }
            None => {
                return Err(err(name.span(), "expected `:` after member name"))
            }
        }
        let mut ty = String::new();
        let mut depth = 0i32;
        loop {
            match it.peek() {
                None => break,
                Some(TokenTree::Punct(p))
                    if p.as_char() == ',' && depth == 0 =>
                {
                    it.next();
                    break;
                }
                Some(TokenTree::Punct(p)) if p.as_char() == '<' => {
                    depth += 1;
                }
                Some(TokenTree::Punct(p)) if p.as_char() == '>' => {
                    depth -= 1;
                }
                _ => {}
            }
            ty.push_str(&it.next().unwrap().to_string());
        }
        if ty.is_empty() {
            return Err(err(name.span(), "member has no type"));
        }
        out.push(Field {
            name: name.to_string(),
            ty,
        });
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
            return Err(err(
                dir.span(),
                &format!("direction must be `in` or `out`, found `{d}`"),
            ));
        }
        let m = expect_ident(&mut it, "a member name after the direction")?;
        ends.push((d, m.to_string()));
        if let Some(TokenTree::Punct(p)) = it.peek() {
            if p.as_char() == ',' {
                it.next();
            }
        }
    }
    Ok(Role {
        name: name.to_string(),
        ends,
    })
}

fn check(
    iface: &Ident,
    members: &[Field],
    roles: &[Role],
) -> Result<(), TokenStream> {
    if roles.is_empty() {
        return Err(err(iface.span(), "an interface needs at least one role"));
    }
    for r in roles {
        for m in members {
            let n = r.ends.iter().filter(|(_, x)| x == &m.name).count();
            if n == 0 {
                return Err(err(
                    iface.span(),
                    &format!(
                        "role `{}` does not say which end of `{}` it takes",
                        r.name, m.name
                    ),
                ));
            }
            if n > 1 {
                return Err(err(
                    iface.span(),
                    &format!(
                        "role `{}` names `{}` {} times",
                        r.name, m.name, n
                    ),
                ));
            }
        }
        for (_, x) in &r.ends {
            if !members.iter().any(|m| &m.name == x) {
                return Err(err(
                    iface.span(),
                    &format!(
                        "role `{}` names `{}`, which is not a member of `{}`",
                        r.name, x, iface
                    ),
                ));
            }
        }
    }
    // One driver per member, across every role. The type system would
    // catch this too, as a move error on the second `Driver`, but this
    // message names both roles and the member; E0382 names a temporary.
    for m in members {
        let drivers: Vec<&str> = roles
            .iter()
            .filter(|r| r.ends.iter().any(|(d, x)| d == "out" && x == &m.name))
            .map(|r| r.name.as_str())
            .collect();
        if drivers.len() > 1 {
            return Err(err(
                iface.span(),
                &format!(
                    "member `{}` is driven by more than one role: {}",
                    m.name,
                    drivers.join(", ")
                ),
            ));
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
            s.push_str(&format!(
                "    pub {mname}: <{ty} as ::txhdl::comp::Member>::{end},\n"
            ));
        }
        s.push_str("}\n");
    }

    // The constructor: split every member once, then hand each role the
    // end it asked for. A reader is cloned, because readers fan out and
    // `Reader` is Clone for that reason. A driver is moved, so a second
    // role driving the same member is a use of a moved value even if the
    // check above were removed.
    let tuple: Vec<&str> = roles.iter().map(|r| r.name.as_str()).collect();
    s.push_str(&format!(
        "impl {iface} {{\n    pub fn new() -> ({}) {{\n",
        tuple.join(", ")
    ));
    for m in members {
        // Fully qualified call syntax, so the caller need not have `Member`
        // in scope for the generated code to resolve.
        s.push_str(&format!(
            "        let {} = ::txhdl::comp::Member::split(\
             <{} as ::txhdl::comp::Member>::new());\n",
            m.name, m.ty
        ));
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
    let iface = match expect_ident(&mut it, "an interface name") {
        Ok(i) => i,
        Err(e) => return e,
    };
    let body = match expect_brace(&mut it, "the interface members") {
        Ok(g) => g,
        Err(e) => return e,
    };
    let members = match parse_members(body) {
        Ok(m) => m,
        Err(e) => return e,
    };

    let mut roles = Vec::new();
    loop {
        match it.next() {
            None => break,
            Some(TokenTree::Ident(kw)) if kw.to_string() == "role" => {
                let name = match expect_ident(&mut it, "a role name") {
                    Ok(i) => i,
                    Err(e) => return e,
                };
                let g = match expect_brace(&mut it, "the role's ends") {
                    Ok(g) => g,
                    Err(e) => return e,
                };
                match parse_role(name, g) {
                    Ok(r) => roles.push(r),
                    Err(e) => return e,
                }
            }
            Some(other) => return err(other.span(), "expected `role`"),
        }
    }

    if let Err(e) = check(&iface, &members, &roles) {
        return e;
    }
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
            TokenTree::Punct(p)
                if p.as_char() == '<'
                    && p.spacing() == proc_macro::Spacing::Joint =>
            {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '=' && depth == 0 {
                        let lhs: TokenStream =
                            toks[..i].iter().cloned().collect();
                        let rhs: TokenStream =
                            toks[i + 2..].iter().cloned().collect();
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
                if !cur.is_empty() {
                    out.push(cur.drain(..).collect())
                }
            }
            _ => cur.push(tt),
        }
    }
    if !cur.is_empty() {
        out.push(cur.into_iter().collect())
    }
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
            None => {
                return err(
                    Span::call_site(),
                    "expected `=>` after the condition",
                )
            }
            Some(TokenTree::Punct(p))
                if p.as_char() == '='
                    && p.spacing() == proc_macro::Spacing::Joint =>
            {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '>' {
                        i += 2;
                        break;
                    }
                }
                cond.push(toks[i].clone());
                i += 1;
            }
            Some(t) => {
                cond.push(t.clone());
                i += 1
            }
        }
    }
    let cond: TokenStream = cond.into_iter().collect();

    let then = match toks.get(i) {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            i += 1;
            g.clone()
        }
        _ => return err(Span::call_site(), "expected `{ ... }` after `=>`"),
    };
    let otherwise = match (toks.get(i), toks.get(i + 1)) {
        (Some(TokenTree::Ident(kw)), Some(TokenTree::Group(g)))
            if kw.to_string() == "else"
                && g.delimiter() == Delimiter::Brace =>
        {
            Some(g.clone())
        }
        (None, _) => None,
        (Some(t), _) => {
            return err(t.span(), "expected `else { ... }` or the end")
        }
    };

    let mut out = String::from("{ let __c: ::txhdl::types::Bit = ");
    out.push_str(&cond.to_string());
    out.push_str(";\n");
    for (arm, pred) in [(Some(&then), "__c"), (otherwise.as_ref(), "__c.not()")]
    {
        let Some(g) = arm else { continue };
        for st in statements(g) {
            let span = st
                .clone()
                .into_iter()
                .next()
                .map(|t| t.span())
                .unwrap_or(g.span());
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
        Some(TokenTree::Group(g))
            if g.delimiter() == Delimiter::Brace && toks.len() == i + 1 =>
        {
            g.clone()
        }
        _ => {
            return err(
                Span::call_site(),
                "expected `{ pattern => { ... }, ... }` after `=>`",
            )
        }
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
            None => {
                return err(atoks[j].span(), "expected `pattern => { ... }`")
            }
        };
        let body = match atoks.get(k) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                g.clone()
            }
            Some(t) => return err(t.span(), "expected `{ ... }` after `=>`"),
            None => return err(arms.span(), "expected `{ ... }` after `=>`"),
        };
        j = k + 1;
        if let Some(TokenTree::Punct(p)) = atoks.get(j) {
            if p.as_char() == ',' {
                j += 1
            }
        }
        n += 1;
        out.push_str(&format!(
            "{{ let __c: ::txhdl::types::Bit = \
             ::txhdl::types::Bit::from_bool(!__done && matches!(__s, {}));\n",
            pat
        ));
        for st in statements(&body) {
            let span = st
                .clone()
                .into_iter()
                .next()
                .map(|t| t.span())
                .unwrap_or(body.span());
            let Some((lhs, rhs)) = split_becomes(st) else {
                return err(span, "expected `register <= value`");
            };
            out.push_str(&format!("({}).set_if(__c, {});\n", lhs, rhs));
        }
        out.push_str("__done = __done || __c.to_bool(); }\n");
    }
    if n == 0 {
        return err(arms.span(), "expected at least one arm");
    }
    out.push_str("let _ = __done; }");
    out.parse().unwrap()
}

/// Tokens from `from` up to the first `=>`, and the index after it.
fn up_to_arrow(
    toks: &[TokenTree],
    from: usize,
) -> Option<(TokenStream, usize)> {
    let mut i = from;
    let mut acc = Vec::new();
    while i < toks.len() {
        if let TokenTree::Punct(p) = &toks[i] {
            if p.as_char() == '=' && p.spacing() == proc_macro::Spacing::Joint {
                if let Some(TokenTree::Punct(q)) = toks.get(i + 1) {
                    if q.as_char() == '>' {
                        return Some((acc.into_iter().collect(), i + 2));
                    }
                }
            }
        }
        acc.push(toks[i].clone());
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------
// #[pipeline(op = latency, ..)]

/// One value in a pipeline being lowered: its Verilog name, its width,
/// and the stage at which it is ready.
struct Val {
    name: String,
    width: usize,
    ready: usize,
}

/// A call in the body: `let x = op(args).await`, or the tail expression.
struct Call {
    dest: Option<String>,
    op: String,
    args: Vec<Arg>,
}

enum Arg {
    Name(String),
    Lit(u128, Option<usize>),
}

fn parse_call(ts: &[TokenTree]) -> Result<Call, String> {
    let mut i = 0;
    let mut dest = None;
    if let Some(TokenTree::Ident(id)) = ts.get(i) {
        if id.to_string() == "let" {
            let TokenTree::Ident(d) = &ts[i + 1] else {
                return Err("expected a name after let".into());
            };
            dest = Some(d.to_string());
            match ts.get(i + 2) {
                Some(TokenTree::Punct(p)) if p.as_char() == '=' => {}
                _ => return Err("expected `=`".into()),
            }
            i += 3;
        }
    }
    let TokenTree::Ident(op) = &ts[i] else {
        return Err("expected an operator call".into());
    };
    let Some(TokenTree::Group(g)) = ts.get(i + 1) else {
        return Err("expected the operator's arguments".into());
    };
    match (ts.get(i + 2), ts.get(i + 3)) {
        (Some(TokenTree::Punct(p)), Some(TokenTree::Ident(a)))
            if p.as_char() == '.' && a.to_string() == "await" => {}
        _ => return Err("an operator call must be awaited".into()),
    }
    let mut args = Vec::new();
    for a in split_commas(g) {
        args.push(parse_arg(&a)?);
    }
    Ok(Call {
        dest,
        op: op.to_string(),
        args,
    })
}

fn split_commas(g: &Group) -> Vec<Vec<TokenTree>> {
    let toks: Vec<TokenTree> = g.stream().into_iter().collect();
    let mask = turbofish(&toks);
    let mut out = vec![Vec::new()];
    for (i, t) in toks.into_iter().enumerate() {
        match &t {
            TokenTree::Punct(p) if p.as_char() == ',' && !mask[i] => {
                out.push(Vec::new())
            }
            _ => out.last_mut().unwrap().push(t),
        }
    }
    out.retain(|v| !v.is_empty());
    out
}

/// Which tokens lie inside a turbofish, `::<` to its `>`: neither
/// operators nor separators, whatever they look like.
fn turbofish(ts: &[TokenTree]) -> Vec<bool> {
    let mut mask = vec![false; ts.len()];
    let mut depth = 0usize;
    for i in 0..ts.len() {
        if let TokenTree::Punct(p) = &ts[i] {
            let after_colons = i > 0
                && matches!(
                    &ts[i - 1],
                    TokenTree::Punct(q) if q.as_char() == ':'
                );
            if p.as_char() == '<' && (after_colons || depth > 0) {
                depth += 1;
                mask[i] = true;
                continue;
            }
            if p.as_char() == '>' && depth > 0 {
                depth -= 1;
                mask[i] = true;
                continue;
            }
        }
        if depth > 0 {
            mask[i] = true;
        }
    }
    mask
}

/// `name`, `LIT`, `U::from(LIT)` or `U::<W>::from(LIT)`.
fn parse_arg(ts: &[TokenTree]) -> Result<Arg, String> {
    match ts {
        [TokenTree::Ident(id)] if id.to_string() != "U" => {
            Ok(Arg::Name(id.to_string()))
        }
        [TokenTree::Literal(l)] => Ok(Arg::Lit(
            l.to_string()
                .replace('_', "")
                .parse()
                .map_err(|_| "a literal")?,
            None,
        )),
        _ => {
            let text: String = ts
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("");
            let width = text
                .strip_prefix("U::<")
                .and_then(|r| r.split_once('>'))
                .and_then(|(w, _)| w.parse().ok());
            let lit = text
                .rsplit_once("from(")
                .and_then(|(_, r)| r.strip_suffix(')'))
                .and_then(|l| l.replace('_', "").parse().ok())
                .ok_or_else(|| format!("cannot lower the argument `{text}`"))?;
            Ok(Arg::Lit(lit, width))
        }
    }
}

/// `#[pipeline(mul = 1, add = 1)]` on an `async fn`: the function stays
/// as written for simulation, and a `NAME_verilog()` beside it returns
/// its lowering. Every awaited operator is a stage of the latency the
/// attribute gives it; a value used later than it is ready is delayed
/// by registers to meet the operator, which is what a stage boundary
/// stored. The body may contain `let x = op(args).await;` statements,
/// a tail `op(args).await`, and macro calls such as `println!`, which
/// are skipped.
#[proc_macro_attribute]
pub fn pipeline(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Latencies.
    let mut latency: Vec<(String, usize)> = Vec::new();
    let atoks: Vec<TokenTree> = attr.into_iter().collect();
    let mut i = 0;
    while i + 2 < atoks.len() + 1 && i < atoks.len() {
        let (TokenTree::Ident(op), Some(TokenTree::Literal(n))) =
            (&atoks[i], atoks.get(i + 2))
        else {
            return err(Span::call_site(), "expected `op = latency`");
        };
        latency.push((op.to_string(), n.to_string().parse().unwrap_or(0)));
        i += 4;
    }
    // The function: `[pub] async fn NAME(params) -> U<R> { body }`.
    let toks: Vec<TokenTree> = item.clone().into_iter().collect();
    let mut i = 0;
    while !matches!(&toks[i], TokenTree::Ident(id) if id.to_string() == "fn") {
        i += 1;
    }
    let name = toks[i + 1].to_string();
    let TokenTree::Group(params) = &toks[i + 2] else {
        return err(Span::call_site(), "expected parameters");
    };
    let mut vals: Vec<Val> = Vec::new();
    let mut ports = Vec::new();
    for p in split_commas(params) {
        let (TokenTree::Ident(pname), Some(w)) = (&p[0], width_of(&p[2..]))
        else {
            return err(p[0].span(), "a parameter must be `name: U<N>`");
        };
        ports.push(format!("input [{}:0] {}", w - 1, pname));
        vals.push(Val {
            name: pname.to_string(),
            width: w,
            ready: 0,
        });
    }
    let Some(ret) = width_of(&toks[i + 4..]) else {
        return err(Span::call_site(), "the return type must be `U<N>`");
    };
    let TokenTree::Group(body) = toks.last().unwrap() else {
        return err(Span::call_site(), "expected a body");
    };
    // The body: operator calls, in order.
    let mut calls = Vec::new();
    for st in statements(body) {
        let ts: Vec<TokenTree> = st.into_iter().collect();
        let is_macro = matches!(
            (&ts[0], ts.get(1)),
            (TokenTree::Ident(_), Some(TokenTree::Punct(p)))
                if p.as_char() == '!'
        );
        if is_macro {
            continue; // println! and friends: not hardware
        }
        match parse_call(&ts) {
            Ok(c) => calls.push(c),
            Err(m) => return err(ts[0].span(), &m),
        }
    }
    // Schedule and emit.
    let mut regs: Vec<String> = Vec::new();
    let mut seq: Vec<String> = Vec::new();
    let mut comb: Vec<String> = Vec::new();
    let mut last = String::new();
    let mut temp = 0;
    for c in &calls {
        let Some(&(_, lat)) = latency.iter().find(|(o, _)| *o == c.op) else {
            return err(
                Span::call_site(),
                &format!("no latency given for `{}`", c.op),
            );
        };
        // Resolve the arguments and the stage they meet at.
        let mut operands: Vec<(String, usize, usize)> = Vec::new();
        for a in &c.args {
            match a {
                Arg::Name(n) => {
                    let Some(v) = vals.iter().find(|v| v.name == *n) else {
                        return err(
                            Span::call_site(),
                            &format!("`{n}` is not a value here"),
                        );
                    };
                    operands.push((v.name.clone(), v.width, v.ready));
                }
                Arg::Lit(l, w) => {
                    let w = w.unwrap_or(ret);
                    operands.push((format!("{w}'d{l}"), w, 0));
                }
            }
        }
        let meet = operands.iter().map(|o| o.2).max().unwrap_or(0);
        let mut names = Vec::new();
        for (n, w, ready) in &operands {
            let mut cur = n.clone();
            if n.contains("'d") {
                names.push(cur);
                continue;
            }
            // Delay a value that is ready early until the operands meet.
            for k in *ready..meet {
                let next = format!("{n}_d{}", k + 1);
                regs.push(format!("  reg [{}:0] {next};", w - 1));
                seq.push(format!("    {next} <= {cur};"));
                cur = next;
            }
            names.push(cur);
        }
        let sym = match c.op.as_str() {
            "mul" => "*",
            "add" => "+",
            "sub" => "-",
            other => {
                return err(
                    Span::call_site(),
                    &format!("unknown operator `{other}`"),
                )
            }
        };
        let width = if c.op == "mul" {
            operands.iter().map(|o| o.1).sum()
        } else {
            operands.iter().map(|o| o.1).max().unwrap_or(ret)
        };
        let dest = c.dest.clone().unwrap_or_else(|| {
            temp += 1;
            format!("t{temp}")
        });
        let expr = names.join(&format!(" {sym} "));
        let mut cur = expr;
        for k in 0..lat {
            let next = if k + 1 == lat {
                dest.clone()
            } else {
                format!("{dest}_s{}", k + 1)
            };
            regs.push(format!("  reg [{}:0] {next};", width - 1));
            seq.push(format!("    {next} <= {cur};"));
            cur = next;
        }
        if lat == 0 {
            comb.push(format!("  wire [{}:0] {dest} = {cur};", width - 1));
        }
        vals.push(Val {
            name: dest.clone(),
            width,
            ready: meet + lat,
        });
        last = dest;
    }
    let depth = vals.last().map(|v| v.ready).unwrap_or(0);
    let mut v = String::new();
    v.push_str(&format!(
        "// {name}: {} stages, lowered by #[pipeline]\n\
         module {name}(input clk, {}, output [{}:0] out);\n",
        depth,
        ports.join(", "),
        ret - 1
    ));
    for r in &regs {
        v.push_str(r);
        v.push('\n');
    }
    for c in &comb {
        v.push_str(c);
        v.push('\n');
    }
    if !seq.is_empty() {
        v.push_str("  always @(posedge clk) begin\n");
        for s in &seq {
            v.push_str(s);
            v.push('\n');
        }
        v.push_str("  end\n");
    }
    v.push_str(&format!("  assign out = {last};\nendmodule\n"));
    let mut out = item;
    let extra: TokenStream = format!(
        "/// The Verilog of `{name}`, as `#[pipeline]` lowered it.\n\
         pub fn {name}_verilog() -> String {{ String::from(r#\"{v}\"#) }}"
    )
    .parse()
    .unwrap();
    out.extend(extra);
    out
}

/// The `N` of a `U<N>` type, from its tokens.
fn width_of(ts: &[TokenTree]) -> Option<usize> {
    let text: String = ts
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join("");
    let r = text
        .trim_start_matches(|c| c == '-' || c == '>')
        .trim_start_matches("U<");
    r.split('>').next()?.trim().parse().ok()
}

// ---------------------------------------------------------------------
// #[lower]

/// The Rust source of an `Expr` for a name.
fn ename(n: &str) -> String {
    format!("E::Name(\"{n}\".to_string())")
}

/// The Rust source of an `Expr::Bin`.
fn ebin(op: &str, a: &str, b: &str) -> String {
    format!("E::Bin(\"{op}\", Box::new({a}), Box::new({b}))")
}

/// The plain name a drive targets: `x` or `self.x`.
/// A drive's target as the Rust source of a `Target`: a name, or
/// `self.m.at(addr)`, a word of a memory.
fn target_expr(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Result<String, String> {
    if ts.len() >= 4 {
        let end = ts.len() - 3;
        if let (
            TokenTree::Punct(dot),
            TokenTree::Ident(m),
            TokenTree::Group(g),
        ) = (&ts[end], &ts[end + 1], &ts[end + 2])
        {
            if dot.as_char() == '.' && m.to_string() == "at" {
                let mem = target_name(&ts[..end])?;
                let at: Vec<TokenTree> = g.stream().into_iter().collect();
                let a = tr(&at, subst)?;
                return Ok(format!("T::Word(\"{mem}\".to_string(), {a})"));
            }
        }
    }
    Ok(format!("T::Name(\"{}\".to_string())", target_name(ts)?))
}

fn target_name(ts: &[TokenTree]) -> Result<String, String> {
    match ts {
        [TokenTree::Ident(id)] => Ok(id.to_string()),
        [TokenTree::Ident(s), TokenTree::Punct(_), TokenTree::Ident(f)]
            if matches!(s.to_string().as_str(), "self" | "this") =>
        {
            Ok(f.to_string())
        }
        _ => Err("a drive's target must be a name or `self.field`".into()),
    }
}

/// Translate a Rust expression, in the subset a unit body uses, into
/// the Rust source that builds its `Expr` when `lowered()` runs.
/// Constants of the configuration are left as the Rust expressions they
/// are, so they are evaluated then. `subst` maps `let` names to the
/// source of what they stand for.
fn tr(ts: &[TokenTree], subst: &[(String, String)]) -> Result<String, String> {
    if ts.is_empty() {
        return Err("empty expression".into());
    }
    // `select!(v => { pat => e, .. })`: a chain of conditions.
    if let [TokenTree::Ident(m), TokenTree::Punct(bang), TokenTree::Group(g)] =
        ts
    {
        if m.to_string() == "select" && bang.as_char() == '!' {
            return tr_select(g, subst);
        }
    }
    let mask = turbofish(ts);
    let infix: &[&str] =
        &["||", "&&", "==", "!=", "<=", ">=", "<", ">", "+", "-"];
    for op in infix {
        let n = op.len();
        let mut i = 1;
        while i + n <= ts.len() {
            if mask[i] {
                i += 1;
                continue;
            }
            let here: String =
                ts[i..i + n].iter().map(|t| t.to_string()).collect();
            let all_punct = ts[i..i + n]
                .iter()
                .all(|t| matches!(t, TokenTree::Punct(_)));
            let next_is_punct =
                matches!(ts.get(i + n), Some(TokenTree::Punct(_)));
            if all_punct && here == *op && !next_is_punct {
                let prev = ts[i - 1].to_string();
                let generic =
                    (*op == "<" || *op == ">") && (prev == ":" || prev == "U");
                if !generic {
                    let l = tr(&ts[..i], subst)?;
                    let r = tr(&ts[i + n..], subst)?;
                    return Ok(ebin(op, &l, &r));
                }
            }
            i += 1;
        }
    }
    if let Some(p) = ts.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "as"),
    ) {
        return tr(&ts[..p], subst);
    }
    if let TokenTree::Punct(p) = &ts[0] {
        if p.as_char() == '!' {
            return Ok(format!("E::Not(Box::new({}))", tr(&ts[1..], subst)?));
        }
    }
    let end = ts.len();
    // A method with a turbofish: `x.slice::<LO, LEN>()`, `x.sext::<M>()`,
    // `x.zext::<M>()`, `x.concat::<K, M>(low)`.
    if end >= 9 {
        if let (TokenTree::Group(g), TokenTree::Punct(gt)) =
            (&ts[end - 1], &ts[end - 2])
        {
            if gt.as_char() == '>' && g.delimiter() == Delimiter::Parenthesis {
                let mut depth = 0;
                let mut lt = None;
                for j in (0..end - 2).rev() {
                    if let TokenTree::Punct(p) = &ts[j] {
                        match p.as_char() {
                            '>' => depth += 1,
                            '<' if depth == 0 => {
                                lt = Some(j);
                                break;
                            }
                            '<' => depth -= 1,
                            _ => {}
                        }
                    }
                }
                if let Some(lt) = lt.filter(|&lt| lt >= 4) {
                    if let (
                        TokenTree::Punct(c1),
                        TokenTree::Punct(c2),
                        TokenTree::Ident(m),
                        TokenTree::Punct(dot),
                    ) = (&ts[lt - 1], &ts[lt - 2], &ts[lt - 3], &ts[lt - 4])
                    {
                        if c1.as_char() == ':'
                            && c2.as_char() == ':'
                            && dot.as_char() == '.'
                        {
                            let l = tr(&ts[..lt - 4], subst)?;
                            let ks: Vec<String> = ts[lt + 1..end - 2]
                                .iter()
                                .filter(|t| !matches!(t, TokenTree::Punct(_)))
                                .map(|t| t.to_string())
                                .collect();
                            let args = split_commas(g);
                            return Ok(match m.to_string().as_str() {
                                "slice" => format!(
                                    "E::Slice(Box::new({l}), {}, {})",
                                    ks[0], ks[1]
                                ),
                                "sext" => {
                                    format!("E::Sext(Box::new({l}), {})", ks[0])
                                }
                                "zext" | "resize" => {
                                    format!("E::Zext(Box::new({l}), {})", ks[0])
                                }
                                "concat" => format!(
                                    "E::Cat(Box::new({l}), Box::new({}))",
                                    tr(&args[0], subst)?
                                ),
                                // Both operands at the result's width,
                                // so the product is that wide in either
                                // target language.
                                "mul" => format!(
                                    "E::Bin(\"*\", \
                                     Box::new(E::Zext(Box::new({l}), {0})), \
                                     Box::new(E::Zext(Box::new({1}), {0})))",
                                    ks[0],
                                    tr(&args[0], subst)?
                                ),
                                other => {
                                    return Err(format!(
                                        "method `{other}::<..>` is not lowered"
                                    ))
                                }
                            });
                        }
                    }
                }
            }
        }
    }
    if end >= 3 {
        if let (
            TokenTree::Punct(dot),
            TokenTree::Ident(m),
            TokenTree::Group(g),
        ) = (&ts[end - 3], &ts[end - 2], &ts[end - 1])
        {
            if dot.as_char() == '.' {
                let args = split_commas(g);
                let m = m.to_string();
                let recv = &ts[..end - 3];
                // A channel's parts are named after the channel.
                if matches!(m.as_str(), "peek" | "ready" | "recv") {
                    let ch = target_name(recv)?;
                    let part = match m.as_str() {
                        "peek" => "valid",
                        "ready" => "ready",
                        _ => "data",
                    };
                    return Ok(ename(&format!("{ch}_{part}")));
                }
                let l = tr(recv, subst)?;
                let mut a = Vec::new();
                for x in &args {
                    a.push(tr(x, subst)?);
                }
                return Ok(match m.as_str() {
                    "wrapping_add" => ebin("+", &l, &a[0]),
                    "shl" => ebin("<<", &l, &a[0]),
                    "shr" => ebin(">>", &l, &a[0]),
                    "sra" => ebin(">>>", &l, &a[0]),
                    "xor" => ebin("^", &l, &a[0]),
                    "lt_signed" => ebin("<s", &l, &a[0]),
                    "eq" => ebin("==", &l, &a[0]),
                    "wrapping_sub" => ebin("-", &l, &a[0]),
                    "and" => ebin("&", &l, &a[0]),
                    "or" => ebin("|", &l, &a[0]),
                    "not" => format!("E::Not(Box::new({l}))"),
                    "bit" | "read" => {
                        format!("E::Index(Box::new({l}), Box::new({}))", a[0])
                    }
                    "raw" | "to_bool" | "get" | "is_some" | "zext"
                    | "unwrap_or_default" => l,
                    other => {
                        return Err(format!("method `{other}` is not lowered"))
                    }
                });
            }
        }
    }
    let text: String = ts
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join("");
    match ts {
        [TokenTree::Ident(id)] => {
            let n = id.to_string();
            Ok(subst
                .iter()
                .find(|(k, _)| *k == n)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| ename(&n)))
        }
        [TokenTree::Literal(l)] => {
            Ok(format!("E::Num({})", l.to_string().replace('_', "")))
        }
        [TokenTree::Group(g)]
            if matches!(
                g.delimiter(),
                Delimiter::Parenthesis | Delimiter::Brace
            ) =>
        {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            tr(&inner, subst)
        }
        [TokenTree::Ident(s), TokenTree::Punct(_), TokenTree::Ident(f)]
            if matches!(s.to_string().as_str(), "self" | "this") =>
        {
            Ok(ename(&f.to_string()))
        }
        // A struct literal, `Name { f: e, .. }`: its fields concatenated
        // in the order written, which must be the declaration's, the
        // first field highest, as `#[derive(Value)]` lays them out.
        [TokenTree::Ident(_), TokenTree::Group(g)]
            if g.delimiter() == Delimiter::Brace =>
        {
            let mut acc: Option<String> = None;
            for f in split_commas(g) {
                let Some(c) = f.iter().position(
                    |t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'),
                ) else {
                    return Err(
                        "a struct literal's field is `name: value`".into()
                    );
                };
                let e = tr(&f[c + 1..], subst)?;
                acc = Some(match acc {
                    None => e,
                    Some(a) => format!("E::Cat(Box::new({a}), Box::new({e}))"),
                });
            }
            acc.ok_or_else(|| "an empty struct literal".to_string())
        }
        _ => {
            let last_group = matches!(ts.last(), Some(TokenTree::Group(_)));
            if ts.len() == 2 {
                if let (TokenTree::Ident(f), TokenTree::Group(g)) =
                    (&ts[0], &ts[1])
                {
                    let mut v = Vec::new();
                    for x in split_commas(g) {
                        v.push(tr(&x, subst)?);
                    }
                    return Ok(match f.to_string().as_str() {
                        "eq" => ebin("==", &v[0], &v[1]),
                        "shl" => ebin("<<", &v[0], &v[1]),
                        "shr" => ebin(">>", &v[0], &v[1]),
                        "sra" => ebin(">>>", &v[0], &v[1]),
                        "band" => ebin("&", &v[0], &v[1]),
                        "bor" => ebin("|", &v[0], &v[1]),
                        "bxor" => ebin("^", &v[0], &v[1]),
                        "lt_signed" => ebin("<s", &v[0], &v[1]),
                        "ne" => ebin("!=", &v[0], &v[1]),
                        "lt" => ebin("<", &v[0], &v[1]),
                        "gt" => ebin(">", &v[0], &v[1]),
                        "mux" => format!(
                            "E::Cond(Box::new({}), Box::new({}), Box::new({}))",
                            v[0], v[1], v[2]
                        ),
                        "is_zero" => ebin("==", &v[0], "E::Num(0)"),
                        other => {
                            return Err(format!(
                                "function `{other}` is not lowered"
                            ))
                        }
                    });
                }
            }
            if text == "Bit::One" {
                return Ok("E::Bits(1, \"1\".to_string())".into());
            }
            if text == "Bit::Zero" {
                return Ok("E::Bits(1, \"0\".to_string())".into());
            }
            if let Some(TokenTree::Group(g)) = ts.last() {
                if text.starts_with("Bit::from_bool") {
                    let inner: Vec<TokenTree> =
                        g.stream().into_iter().collect();
                    return tr(&inner, subst);
                }
                // A sized literal keeps its width; a bare one is a number.
                if text.starts_with("U::<") {
                    return Ok(format!("::txhdl::netlist::lit({text})"));
                }
                if text.starts_with("U::from") {
                    return Ok(format!("E::Num(({}) as u128)", g.stream()));
                }
            }
            if text.contains("::") && !last_group {
                let leaf = text.rsplit("::").next().unwrap_or("");
                let upper = leaf.chars().all(|c| {
                    c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()
                });
                return Ok(if upper {
                    format!("E::Num(({text}) as u128)")
                } else {
                    format!("::txhdl::netlist::lit({text})")
                });
            }
            Err(format!("cannot lower `{text}`"))
        }
    }
}

/// `select!(v => { pat => e, .., _ => e })` as a chain of conditions:
/// the first arm's condition selects its value, else the next, down
/// to the last arm, whose value is the default.
fn tr_select(g: &Group, subst: &[(String, String)]) -> Result<String, String> {
    let ct: Vec<TokenTree> = g.stream().into_iter().collect();
    let Some((value, i)) = up_to_arrow(&ct, 0) else {
        return Err("select! needs `value =>`".into());
    };
    let vt: Vec<TokenTree> = value.into_iter().collect();
    let v = tr(&vt, subst)?;
    let Some(TokenTree::Group(arms)) = ct.get(i) else {
        return Err("select! needs `{ pattern => value, .. }`".into());
    };
    let mut chain: Vec<(String, String)> = Vec::new();
    for arm in split_commas(arms) {
        let Some((pat, k)) = up_to_arrow(&arm, 0) else {
            return Err("a select! arm is `pattern => value`".into());
        };
        let pt: Vec<TokenTree> = pat.into_iter().collect();
        let c = pattern_cond(&pt, &v, subst)?;
        let e = tr(&arm[k..], subst)?;
        chain.push((c, e));
    }
    let Some((_, mut acc)) = chain.pop() else {
        return Err("select! needs an arm".into());
    };
    for (c, e) in chain.into_iter().rev() {
        acc = format!("E::Cond(Box::new({c}), Box::new({e}), Box::new({acc}))");
    }
    Ok(acc)
}

/// The first `,` at angle depth 0 of a type's text.
fn depth0_comma(s: &str) -> Option<usize> {
    let mut d = 0;
    for (i, c) in s.char_indices() {
        match c {
            '<' => d += 1,
            '>' => d -= 1,
            ',' if d == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// A `case!` pattern as a condition on `v`: alternatives joined by
/// `||`, `_` as true, an `if` guard joined by `&&`, and a variant as
/// equality with its literal.
fn pattern_cond(
    pt: &[TokenTree],
    v: &str,
    subst: &[(String, String)],
) -> Result<String, String> {
    let guard_at = pt.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "if"),
    );
    let (pat, guard) = match guard_at {
        Some(g) => (&pt[..g], Some(&pt[g + 1..])),
        None => (pt, None),
    };
    let mut alts: Vec<Vec<TokenTree>> = vec![Vec::new()];
    for t in pat {
        match t {
            TokenTree::Punct(p) if p.as_char() == '|' => alts.push(Vec::new()),
            _ => alts.last_mut().unwrap().push(t.clone()),
        }
    }
    let mut cond: Option<String> = None;
    for a in &alts {
        let text: String =
            a.iter().map(|t| t.to_string()).collect::<Vec<_>>().join("");
        let digit = text.chars().next().is_some_and(|c| c.is_ascii_digit());
        let c = if text == "_" {
            "E::Bits(1, \"1\".to_string())".to_string()
        } else if digit {
            ebin("==", v, &format!("E::Num(({text}) as u128)"))
        } else {
            ebin("==", v, &format!("::txhdl::netlist::lit({text})"))
        };
        cond = Some(match cond {
            None => c,
            Some(prev) => ebin("||", &prev, &c),
        });
    }
    let mut cond =
        cond.unwrap_or_else(|| "E::Bits(1, \"1\".to_string())".to_string());
    if let Some(g) = guard {
        cond = ebin("&&", &cond, &tr(g, subst)?);
    }
    Ok(cond)
}

/// `#[lower]` on `impl Unit<In, Out> for Unit`: the impl stays, and
/// `Unit::verilog(name)` is written beside it. `run` must be one
/// `loop` whose first statement waits for a rising edge; the rest may
/// read registers into `let` names, drive registers and output ports
/// with `set`, predicate drives with `when!`, and call macros such as
/// `println!`, which are skipped. Constants of the configuration are
/// evaluated when `verilog` runs, so a generic unit lowers once per
/// build.
#[proc_macro_attribute]
pub fn lower(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = item.clone().into_iter().collect();
    // Past any attributes and doc comments, to `impl`.
    let Some(at) = toks.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "impl"),
    ) else {
        return err(Span::call_site(), "expected an `impl`");
    };
    let mut i = at + 1;
    let mut generics = String::new();
    if matches!(&toks[i], TokenTree::Punct(p) if p.as_char() == '<') {
        let start = i;
        let mut depth = 0;
        loop {
            match &toks[i] {
                TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
                TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
                _ => {}
            }
            i += 1;
            if depth == 0 {
                break;
            }
        }
        generics = toks[start..i]
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let Some(f) = toks.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "for"),
    ) else {
        return err(Span::call_site(), "expected `impl Unit<..> for Unit`");
    };
    let TokenTree::Group(body) = toks.last().unwrap() else {
        return err(Span::call_site(), "expected an impl body");
    };
    let unit: String = toks[f + 1..toks.len() - 1]
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let bt: Vec<TokenTree> = body.stream().into_iter().collect();
    let Some(r) = bt.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "run"),
    ) else {
        return err(body.span(), "expected `run`");
    };
    let TokenTree::Group(params) = &bt[r + 1] else {
        return err(body.span(), "expected run's parameters");
    };
    // A port is `name: In<T>`; a side with several ports is a tuple,
    // `(a, b): (In<X>, In<Y>)`, paired name by name.
    let mut pairs: Vec<(String, String, Span)> = Vec::new();
    for p in split_commas(params).into_iter().skip(1) {
        let Some(colon) = p.iter().position(
            |t| matches!(t, TokenTree::Punct(c) if c.as_char() == ':'),
        ) else {
            return err(p[0].span(), "a port must be `name: Out<T>`");
        };
        let text = |ts: &[TokenTree]| -> String {
            ts.iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("")
        };
        match (&p[0], &p[colon + 1]) {
            (TokenTree::Ident(n), _) => {
                pairs.push((n.to_string(), text(&p[colon + 1..]), n.span()));
            }
            (TokenTree::Group(names), TokenTree::Group(tys))
                if names.delimiter() == Delimiter::Parenthesis
                    && tys.delimiter() == Delimiter::Parenthesis =>
            {
                let ns = split_commas(names);
                let ts = split_commas(tys);
                if ns.len() != ts.len() {
                    return err(names.span(), "ports and types differ");
                }
                for (n, t) in ns.iter().zip(ts.iter()) {
                    let TokenTree::Ident(n) = &n[0] else {
                        return err(n[0].span(), "a port is a name");
                    };
                    pairs.push((n.to_string(), text(t), n.span()));
                }
            }
            _ => {
                return err(
                    p[0].span(),
                    "a port must be `name: Out<T>` or `name: In<T>`",
                )
            }
        }
    }
    let pnames: Vec<String> = pairs.iter().map(|(n, _, _)| n.clone()).collect();
    let mut ports: Vec<String> = Vec::new();
    for (pname, ty, span) in pairs {
        if ty == "()" {
            continue;
        }
        let (kind, inner) = if let Some(x) = ty.strip_prefix("Out<") {
            ("Out", x)
        } else if let Some(x) = ty.strip_prefix("In<") {
            ("In", x)
        } else if let Some(x) = ty.strip_prefix("Tx<") {
            ("Tx", x)
        } else if let Some(x) = ty.strip_prefix("Rx<") {
            ("Rx", x)
        } else {
            return err(span, "a port must be an Out, In, Tx or Rx");
        };
        // The transaction type: up to the clock argument, if any.
        let inner = inner.strip_suffix('>').unwrap_or(inner);
        let inner = match depth0_comma(inner) {
            Some(c) => &inner[..c],
            None => inner,
        };
        ports.push(format!(
            "(\"{pname}\".to_string(), ::txhdl::comp::trace::Kind::{kind}, \
             <{inner} as ::txhdl::types::Value>::WIDTH)"
        ));
    }
    // run's body: the brace group after its parameters; inside it, the loop.
    fn is_brace(t: &&TokenTree) -> bool {
        matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
    }
    let Some(TokenTree::Group(fbody)) = bt[r + 2..].iter().find(is_brace)
    else {
        return err(body.span(), "expected run's body");
    };
    // Every `loop` in run's body is a process: one, or several under
    // `join2(async { loop .. }, async { loop .. })`.
    fn find_loops(ts: &[TokenTree], out: &mut Vec<Group>) {
        let mut i = 0;
        while i < ts.len() {
            match (&ts[i], ts.get(i + 1)) {
                (TokenTree::Ident(id), Some(TokenTree::Group(g)))
                    if id.to_string() == "loop"
                        && g.delimiter() == Delimiter::Brace =>
                {
                    out.push(g.clone());
                    i += 2;
                    continue;
                }
                (TokenTree::Group(g), _) => {
                    let inner: Vec<TokenTree> =
                        g.stream().into_iter().collect();
                    find_loops(&inner, out);
                }
                _ => {}
            }
            i += 1;
        }
    }
    let ft: Vec<TokenTree> = fbody.stream().into_iter().collect();
    let mut loops: Vec<Group> = Vec::new();
    find_loops(&ft, &mut loops);
    if loops.is_empty() {
        return err(fbody.span(), "run must be a `loop`, or `join2` of loops");
    }
    // `let` names that became wires of the netlist, with what drives
    // each; a name bound twice gets a numbered second wire.
    let mut wires: Vec<(String, String)> = Vec::new();
    let mut procs: Vec<String> = Vec::new();
    for lbody in &loops {
        let mut clock = String::new();
        let mut falling = false;
        let mut guard: Option<String> = None;
        let mut subst: Vec<(String, String)> = Vec::new();
        let mut stmts: Vec<String> = Vec::new();
        for st in statements(lbody) {
            let ts: Vec<TokenTree> = st.into_iter().collect();
            let text: String = ts
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("");
            if let Some(c) = text.strip_suffix("::rising().await") {
                clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
                continue;
            }
            if let Some(c) = text.strip_suffix("::falling().await") {
                clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
                falling = true;
                continue;
            }
            // until(C::rising, || cond).await: the wait, and the guard on
            // every register drive after it.
            if text.starts_with("until(") && text.ends_with(").await") {
                let TokenTree::Group(g) = &ts[1] else {
                    return err(ts[0].span(), "expected until(..)");
                };
                let parts = split_commas(g);
                if parts.len() != 2 {
                    return err(
                        ts[0].span(),
                        "expected until(C::rising, || cond)",
                    );
                }
                let ctext: String = parts[0]
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join("");
                let c = if let Some(c) = ctext.strip_suffix("::rising") {
                    c
                } else if let Some(c) = ctext.strip_suffix("::falling") {
                    falling = true;
                    c
                } else {
                    return err(ts[0].span(), "expected until(C::rising, ..)");
                };
                clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
                let body = &parts[1][2..]; // past the `||` of the closure
                let cond = match tr(body, &subst) {
                    Ok(c) => c,
                    Err(m) => return err(ts[0].span(), &m),
                };
                guard = Some(cond.clone());
                stmts.push(format!("S::Guard({cond})"));
                continue;
            }
            // let v = rx.wait().await: a receive is the wait, and its guard.
            if text.starts_with("let") && text.ends_with(".wait().await") {
                let TokenTree::Ident(n) = &ts[1] else {
                    return err(ts[1].span(), "expected a name");
                };
                let TokenTree::Ident(rx) = &ts[3] else {
                    return err(ts[3].span(), "expected a channel");
                };
                let cond = ename(&format!("{rx}_valid"));
                guard = Some(cond.clone());
                stmts.push(format!("S::Guard({cond})"));
                stmts.push(format!(
                    "S::Drive(T::Name(\"{rx}_ready\".to_string()), \
                 E::Bits(1, \"1\".to_string()))"
                ));
                subst.push((n.to_string(), ename(&format!("{rx}_data"))));
                continue;
            }
            let is_macro = matches!(
                (&ts[0], ts.get(1)),
                (TokenTree::Ident(_), Some(TokenTree::Punct(p)))
                    if p.as_char() == '!'
            );
            if is_macro
                && !text.starts_with("when!")
                && !text.starts_with("case!")
            {
                continue;
            }
            // A receive under the guard: ready is asserted with the guard.
            if text.contains(".recv()") {
                let Some(g) = &guard else {
                    return err(ts[0].span(), "recv needs a wait before it");
                };
                let TokenTree::Ident(rx) = &ts[3] else {
                    return err(ts[0].span(), "expected `let v = rx.recv()`");
                };
                stmts.push(format!(
                    "S::Drive(T::Name(\"{rx}_ready\".to_string()), {g})"
                ));
            }
            if text.starts_with("let") {
                // `let a = e` or `let (a, b) = (e1, e2)`, bound pairwise.
                let (names, exprs): (Vec<Vec<TokenTree>>, Vec<Vec<TokenTree>>) =
                    match (&ts[1], ts.get(3)) {
                        (TokenTree::Group(ng), Some(TokenTree::Group(eg))) => {
                            (split_commas(ng), split_commas(eg))
                        }
                        _ => {
                            (vec![vec![ts[1].clone()]], vec![ts[3..].to_vec()])
                        }
                    };
                if names.len() != exprs.len() {
                    return err(ts[0].span(), "a tuple let must bind pairwise");
                }
                for (n, e) in names.iter().zip(&exprs) {
                    let name = n[0].to_string();
                    let v = match tr(e, &subst) {
                        Ok(v) => v,
                        Err(m) => return err(ts[0].span(), &m),
                    };
                    // A read of a port or a register, or a number, is an
                    // alias; anything computed is a wire named for the let.
                    let alias = v.starts_with("E::Name(")
                        || v.starts_with("E::Num(")
                        || v.starts_with("E::Bits(")
                        || v.starts_with("::txhdl::netlist::lit(");
                    if alias || name == "_" {
                        subst.push((name, v));
                        continue;
                    }
                    let mut w = name.clone();
                    if pnames.contains(&w) {
                        w.push_str("_w");
                    }
                    let taken = wires.iter().filter(|(x, _)| *x == w).count();
                    if taken > 0 {
                        w = format!("{w}_{}", taken + 1);
                    }
                    wires.push((w.clone(), v));
                    subst.push((name, ename(&w)));
                }
                continue;
            }
            if text.starts_with("case!") {
                let TokenTree::Group(g) = &ts[2] else {
                    return err(ts[0].span(), "expected case!(..)");
                };
                let ct: Vec<TokenTree> = g.stream().into_iter().collect();
                let Some((value, k)) = up_to_arrow(&ct, 0) else {
                    return err(ts[0].span(), "expected `value =>`");
                };
                let vt: Vec<TokenTree> = value.into_iter().collect();
                let v = match tr(&vt, &subst) {
                    Ok(v) => v,
                    Err(m) => return err(ts[0].span(), &m),
                };
                let TokenTree::Group(arms_g) = &ct[k] else {
                    return err(ts[0].span(), "expected the arms");
                };
                let at: Vec<TokenTree> = arms_g.stream().into_iter().collect();
                let mut arms = Vec::new();
                let mut j = 0;
                while j < at.len() {
                    let Some((pat, k2)) = up_to_arrow(&at, j) else {
                        return err(
                            at[j].span(),
                            "expected `pattern => { .. }`",
                        );
                    };
                    let pt: Vec<TokenTree> = pat.into_iter().collect();
                    let cond = match pattern_cond(&pt, &v, &subst) {
                        Ok(c) => c,
                        Err(m) => return err(at[j].span(), &m),
                    };
                    let TokenTree::Group(body) = &at[k2] else {
                        return err(at[j].span(), "expected `{ .. }`");
                    };
                    let mut drives = Vec::new();
                    for d in statements(body) {
                        let Some((lhs, rhs)) = split_becomes(d) else {
                            return err(
                                body.span(),
                                "expected `register <= value`",
                            );
                        };
                        let lt: Vec<TokenTree> = lhs.into_iter().collect();
                        let rt: Vec<TokenTree> = rhs.into_iter().collect();
                        let l = match target_expr(&lt, &subst) {
                            Ok(l) => l,
                            Err(m) => return err(body.span(), &m),
                        };
                        let r = match tr(&rt, &subst) {
                            Ok(r) => r,
                            Err(m) => return err(body.span(), &m),
                        };
                        drives.push(format!("({l}, {r})"));
                    }
                    arms.push(format!("({cond}, vec![{}])", drives.join(", ")));
                    j = k2 + 1;
                    if let Some(TokenTree::Punct(p)) = at.get(j) {
                        if p.as_char() == ',' {
                            j += 1;
                        }
                    }
                }
                stmts.push(format!("S::Case(vec![{}])", arms.join(", ")));
                continue;
            }
            if text.starts_with("when!") {
                let TokenTree::Group(g) = &ts[2] else {
                    return err(ts[0].span(), "expected when!(..)");
                };
                let wt: Vec<TokenTree> = g.stream().into_iter().collect();
                let Some((cond, k)) = up_to_arrow(&wt, 0) else {
                    return err(ts[0].span(), "expected `cond =>`");
                };
                let ct: Vec<TokenTree> = cond.into_iter().collect();
                let c = match tr(&ct, &subst) {
                    Ok(c) => c,
                    Err(m) => return err(ts[0].span(), &m),
                };
                let TokenTree::Group(then) = &wt[k] else {
                    return err(ts[0].span(), "expected `{ .. }`");
                };
                let otherwise = match (wt.get(k + 1), wt.get(k + 2)) {
                    (Some(TokenTree::Ident(e)), Some(TokenTree::Group(g)))
                        if e.to_string() == "else" =>
                    {
                        Some(g)
                    }
                    _ => None,
                };
                let mut arms = Vec::new();
                for arm in [Some(then), otherwise] {
                    let mut drives = Vec::new();
                    if let Some(g) = arm {
                        for d in statements(g) {
                            let Some((lhs, rhs)) = split_becomes(d) else {
                                return err(
                                    g.span(),
                                    "expected `register <= value`",
                                );
                            };
                            let lt: Vec<TokenTree> = lhs.into_iter().collect();
                            let rt: Vec<TokenTree> = rhs.into_iter().collect();
                            let l = match target_expr(&lt, &subst) {
                                Ok(l) => l,
                                Err(m) => return err(g.span(), &m),
                            };
                            let r = match tr(&rt, &subst) {
                                Ok(r) => r,
                                Err(m) => return err(g.span(), &m),
                            };
                            drives.push(format!("({l}, {r})"));
                        }
                    }
                    arms.push(format!("vec![{}]", drives.join(", ")));
                }
                stmts.push(format!("S::When({c}, {}, {})", arms[0], arms[1]));
                continue;
            }
            // tx.send(e): data driven, valid asserted with the guard.
            if text.ends_with(")") && ts.len() >= 3 {
                let end = ts.len() - 3;
                if let (
                    TokenTree::Punct(dot),
                    TokenTree::Ident(m),
                    TokenTree::Group(g),
                ) = (&ts[end], &ts[end + 1], &ts[end + 2])
                {
                    if dot.as_char() == '.' && m.to_string() == "send" {
                        let Some(gd) = guard.clone() else {
                            return err(
                                ts[0].span(),
                                "send needs a wait before it",
                            );
                        };
                        let tx = match target_name(&ts[..end]) {
                            Ok(t) => t,
                            Err(m) => return err(ts[0].span(), &m),
                        };
                        let at: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let e = match tr(&at, &subst) {
                            Ok(e) => e,
                            Err(m) => return err(ts[0].span(), &m),
                        };
                        stmts.push(format!(
                            "S::Drive(T::Name(\"{tx}_data\".to_string()), {e})"
                        ));
                        stmts.push(format!(
                        "S::Drive(T::Name(\"{tx}_valid\".to_string()), {gd})"
                    ));
                        continue;
                    }
                }
            }
            if ts.len() >= 3 {
                let end = ts.len() - 3;
                if let (
                    TokenTree::Punct(dot),
                    TokenTree::Ident(m),
                    TokenTree::Group(g),
                ) = (&ts[end], &ts[end + 1], &ts[end + 2])
                {
                    if dot.as_char() == '.' && m.to_string() == "set" {
                        let target = match target_name(&ts[..end]) {
                            Ok(t) => t,
                            Err(m) => return err(ts[0].span(), &m),
                        };
                        let at: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let e = match tr(&at, &subst) {
                            Ok(e) => e,
                            Err(m) => return err(ts[0].span(), &m),
                        };
                        stmts.push(format!(
                            "S::Drive(T::Name(\"{target}\".to_string()), {e})"
                        ));
                        continue;
                    }
                }
            }
            return err(ts[0].span(), &format!("cannot lower `{text}`"));
        }
        if clock.is_empty() {
            return err(
                body.span(),
                "a loop must start by waiting for an edge",
            );
        }
        procs.push(format!(
            "::txhdl::netlist::Process {{ clock: {clock}, falling: {falling}, \
         body: vec![{}] }}",
            stmts.join(",\n")
        ));
    }
    let generated_text = format!(
        "impl{generics} {unit} {{\n\
         /// This unit as `#[lower]` read it from `run`; `.verilog()` and\n\
         /// `.vhdl()` render it.\n\
         #[allow(unused_variables, clippy::all)]\n\
         pub fn lowered(name: &str) -> ::txhdl::netlist::Lowered {{\n\
         use ::txhdl::netlist::{{Expr as E, Stmt as S, Target as T}};\n\
         ::txhdl::netlist::Lowered {{\n\
         name: name.to_string(),\n\
         fields: <Self as ::txhdl::netlist::Fields>::fields(),\n\
         ports: vec![{ports}],\n\
         wires: vec![{wires}],\n\
         procs: vec![{procs}],\n\
         init: Vec::new(),\n\
         }}\n}}\n\
         /// The Verilog of this unit.\n\
         pub fn verilog(name: &str) -> String {{\n\
         Self::lowered(name).verilog() }}\n\
         /// The VHDL of this unit.\n\
         pub fn vhdl(name: &str) -> String {{ Self::lowered(name).vhdl() }}\n\
         }}",
        ports = ports.join(", "),
        wires = wires
            .iter()
            .map(|(n, e)| format!("(\"{n}\".to_string(), {e})"))
            .collect::<Vec<_>>()
            .join(",\n"),
        procs = procs.join(",\n"),
    );
    if let Ok(dir) = std::env::var("TXHDL_MACRO_DUMP") {
        let _ = std::fs::write(
            format!("{dir}/lower_{}.rs", unit.replace(' ', "")),
            &generated_text,
        );
    }
    let generated: TokenStream = generated_text.parse().unwrap();
    let mut out = item;
    out.extend(generated);
    out
}
