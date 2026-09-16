// SPDX-License-Identifier: Apache-2.0
//! The macros of the runtime: four derives, `interface!`, `when!`,
//! `case!` and `#[lower]`, on a unit's impl and on a function it
//! inlines. Written
//! against `proc_macro` alone, without syn or quote, because the
//! grammars are small and a crate registry would be the larger cost.
extern crate proc_macro;
use proc_macro::{
    Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree,
};

fn marker(input: TokenStream, trait_path: &str) -> TokenStream {
    let item = parse_item(input);
    format!(
        "impl{b} {trait_path} for {n}{a} {{}}",
        b = item.bounds,
        n = item.name,
        a = item.args
    )
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
        let layout = names
            .iter()
            .zip(&types)
            .map(|(n, t)| {
                format!("(\"{n}\", <{t} as ::txhdl::types::Value>::WIDTH),")
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "impl{b} ::txhdl::types::Value for {n}{a} {{\n\
             const WIDTH: usize = {width};\n\
             fn vcd(self) -> String {{\n\
             let mut s = String::new(); {parts} s }}\n\
             fn parts(self) -> Vec<::txhdl::types::Part> {{\n\
             vec![{fields}] }}\n\
             fn layout() -> Vec<(&'static str, usize)> {{\n\
             vec![{layout}] }}\n}}",
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
// with!

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

/// One entry of a `with!` block: a drive of a field, under the
/// predicates of the groups around it and its own, if it has one; or
/// a group of entries under a predicate, with the entries under its
/// failure.
enum Entry {
    Drive {
        pred: Option<Vec<TokenTree>>,
        path: Vec<TokenTree>,
        value: Vec<TokenTree>,
    },
    Group {
        pred: Vec<TokenTree>,
        then: Vec<Entry>,
        otherwise: Vec<Entry>,
    },
}

/// The entries of a `with!` block, `field: value`, `c ? field: value`
/// and `c ? { .. } else { .. }`, separated by commas.
fn entries(g: &Group) -> Result<Vec<Entry>, (Span, String)> {
    let mut out = Vec::new();
    for part in split_commas(g) {
        let mask = turbofish(&part);
        let at = |c: char| {
            part.iter().enumerate().position(|(i, t)| {
                !mask[i]
                    && matches!(t, TokenTree::Punct(p)
                        if p.as_char() == c && p.spacing() == Spacing::Alone)
            })
        };
        let span = part[0].span();
        let (pred, rest) = match at('?') {
            Some(q) => (Some(part[..q].to_vec()), &part[q + 1..]),
            None => (None, &part[..]),
        };
        if pred.as_ref().is_some_and(|p| p.is_empty()) {
            return Err((span, "`?` needs a condition before it".into()));
        }
        match rest.first() {
            Some(TokenTree::Group(then))
                if then.delimiter() == Delimiter::Brace =>
            {
                let Some(pred) = pred else {
                    return Err((span, "a group needs `c ?` before it".into()));
                };
                let otherwise =
                    match (rest.get(1), rest.get(2)) {
                        (Some(e), Some(TokenTree::Group(g)))
                            if is_ident(e, "else")
                                && g.delimiter() == Delimiter::Brace
                                && rest.len() == 3 =>
                        {
                            entries(g)?
                        }
                        (None, _) => Vec::new(),
                        _ => return Err((
                            span,
                            "expected `c ? { .. }` or `c ? { .. } else { .. }`"
                                .into(),
                        )),
                    };
                out.push(Entry::Group {
                    pred,
                    then: entries(then)?,
                    otherwise,
                });
            }
            Some(_) => {
                let rmask = turbofish(rest);
                let colon = rest.iter().enumerate().position(|(i, t)| {
                    !rmask[i]
                        && matches!(t, TokenTree::Punct(p)
                            if p.as_char() == ':'
                                && p.spacing() == Spacing::Alone)
                });
                let Some(c) = colon.filter(|&c| c > 0 && c + 1 < rest.len())
                else {
                    return Err((
                        span,
                        "expected `field: value`, `c ? field: value` or \
                         `c ? { .. }`"
                            .into(),
                    ));
                };
                out.push(Entry::Drive {
                    pred,
                    path: rest[..c].to_vec(),
                    value: rest[c + 1..].to_vec(),
                });
            }
            None => return Err((span, "an entry needs a drive".into())),
        }
    }
    Ok(out)
}

/// `with!(target <= { .. })`: the target's tokens and the block.
fn with_parts(
    input: TokenStream,
) -> Result<(Vec<TokenTree>, Group), (Span, String)> {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let span = toks.first().map(|t| t.span()).unwrap_or(Span::call_site());
    let Some((lhs, rhs)) = split_becomes(toks.into_iter().collect()) else {
        return Err((span, "expected `with!(self <= { .. })`".into()));
    };
    let lhs: Vec<TokenTree> = lhs.into_iter().collect();
    let rhs: Vec<TokenTree> = rhs.into_iter().collect();
    match (lhs.is_empty(), rhs.as_slice()) {
        (false, [TokenTree::Group(g)]) if g.delimiter() == Delimiter::Brace => {
            Ok((lhs, g.clone()))
        }
        _ => Err((span, "expected `with!(self <= { .. })`".into())),
    }
}

/// The Rust of a `with!` block: a drive per entry, `set` with no
/// predicate over it and `set_if` under the predicates in force; a
/// group's predicate is taken once, as a bit, and its failure is the
/// predicate of the entries after `else`.
fn with_rust(
    target: &str,
    es: &[Entry],
    preds: &[String],
    out: &mut String,
    n: &mut usize,
) {
    for e in es {
        match e {
            Entry::Drive { pred, path, value } => {
                let mut ps = preds.to_vec();
                if let Some(p) = pred {
                    ps.push(format!(
                        "::core::convert::Into::<::txhdl::types::Bit>::into({})",
                        text_of(p)
                    ));
                }
                let path = text_of(path);
                let value = text_of(value);
                if ps.is_empty() {
                    out.push_str(&format!("{target}.{path}.set({value});\n"));
                } else {
                    out.push_str(&format!(
                        "{target}.{path}.set_if({}, {value});\n",
                        ps.join(" & ")
                    ));
                }
            }
            Entry::Group {
                pred,
                then,
                otherwise,
            } => {
                let c = format!("__w{n}");
                *n += 1;
                out.push_str(&format!(
                    "let {c}: ::txhdl::types::Bit = \
                     ::core::convert::Into::into({});\n",
                    text_of(pred)
                ));
                let mut yes = preds.to_vec();
                yes.push(c.clone());
                with_rust(target, then, &yes, out, n);
                let mut no = preds.to_vec();
                no.push(format!("!{c}"));
                with_rust(target, otherwise, &no, out, n);
            }
        }
    }
}

/// `with!(self <= { a: x, m.at(i): v, c ? b: y, c ? { .. } else { .. } })`
///
/// The drives of one struct, its name written once. Each entry is
/// `field: value`, a drive, the field becoming the value at the end of
/// the step; `c ? field: value` is a drive under a condition, and
/// `c ? { entries } else { entries }` a group of them under one, with
/// the entries under its failure. Entries apply in order, the last
/// drive of a field winning, as statements do. A condition is a
/// `bool` or a `Bit`. Both arms of a group exist in the hardware at
/// once and the condition selects, so nothing branches. Procedural,
/// since an `expr` fragment cannot be followed by `?` or `<=`.
#[proc_macro]
pub fn with(input: TokenStream) -> TokenStream {
    let (target, block) = match with_parts(input) {
        Ok(x) => x,
        Err((s, m)) => return err(s, &m),
    };
    let es = match entries(&block) {
        Ok(e) => e,
        Err((s, m)) => return err(s, &m),
    };
    let mut out = String::from("{\n");
    let mut n = 0;
    with_rust(&text_of(&target), &es, &[], &mut out, &mut n);
    out.push('}');
    out.parse().unwrap()
}

/// `when!(cond => target { .. } else { .. })`: the condition, the
/// target's tokens, the group and the group after `else`, if any.
fn when_parts(
    input: TokenStream,
) -> Result<
    (Vec<TokenTree>, Vec<TokenTree>, Group, Option<Group>),
    (Span, String),
> {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let span = toks.first().map(|t| t.span()).unwrap_or(Span::call_site());
    let bad = || {
        (
            span,
            "expected `when!(c => self { .. } else { .. })`".into(),
        )
    };
    let Some((cond, i)) = up_to_arrow(&toks, 0) else {
        return Err(bad());
    };
    let cond: Vec<TokenTree> = cond.into_iter().collect();
    let brace = |t: &TokenTree| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace);
    let Some(b) = toks[i..].iter().position(brace) else {
        return Err(bad());
    };
    let target = toks[i..i + b].to_vec();
    if cond.is_empty() || target.is_empty() {
        return Err(bad());
    }
    let TokenTree::Group(then) = &toks[i + b] else {
        unreachable!()
    };
    let otherwise = match (toks.get(i + b + 1), toks.get(i + b + 2)) {
        (Some(e), Some(TokenTree::Group(g)))
            if is_ident(e, "else")
                && g.delimiter() == Delimiter::Brace
                && toks.len() == i + b + 3 =>
        {
            Some(g.clone())
        }
        (None, _) => None,
        _ => return Err(bad()),
    };
    Ok((cond, target, then.clone(), otherwise))
}

/// `when!(c => self { a: x, m.at(i): v } else { a: 0 })`
///
/// A group of `with!`, the condition written first: the drives of
/// one struct under a condition, and under its failure after `else`.
/// The entries are `with!`'s, `field: value`, a nested `c ? ..` or a
/// group. Both arms exist in the hardware at once and the condition
/// selects, so nothing branches, which is why the name is `when` and
/// not `if`. `with!(self <= { c ? { .. } else { .. } })` is the same
/// block with the struct first.
#[proc_macro]
pub fn when(input: TokenStream) -> TokenStream {
    let (cond, target, then, otherwise) = match when_parts(input) {
        Ok(x) => x,
        Err((s, m)) => return err(s, &m),
    };
    let group = match (entries(&then), otherwise.as_ref().map(entries)) {
        (Ok(then), None) => Entry::Group {
            pred: cond,
            then,
            otherwise: Vec::new(),
        },
        (Ok(then), Some(Ok(otherwise))) => Entry::Group {
            pred: cond,
            then,
            otherwise,
        },
        (Err((s, m)), _) | (_, Some(Err((s, m)))) => return err(s, &m),
    };
    let mut out = String::from("{\n");
    let mut n = 0;
    with_rust(&text_of(&target), &[group], &[], &mut out, &mut n);
    out.push('}');
    out.parse().unwrap()
}

// ---------------------------------------------------------------------
// station!

/// The text of a reservation station of `n` inputs, `name`: a line
/// struct, the station struct and its lowered `run`. Every wire of
/// the step is named for what it is, so the netlist reads as the rule.
fn station_text(name: &str, n: usize) -> String {
    let idx: Vec<usize> = (0..n).collect();
    let each = |f: &dyn Fn(usize) -> String, sep: &str| -> String {
        idx.iter().map(|&i| f(i)).collect::<Vec<_>>().join(sep)
    };
    let lname = format!("Line{n}");
    let module = format!("station{n}");
    // The type parameters, one per line, so a header stays under
    // eighty columns at any arity.
    let tparams = each(&|i| format!("    T{i}: Transaction + Value,"), "\n");
    let targs = each(&|i| format!("T{i}"), ", ");
    let line_fields = each(&|i| format!("    pub v{i}: T{i},"), "\n");
    let state = each(
        &|i| {
            format!(
                "    /// Input {i}: its cell of every line, and which are held.\n\
                 \x20   pub mem{i}: Mem<T{i}, L>,\n\
                 \x20   pub occ{i}: Reg<U<L>>,"
            )
        },
        "\n",
    );
    let in_names = each(&|i| format!("in{i}"), ", ");
    let in_types = each(&|i| format!("Rx<Tagged<TB, T{i}>>"), ", ");
    let reads = each(
        &|i| {
            format!(
                "            let offered{i} = in{i}.peek().is_some();\n\
                 \x20           let head{i} = in{i}.head();\n\
                 \x20           let tag{i} = head{i}.tag;\n\
                 \x20           let value{i} = head{i}.value;\n\
                 \x20           let cell_free{i} =\n\
                 \x20               !self.occ{i}.get().bit(tag{i}.raw() as usize);"
            )
        },
        "\n",
    );
    // Taking input i completes its line when every other input's
    // cell of that line holds, or that input offers the same tag now
    // and its cell is free.
    let completes = each(
        &|i| {
            let others = idx
                .iter()
                .filter(|&&j| j != i)
                .map(|&j| {
                    format!(
                        "(self.occ{j}.get().bit(tag{i}.raw() as usize)\n\
                         \x20                   | (offered{j} & cell_free{j} \
                         & (tag{j} == tag{i})))"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n                & ");
            format!(
                "            let completes{i} = offered{i} & cell_free{i}\n\
                 \x20               & {others};"
            )
        },
        "\n",
    );
    let any = each(&|i| format!("completes{i}"), " | ");
    // The line sent is the completing input's of lowest index.
    let mut line_tag = format!("tag{}", n - 1);
    for &i in idx.iter().rev().skip(1) {
        line_tag = format!("mux(completes{i}, tag{i}, {line_tag})");
    }
    let takes = each(
        &|i| {
            format!(
                "            let take{i} = offered{i} & cell_free{i}\n\
                 \x20               & (!completes{i} | (send_line & (tag{i} == line_tag)));\n\
                 \x20           let _ = in{i}.recv_if(take{i});\n\
                 \x20           let cell_bit{i} =\n\
                 \x20               U::<L>::from(1u8) << (tag{i}.raw() as usize);"
            )
        },
        "\n",
    );
    let drives = each(
        &|i| {
            format!(
                "                take{i} ? mem{i}.at(tag{i}): value{i},\n\
                 \x20               occ{i}: (self.occ{i}.get()\n\
                 \x20                   | mux(take{i}, cell_bit{i}, U::<L>::from(0u8)))\n\
                 \x20                   & !line_clear,"
            )
        },
        "\n",
    );
    let outs = each(
        &|i| {
            format!(
                "            let out{i} = mux(\n\
                 \x20               self.occ{i}.get().bit(line_tag.raw() as usize),\n\
                 \x20               self.mem{i}.read(line_tag),\n\
                 \x20               value{i},\n\
                 \x20           );"
            )
        },
        "\n",
    );
    let line_lit = each(&|i| format!("v{i}: out{i}"), ", ");
    format!(
        "pub mod {module} {{\n\
         use super::Tagged;\n\
         use ::txhdl::comp::{{mux, Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit}};\n\
         use ::txhdl::types::{{Transaction, U, Value}};\n\
         use ::txhdl::{{lower, with, Trace}};\n\
         \n\
         /// A complete line of {n}: the tag and one value per input.\n\
         #[derive(::txhdl::Transaction, ::txhdl::Value, Clone, Copy, Default)]\n\
         pub struct {lname}<\n    const TB: usize,\n{tparams}\n> {{\n\
         \x20   pub tag: U<TB>,\n\
         {line_fields}\n\
         }}\n\
         \n\
         /// A reservation station of {n} inputs, each an `Rx<Tagged<TB, T>>`\n\
         /// of its own value type, and one output, a `Tx<{lname}>`. A line\n\
         /// per tag value, `L = 1 << TB` of them, stated; an input lands in\n\
         /// its cell of the line its tag names, and a line whose every cell\n\
         /// holds is sent as its tag and its values, at most one line per\n\
         /// cycle, the completing input of lowest index choosing when two\n\
         /// complete at once; the value that completes a line goes straight\n\
         /// through, the rest come from their cells. An input's `ready` is\n\
         /// its take: its cell is free, and taking it either completes\n\
         /// nothing or completes the line sent this cycle, so an input never\n\
         /// waits on another's cell, only on its own or on the output's\n\
         /// room. The tag has two bits at least. Written in the lowered\n\
         /// subset, so it is a netlist too.\n\
         // begin{{state}}\n\
         #[derive(Trace, Default)]\n\
         pub struct {name}<\n    const TB: usize,\n    const L: usize,\n{tparams}\n> {{\n\
         {state}\n\
         }}\n\
         // end{{state}}\n\
         \n\
         // begin{{ports}}\n\
         #[lower]\n\
         impl<\n    const TB: usize,\n    const L: usize,\n{tparams}\n> Unit\n\
         \x20   for {name}<TB, L, {targs}>\n\
         {{\n\
         \x20   async fn run(\n\
         \x20       &mut self,\n\
         \x20       ({in_names}): ({in_types}),\n\
         \x20       out: Tx<{lname}<TB, {targs}>>,\n\
         \x20   ) {{\n\
         \x20       loop {{\n\
         \x20           DefaultClock::rising().await;\n\
         // end{{ports}}\n\
         // begin{{reads}}\n\
         \x20           // What each input offers, and whether its cell is free.\n\
         {reads}\n\
         // end{{reads}}\n\
         // begin{{completes}}\n\
         \x20           // Taking an input completes its line when every other\n\
         \x20           // cell of that line holds, or is offered now.\n\
         {completes}\n\
         // end{{completes}}\n\
         // begin{{takes}}\n\
         \x20           let any_complete = {any};\n\
         \x20           // The line sent: the completing input's of lowest index.\n\
         \x20           let line_tag = {line_tag};\n\
         \x20           let send_line = any_complete & out.ready();\n\
         \x20           // An input is taken when its cell is free and it\n\
         \x20           // completes nothing, or completes the line being sent.\n\
         {takes}\n\
         // end{{takes}}\n\
         // begin{{drives}}\n\
         \x20           let line_clear = mux(\n\
         \x20               send_line,\n\
         \x20               U::<L>::from(1u8) << (line_tag.raw() as usize),\n\
         \x20               U::<L>::from(0u8),\n\
         \x20           );\n\
         \x20           with!(self <= {{\n\
         {drives}\n\
         \x20           }});\n\
         // end{{drives}}\n\
         // begin{{outputs}}\n\
         \x20           // The line's values: from the cell if held, else\n\
         \x20           // straight from the input.\n\
         {outs}\n\
         \x20           if send_line.to_bool() {{\n\
         \x20               out.send({lname} {{ tag: line_tag, {line_lit} }});\n\
         \x20           }}\n\
         // end{{outputs}}\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n\
         }}\n\
         pub use {module}::{{{lname}, {name}}};\n"
    )
}

/// `station!(Station3, 3)`: a reservation station of three inputs,
/// named, with its line struct `Line3` beside it. The station is a
/// unit of `Rx<Tagged<TB, T_i>>` inputs and a `Tx<Line3<TB, T_0..>>`
/// output, generic over the tag width `TB`, the line count `L`, which
/// is `1 << TB` stated, and the inputs' value types. Written out per
/// arity, since the lowering reads a body and not a loop over inputs;
/// `TXHDL_MACRO_DUMP` names a directory to write the text to.
#[proc_macro]
pub fn station(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let (name, n) = match toks.as_slice() {
        [TokenTree::Ident(name), TokenTree::Punct(c), TokenTree::Literal(n)]
            if c.as_char() == ',' =>
        {
            (name.to_string(), n.to_string().parse::<usize>().ok())
        }
        _ => return err(Span::call_site(), "expected `station!(Name, N)`"),
    };
    let Some(n) = n.filter(|n| (2..=16).contains(n)) else {
        return err(toks[2].span(), "a station has from two to sixteen inputs");
    };
    let text = station_text(&name, n);
    if let Ok(dir) = std::env::var("TXHDL_MACRO_DUMP") {
        let _ = std::fs::write(format!("{dir}/station_{name}.rs"), &text);
    }
    // The text keeps itself, as `SOURCE` in the module, so a document
    // can show what was written without a hand-typed copy.
    let module = format!("station{n}");
    let with_source = text.replacen(
        &format!("pub mod {module} {{\n"),
        &format!(
            "pub mod {module} {{\n/// The text of this module, as `station!` wrote it.\n\
             pub const SOURCE: &str = r####\"{text}\"####;\n"
        ),
        1,
    );
    with_source.parse().unwrap()
}

// ---------------------------------------------------------------------
// router!

/// The text of an AXI router of `n` peripherals, `name`: the struct
/// and its lowered `run`. The host's five channels come in and five
/// go out per peripheral, chosen by a base and a mask each, stated as
/// const parameters. Every wire of the step is named for what it is,
/// so the netlist reads as the rule.
fn router_text(name: &str, n: usize) -> String {
    let idx: Vec<usize> = (0..n).collect();
    let each = |f: &dyn Fn(usize) -> String, sep: &str| -> String {
        idx.iter().map(|&i| f(i)).collect::<Vec<_>>().join(sep)
    };
    let module = format!("router{n}");
    // A peripheral's range is a base and a mask, so a design states
    // its address map where it names the type.
    let ranges = each(
        &|i| format!("    const BASE{i}: usize,\n    const MASK{i}: usize,"),
        "\n",
    );
    let rargs = each(&|i| format!("        BASE{i},\n        MASK{i},"), "\n");
    let in_names = format!(
        "\n            aw,\n            ar,\n            w,\n{}",
        each(&|i| format!("            b{i},\n            r{i},"), "\n")
    );
    let in_types = format!(
        "Rx<Aw<A, I>>,\n        Rx<Ar<A, I>>,\n        Rx<W<D, S>>,\n{}",
        each(
            &|_| "        Rx<B<I>>,\n        Rx<R<D, I>>,".to_string(),
            "\n"
        )
    );
    let out_names = format!(
        "\n{}\n            b,\n            r,",
        each(
            &|i| format!(
                "            aw{i},\n            ar{i},\n            w{i},"
            ),
            "\n"
        )
    );
    let out_types = format!(
        "{}\n        Tx<B<I>>,\n        Tx<R<D, I>>,",
        each(
            &|_| "        Tx<Aw<A, I>>,\n        Tx<Ar<A, I>>,\n        \
                  Tx<W<D, S>>,"
                .to_string(),
            "\n"
        )
    );
    // A choice over the peripherals, as a wire each rather than one
    // nested expression: the netlist then names every step, and no
    // line grows with the count. `tail` is what is chosen when no
    // peripheral is, or nothing when the highest is the last resort.
    // The choice is `{name}0`, since each wire falls back to the next.
    let chain = |name: &str,
                 pred: &dyn Fn(usize) -> String,
                 val: &dyn Fn(usize) -> String,
                 tail: Option<&str>|
     -> String {
        let mut lines: Vec<String> = Vec::new();
        let top = n - 1;
        match tail {
            Some(t) => lines.push(format!(
                "            let {name}{top} =\n                \
                 mux({}, {}, {t});",
                pred(top),
                val(top)
            )),
            None => lines
                .push(format!("            let {name}{top} = {};", val(top))),
        }
        for i in (0..top).rev() {
            lines.push(format!(
                "            let {name}{i} =\n                \
                 mux({}, {}, {name}{});",
                pred(i),
                val(i),
                i + 1
            ));
        }
        lines.join("\n")
    };
    // The same, for a one-hot of `n` bits.
    let onehot = |name: &str, pred: &dyn Fn(usize) -> String| -> String {
        chain(
            name,
            pred,
            &|i| format!("U::<{n}>::from({}u32)", 1u32 << i),
            Some(&format!("U::<{n}>::from(0u8)")),
        )
    };
    // The address decode, one line per peripheral on each phase. The
    // compare is on the raw address so that a base and a mask may be
    // plain integers; the lowering reads `raw` as the value itself.
    let decode = |ph: &str, h: &str| -> String {
        each(
            &|i| {
                format!(
                    "            let {ph}_hit{i} =\n                \
                     ({h}.addr.raw() as usize & MASK{i}) == BASE{i};"
                )
            },
            "\n",
        )
    };
    let none_of = |ph: &str| -> String {
        each(&|i| format!("!{ph}_hit{i}"), "\n                & ")
    };
    let state = format!(
        "    /// A write burst's beats are going out; a second address\n\
         \x20   /// phase waits, since AXI4 puts no identifier on `w`.\n\
         \x20   pub wbusy: Reg<Bit>,\n\
         \x20   /// Where this write burst's beats go, one bit per\n\
         \x20   /// peripheral; none set means they are being swallowed.\n\
         \x20   pub wsel: Reg<U<{n}>>,\n\
         \x20   /// This write burst decoded to no peripheral.\n\
         \x20   pub whole: Reg<Bit>,\n\
         \x20   /// Its identifier, to answer when its last beat has gone.\n\
         \x20   pub wid: Reg<U<I>>,\n\
         \x20   /// A write response the router owes for a decode error.\n\
         \x20   pub berr: Reg<Bit>,\n\
         \x20   pub berr_id: Reg<U<I>>,\n\
         \x20   /// A read burst is going up; its beats stay together.\n\
         \x20   pub rbusy: Reg<Bit>,\n\
         \x20   pub rsel: Reg<U<{n}>>,\n\
         \x20   /// A read burst the router owes for a decode error, and\n\
         \x20   /// the beats of it still to send.\n\
         \x20   pub rerr: Reg<Bit>,\n\
         \x20   pub rerr_id: Reg<U<I>>,\n\
         \x20   pub rerr_left: Reg<U<9>>,"
    );
    let send_if = |cond: &str, port: &str, val: &str| -> String {
        format!(
            "            if ({cond}).to_bool() {{\n                \
             {port}.send({val});\n            }}"
        )
    };
    let aw_sends = each(
        &|i| send_if(&format!("take_aw & aw_hit{i}"), &format!("aw{i}"), "awh"),
        "\n",
    );
    let ar_sends = each(
        &|i| send_if(&format!("take_ar & ar_hit{i}"), &format!("ar{i}"), "arh"),
        "\n",
    );
    let w_sends = each(
        &|i| {
            send_if(
                &format!("w_go & self.wsel.get().bit({i})"),
                &format!("w{i}"),
                "wh",
            )
        },
        "\n",
    );
    let heads = each(
        &|i| {
            format!(
                "            let b{i}h = b{i}.head();\n            \
                 let r{i}h = r{i}.head();\n            \
                 let off_b{i} = b{i}.peek().is_some();\n            \
                 let off_r{i} = r{i}.peek().is_some();"
            )
        },
        "\n",
    );
    // A peripheral wins the merge when it offers and no lower one
    // does. The waiting term is a wire of its own, so a line stays
    // short however many peripherals there are, and the netlist names
    // what each one is waiting for.
    let waits = |kind: &str| -> String {
        each(
            &|i| {
                if i == 0 {
                    format!("            let {kind}_first0 = Bit::One;")
                } else {
                    format!(
                        "            let {kind}_first{i} =\n                \
                         {kind}_first{} & !off_{kind}{};",
                        i - 1,
                        i - 1
                    )
                }
            },
            "\n",
        )
    };
    let sel_r = format!(
        "{}\n{}",
        waits("r"),
        each(
            &|i| {
                format!(
                    "            let sel_r{i} = mux(\n                \
                     self.rbusy,\n                \
                     self.rsel.get().bit({i}),\n                \
                     !err_turn & r_first{i} & off_r{i},\n            );"
                )
            },
            "\n",
        )
    );
    let sel_b = format!(
        "{}\n{}",
        waits("b"),
        each(
            &|i| {
                format!(
                    "            let sel_b{i} =\n                \
                     !berr_turn & b_first{i} & off_b{i};"
                )
            },
            "\n",
        )
    );
    let takes_r = each(
        &|i| format!("            let _ = r{i}.recv_if(sel_r{i} & r_room);"),
        "\n",
    );
    let takes_b = each(
        &|i| format!("            let _ = b{i}.recv_if(sel_b{i} & b_room);"),
        "\n",
    );
    // An `or` over the peripherals, wrapped so no line runs long.
    let any = |p: &dyn Fn(usize) -> String| -> String {
        each(&|i| p(i), "\n                | ")
    };
    format!(
        "pub mod {module} {{\n\
         use crate::bus::axi::{{Ar, Aw, Resp, B, R, W}};\n\
         use ::txhdl::comp::{{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit}};\n\
         use ::txhdl::types::{{Bit, U}};\n\
         use ::txhdl::{{lower, with, Trace}};\n\
         \n\
         /// An AXI4 router of one host and {n} peripherals. The host's\n\
         /// five channels come in; five go out per peripheral, and a\n\
         /// peripheral's range is its `BASE` and `MASK`, matched on the\n\
         /// address of each phase. A burst whose address is no\n\
         /// peripheral's is answered `DecErr` by the router itself\n\
         /// rather than dropped. Write beats follow the address phase\n\
         /// they belong to, and a second write waits for the first\n\
         /// burst's beats, since AXI4 puts no identifier on `w`; a read\n\
         /// burst's beats stay together, so `last` still means what it\n\
         /// says. Responses merge back to the host, the lowest\n\
         /// peripheral first when two answer at once. Written in the\n\
         /// lowered subset, so it is a netlist too.\n\
         // begin{{state}}\n\
         #[derive(Trace, Default)]\n\
         pub struct {name}<\n\
         \x20   const A: usize,\n\
         \x20   const D: usize,\n\
         \x20   const S: usize,\n\
         \x20   const I: usize,\n\
         {ranges}\n\
         > {{\n\
         {state}\n\
         }}\n\
         // end{{state}}\n\
         \n\
         // begin{{ports}}\n\
         #[lower]\n\
         impl<\n\
         \x20   const A: usize,\n\
         \x20   const D: usize,\n\
         \x20   const S: usize,\n\
         \x20   const I: usize,\n\
         {ranges}\n\
         > Unit\n\
         \x20   for {name}<\n\
         \x20       A,\n\
         \x20       D,\n\
         \x20       S,\n\
         \x20       I,\n\
         {rargs}\n\
         \x20   >\n\
         {{\n\
         \x20   async fn run(\n\
         \x20       &mut self,\n\
         \x20       ({in_names}): (\n\
         {in_types}\n\
         \x20       ),\n\
         \x20       ({out_names}): (\n\
         {out_types}\n\
         \x20       ),\n\
         \x20   ) {{\n\
         \x20       loop {{\n\
         \x20           DefaultClock::rising().await;\n\
         // end{{ports}}\n\
         // begin{{decode}}\n\
         \x20           // Where the write address at the head would go,\n\
         \x20           // and whether that peripheral has room for it.\n\
         \x20           let aw_off = aw.peek().is_some();\n\
         \x20           let awh = aw.head();\n\
         {aw_decode}\n\
         \x20           let aw_hole = {aw_none};\n\
         {aw_room}\n\
         \x20           let take_aw = aw_off & !self.wbusy & aw_room0;\n\
         \x20           let _ = aw.recv_if(!self.wbusy & aw_room0);\n\
         {aw_sends}\n\
         \x20           // The read address phase, the same way. Several\n\
         \x20           // reads may be outstanding, since each carries\n\
         \x20           // its identifier.\n\
         \x20           let ar_off = ar.peek().is_some();\n\
         \x20           let arh = ar.head();\n\
         {ar_decode}\n\
         \x20           let ar_hole = {ar_none};\n\
         {ar_room}\n\
         \x20           let take_ar = ar_off & ar_room0;\n\
         \x20           let _ = ar.recv_if(ar_room0);\n\
         {ar_sends}\n\
         // end{{decode}}\n\
         // begin{{beats}}\n\
         \x20           // The write beats go where this burst's address\n\
         \x20           // went; a burst to a hole is swallowed here and\n\
         \x20           // answered when its last beat has gone.\n\
         \x20           let w_off = w.peek().is_some();\n\
         \x20           let wh = w.head();\n\
         {w_room}\n\
         \x20           let w_go = self.wbusy & w_off & w_room0;\n\
         \x20           let _ = w.recv_if(self.wbusy & w_room0);\n\
         {w_sends}\n\
         \x20           let w_done = w_go & wh.last;\n\
         // end{{beats}}\n\
         // begin{{merge}}\n\
         {heads}\n\
         \x20           // The read beats going up: the burst already\n\
         \x20           // going stays chosen, so its beats stay together;\n\
         \x20           // a decode error's own burst goes before a new one.\n\
         \x20           let r_room = r.ready();\n\
         \x20           let err_turn = self.rerr & !self.rbusy;\n\
         {sel_r}\n\
         {takes_r}\n\
         \x20           let r_from_per = {r_any};\n\
         {wsel}\n\
         {rsel}\n\
         \x20           let r_go = r_from_per & r_room;\n\
         \x20           let err_last = self.rerr_left == 1;\n\
         \x20           let err_go = err_turn & r_room;\n\
         {r_id}\n\
         {r_data}\n\
         {r_resp}\n\
         {r_last}\n\
         \x20           // The beat that goes up, from the chosen\n\
         \x20           // peripheral or from the router's own error.\n\
         \x20           let up_id = mux(err_turn, self.rerr_id.get(), sel_id0);\n\
         \x20           let up_data =\n\
         \x20               mux(err_turn, U::<D>::from(0u8), sel_data0);\n\
         \x20           let up_resp = mux(err_turn, Resp::DecErr, sel_resp0);\n\
         \x20           let per_last = sel_last0;\n\
         \x20           let up_last =\n\
         \x20               mux(err_turn, Bit::from(err_last), per_last);\n\
         \x20           if (err_go | r_go).to_bool() {{\n\
         \x20               r.send(R {{\n\
         \x20                   id: up_id,\n\
         \x20                   data: up_data,\n\
         \x20                   resp: up_resp,\n\
         \x20                   last: up_last,\n\
         \x20               }});\n\
         \x20           }}\n\
         \x20           // The write responses, the same choice, with the\n\
         \x20           // router's own decode error first.\n\
         \x20           let b_room = b.ready();\n\
         \x20           let berr_turn = self.berr;\n\
         {sel_b}\n\
         {takes_b}\n\
         \x20           let b_any = {b_any};\n\
         \x20           let b_go = b_any & b_room;\n\
         \x20           let berr_go = berr_turn & b_room;\n\
         {b_id}\n\
         {b_resp}\n\
         \x20           let upb_id =\n\
         \x20               mux(berr_turn, self.berr_id.get(), selb_id0);\n\
         \x20           let upb_resp =\n\
         \x20               mux(berr_turn, Resp::DecErr, selb_resp0);\n\
         \x20           if (b_go | berr_go).to_bool() {{\n\
         \x20               b.send(B {{ id: upb_id, resp: upb_resp }});\n\
         \x20           }}\n\
         // end{{merge}}\n\
         // begin{{drives}}\n\
         \x20           with!(self <= {{\n\
         \x20               take_aw ? {{\n\
         \x20                   wbusy: Bit::One,\n\
         \x20                   wsel: wsel0,\n\
         \x20                   whole: aw_hole,\n\
         \x20                   wid: awh.id,\n\
         \x20               }},\n\
         \x20               w_done ? {{ wbusy: Bit::Zero }},\n\
         \x20               (w_done & self.whole) ? {{\n\
         \x20                   berr: Bit::One,\n\
         \x20                   berr_id: self.wid,\n\
         \x20               }},\n\
         \x20               berr_go ? {{ berr: Bit::Zero }},\n\
         \x20               (take_ar & ar_hole) ? {{\n\
         \x20                   rerr: Bit::One,\n\
         \x20                   rerr_id: arh.id,\n\
         \x20                   rerr_left: arh.len.zext::<9>() + 1,\n\
         \x20               }},\n\
         \x20               err_go ? {{\n\
         \x20                   rerr_left: self.rerr_left.get() - 1,\n\
         \x20                   rerr: !err_last,\n\
         \x20               }},\n\
         \x20               r_go ? {{ rbusy: !per_last, rsel: rsel0 }},\n\
         \x20           }});\n\
         // end{{drives}}\n\
         \x20       }}\n\
         \x20   }}\n\
         }}\n\
         }}\n\
         pub use {module}::{name};\n",
        aw_decode = decode("aw", "awh"),
        ar_decode = decode("ar", "arh"),
        aw_none = none_of("aw"),
        ar_none = none_of("ar"),
        aw_room = chain(
            "aw_room",
            &|i| format!("aw_hit{i}"),
            &|i| format!("aw{i}.ready()"),
            Some("!self.berr")
        ),
        ar_room = chain(
            "ar_room",
            &|i| format!("ar_hit{i}"),
            &|i| format!("ar{i}.ready()"),
            Some("!self.rerr")
        ),
        w_room = chain(
            "w_room",
            &|i| format!("self.wsel.get().bit({i})"),
            &|i| format!("w{i}.ready()"),
            Some("Bit::One")
        ),
        r_any = any(&|i| format!("(sel_r{i} & off_r{i})")),
        b_any = any(&|i| format!("(sel_b{i} & off_b{i})")),
        r_id = chain(
            "sel_id",
            &|i| format!("sel_r{i}"),
            &|i| format!("r{i}h.id"),
            None
        ),
        r_data = chain(
            "sel_data",
            &|i| format!("sel_r{i}"),
            &|i| format!("r{i}h.data"),
            None
        ),
        r_resp = chain(
            "sel_resp",
            &|i| format!("sel_r{i}"),
            &|i| format!("r{i}h.resp"),
            None
        ),
        r_last = chain(
            "sel_last",
            &|i| format!("sel_r{i}"),
            &|i| format!("r{i}h.last"),
            None
        ),
        b_id = chain(
            "selb_id",
            &|i| format!("sel_b{i}"),
            &|i| format!("b{i}h.id"),
            None
        ),
        b_resp = chain(
            "selb_resp",
            &|i| format!("sel_b{i}"),
            &|i| format!("b{i}h.resp"),
            None
        ),
        wsel = onehot("wsel", &|i| format!("aw_hit{i}")),
        rsel = onehot("rsel", &|i| format!("sel_r{i}")),
    )
}

/// `router!(Router3, 3)`: an AXI4 router of one host and three
/// peripherals, named. The host's five channels come in and five go
/// out per peripheral; a peripheral's address range is a `BASE` and a
/// `MASK` const parameter, so a design states its address map where it
/// names the type. Written out per count, since the lowering reads a
/// body and not a loop over ports; `TXHDL_MACRO_DUMP` names a
/// directory to write the text to.
#[proc_macro]
pub fn router(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let (name, n) = match toks.as_slice() {
        [TokenTree::Ident(name), TokenTree::Punct(c), TokenTree::Literal(n)]
            if c.as_char() == ',' =>
        {
            (name.to_string(), n.to_string().parse::<usize>().ok())
        }
        _ => return err(Span::call_site(), "expected `router!(Name, N)`"),
    };
    let Some(n) = n.filter(|n| (2..=8).contains(n)) else {
        return err(
            toks[2].span(),
            "a router has from two to eight peripherals",
        );
    };
    let text = router_text(&name, n);
    if let Ok(dir) = std::env::var("TXHDL_MACRO_DUMP") {
        let _ = std::fs::write(format!("{dir}/router_{name}.rs"), &text);
    }
    let module = format!("router{n}");
    let with_source = text.replacen(
        &format!("pub mod {module} {{\n"),
        &format!(
            "pub mod {module} {{\n/// The text of this module, as `router!` wrote it.\n\
             pub const SOURCE: &str = r####\"{text}\"####;\n"
        ),
        1,
    );
    with_source.parse().unwrap()
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

/// A tuple of types split on its commas, a comma inside a type's
/// angle brackets, `Rx<Tagged<TB, T0>>`, being the type's own.
fn split_type_commas(g: &Group) -> Vec<Vec<TokenTree>> {
    let mut out = vec![Vec::new()];
    let mut depth = 0usize;
    for t in g.stream() {
        match &t {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => {
                depth = depth.saturating_sub(1)
            }
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => {
                out.push(Vec::new());
                continue;
            }
            _ => {}
        }
        out.last_mut().unwrap().push(t);
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
    format!("NlE::Name(\"{n}\".to_string())")
}

/// The Rust source of an `Expr::Bin`.
fn ebin(op: &str, a: &str, b: &str) -> String {
    format!("NlE::bin(\"{op}\", {a}, {b})")
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
                return Ok(format!("NlT::Word(\"{mem}\".to_string(), {a})"));
            }
        }
    }
    Ok(format!("NlT::Name(\"{}\".to_string())", target_name(ts)?))
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
/// A function under `#[lower]`: combinational, a few `let`s and a
/// value, plain Rust for the simulation and inlined at every call in
/// a lowered unit of the same file. Kept as text, not tokens: a
/// token is a handle into the compiler's bridge for one invocation
/// of the macro, and one kept past it hangs the compiler when it is
/// dropped.
#[derive(Clone)]
struct Helper {
    name: String,
    params: Vec<String>,
    lets: Vec<(String, String)>,
    value: String,
}

fn punct_at(ts: &[TokenTree], i: usize, c: char) -> bool {
    matches!(ts.get(i), Some(TokenTree::Punct(p)) if p.as_char() == c)
}

fn is_ident(t: &TokenTree, s: &str) -> bool {
    matches!(t, TokenTree::Ident(id) if id.to_string() == s)
}

/// Tokens as text, to be parsed again inside a later invocation.
/// The tokens as text that parses back to the same tokens: a punct
/// joined to the next, the first half of `>>` or `==`, gets no space.
fn text_of(ts: &[TokenTree]) -> String {
    let mut out = String::new();
    for t in ts {
        out.push_str(&t.to_string());
        let joint = matches!(t, TokenTree::Punct(p)
            if p.spacing() == proc_macro::Spacing::Joint);
        if !joint {
            out.push(' ');
        }
    }
    out
}

thread_local! {
    /// The functions of the file the unit being lowered is in.
    static HELPERS: std::cell::RefCell<Vec<Helper>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The ports of the unit being lowered, each with the text of its
    /// value's type, so a field of a port's value can be sliced out.
    static PTYPES: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A field of a port's value, `p.f`: the base must be a port's data,
/// and the field's bits are found from the value type's layout when
/// `lowered` runs, so a generic value's width is no obstacle.
fn field_of(base: &str, f: &str) -> Result<String, String> {
    let name = base
        .strip_prefix("NlE::Name(\"")
        .and_then(|s| s.strip_suffix("\".to_string())"))
        .ok_or_else(|| {
            format!("`.{f}` needs a port's value before it, not a computed one")
        })?;
    let port = name.strip_suffix("_data").unwrap_or(name);
    let ty = PTYPES.with(|p| {
        p.borrow()
            .iter()
            .find(|(n, _)| n == port)
            .map(|(_, t)| t.clone())
    });
    let Some(ty) = ty else {
        return Err(format!("`{port}` is not a port, so `.{f}` has no layout"));
    };
    Ok(format!("::txhdl::netlist::field::<{ty}>({base}, \"{f}\")"))
}

/// The functions under `#[lower]` in a file. The macro reads the file,
/// which is what `Span::local_file` names, since a proc macro sees one
/// item at a time and a function's body is not in the unit's. A
/// function is `fn name(a: A, ..) -> R { let x = e; ..; value }`, with
/// or without `pub` and generics; its parameters are taken by name.
fn find_helpers(file: Option<std::path::PathBuf>) -> Vec<Helper> {
    let Some(text) = file.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let Ok(stream) = text.parse::<TokenStream>() else {
        return Vec::new();
    };
    let ts: Vec<TokenTree> = stream.into_iter().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < ts.len() {
        let marked = matches!(&ts[i], TokenTree::Punct(p) if p.as_char() == '#')
            && matches!(&ts[i + 1], TokenTree::Group(g)
                if g.delimiter() == Delimiter::Bracket
                    && g.stream().to_string() == "lower");
        if !marked {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        if ts.get(j).is_some_and(|t| is_ident(t, "pub")) {
            j += 1;
            if matches!(ts.get(j), Some(TokenTree::Group(g))
                if g.delimiter() == Delimiter::Parenthesis)
            {
                j += 1;
            }
        }
        if !ts.get(j).is_some_and(|t| is_ident(t, "fn")) {
            i += 1;
            continue;
        }
        let Some(TokenTree::Ident(name)) = ts.get(j + 1) else {
            i += 1;
            continue;
        };
        j += 2;
        if matches!(ts.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '<')
        {
            let mut depth = 0;
            while j < ts.len() {
                match &ts[j] {
                    TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
                    TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
                    _ => {}
                }
                j += 1;
                if depth == 0 {
                    break;
                }
            }
        }
        let Some(TokenTree::Group(params)) = ts.get(j) else {
            i += 1;
            continue;
        };
        let params: Vec<String> = split_commas(params)
            .iter()
            .filter_map(|p| match p.first() {
                Some(TokenTree::Ident(n)) => Some(n.to_string()),
                _ => None,
            })
            .collect();
        let brace = |t: &TokenTree| {
            matches!(t, TokenTree::Group(g)
                if g.delimiter() == Delimiter::Brace)
        };
        let Some(k) = ts[j..].iter().position(brace) else {
            break;
        };
        let TokenTree::Group(body) = &ts[j + k] else {
            break;
        };
        let mut lets = Vec::new();
        let mut value = String::new();
        for st in statements(body) {
            let st: Vec<TokenTree> = st.into_iter().collect();
            if st.len() > 3 && is_ident(&st[0], "let") {
                lets.push((st[1].to_string(), text_of(&st[3..])));
            } else {
                value = text_of(&st);
            }
        }
        out.push(Helper {
            name: name.to_string(),
            params,
            lets,
            value,
        });
        i = j + k + 1;
    }
    out
}

/// A call of a function under `#[lower]`, inlined: its parameters
/// bound to the arguments, its `let`s to expressions of their own,
/// and its value the call's.
fn inline_helper(h: &Helper, args: &[String]) -> Result<String, String> {
    if args.len() != h.params.len() {
        return Err(format!("`{}` takes {} arguments", h.name, h.params.len()));
    }
    let mut s: Vec<(String, String)> =
        h.params.iter().cloned().zip(args.iter().cloned()).collect();
    let toks = |text: &str| -> Result<Vec<TokenTree>, String> {
        text.parse::<TokenStream>()
            .map(|t| t.into_iter().collect())
            .map_err(|_| format!("cannot parse `{text}` in `{}`", h.name))
    };
    for (n, e) in &h.lets {
        let v = tr(&toks(e)?, &s)?;
        s.push((n.clone(), v));
    }
    if h.value.is_empty() {
        return Err(format!("`{}` has no value", h.name));
    }
    tr(&toks(&h.value)?, &s)
}

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
    // The binary operators by precedence, loosest first, as Rust binds
    // them. The split is at the rightmost operator of the loosest level
    // present, which makes every level left-associative, as Rust's are.
    const LEVELS: &[&[&str]] = &[
        &["||"],
        &["&&"],
        &["==", "!=", "<=", ">=", "<", ">"],
        &["|"],
        &["^"],
        &["&"],
        &["<<", ">>"],
        &["+", "-"],
        &["/"],
    ];
    let joint = |t: &TokenTree| {
        matches!(t, TokenTree::Punct(p)
            if p.spacing() == proc_macro::Spacing::Joint)
    };
    for level in LEVELS {
        for i in (1..ts.len()).rev() {
            if mask[i] || joint(&ts[i - 1]) {
                continue;
            }
            for op in *level {
                let n = op.len();
                if i + n > ts.len() {
                    continue;
                }
                let here: String =
                    ts[i..i + n].iter().map(|t| t.to_string()).collect();
                let all_punct = ts[i..i + n]
                    .iter()
                    .all(|t| matches!(t, TokenTree::Punct(_)));
                // The operator's last character is not joined to what
                // follows, or this is the head of a longer operator.
                if !all_punct || here != *op || joint(&ts[i + n - 1]) {
                    continue;
                }
                let prev = ts[i - 1].to_string();
                let generic =
                    (*op == "<" || *op == ">") && (prev == ":" || prev == "U");
                if generic {
                    continue;
                }
                let l = tr(&ts[..i], subst)?;
                let r = tr(&ts[i + n..], subst)?;
                return Ok(ebin(op, &l, &r));
            }
        }
    }
    if let Some(p) = ts.iter().position(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "as"),
    ) {
        return tr(&ts[..p], subst);
    }
    if let TokenTree::Punct(p) = &ts[0] {
        if p.as_char() == '!' {
            return Ok(format!("NlE::Not(Box::new({}))", tr(&ts[1..], subst)?));
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
                            // A width the netlist needs is written: the
                            // macro sees tokens, not types, so it cannot
                            // infer one. The low operand of a `concat`
                            // is the exception, since its width is not
                            // needed.
                            let needed: &[usize] = match m.to_string().as_str()
                            {
                                "concat" => &[1],
                                "slice" => &[0, 1],
                                _ => &[0],
                            };
                            if needed
                                .iter()
                                .any(|&k| ks.get(k).is_some_and(|x| x == "_"))
                            {
                                return Err(format!(
                                    "`{}::<..>` needs its width written: the \
                                     lowering sees no types, so it cannot \
                                     infer one",
                                    m
                                ));
                            }
                            return Ok(match m.to_string().as_str() {
                                "slice" => format!(
                                    "NlE::Slice(Box::new({l}), {}, {})",
                                    ks[0], ks[1]
                                ),
                                "sext" => {
                                    format!(
                                        "NlE::Sext(Box::new({l}), {})",
                                        ks[0]
                                    )
                                }
                                "zext" | "resize" => {
                                    format!(
                                        "NlE::Zext(Box::new({l}), {})",
                                        ks[0]
                                    )
                                }
                                "concat" => format!(
                                    "NlE::Cat(Box::new({l}), Box::new({}))",
                                    tr(&args[0], subst)?
                                ),
                                // Both operands at the result's width,
                                // so the product is that wide in either
                                // target language.
                                "mul" => format!(
                                    "NlE::Bin(\"*\", \
                                     Box::new(NlE::Zext(Box::new({l}), {0})), \
                                     Box::new(NlE::Zext(Box::new({1}), {0})))",
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
                if matches!(
                    m.as_str(),
                    "peek" | "ready" | "recv" | "recv_if" | "head"
                ) {
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
                    "sra" => ebin(">>>", &l, &a[0]),
                    "lt_signed" => ebin("<s", &l, &a[0]),
                    "bit" | "read" => {
                        format!("NlE::Index(Box::new({l}), Box::new({}))", a[0])
                    }
                    // Conversions between a bit and a truth value, and
                    // a read, are the value itself.
                    "raw" | "to_bool" | "into" | "get" | "is_some" | "zext"
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
        // A name: a parameter of an inlined function, or a wire, a
        // register, a port; a name in capitals is a constant, `DIV`,
        // and is its value.
        [TokenTree::Ident(id)] => {
            let n = id.to_string();
            if let Some((_, v)) = subst.iter().rev().find(|(k, _)| *k == n) {
                return Ok(v.clone());
            }
            if n == "true" || n == "false" {
                let b = if n == "true" { 1 } else { 0 };
                return Ok(format!("NlE::Bits(1, \"{b}\".to_string())"));
            }
            let upper = n.chars().all(|c| {
                c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()
            });
            Ok(if upper && n.chars().any(|c| c.is_ascii_uppercase()) {
                format!("NlE::Num(({n}) as u128)")
            } else {
                ename(&n)
            })
        }
        [TokenTree::Literal(l)] => {
            Ok(format!("NlE::Num({})", l.to_string().replace('_', "")))
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
        // A field of a port's value, `p.tag`: a slice of the port's data.
        [base @ .., TokenTree::Punct(dot), TokenTree::Ident(f)]
            if dot.as_char() == '.' && !base.is_empty() =>
        {
            let b = tr(base, subst)?;
            field_of(&b, &f.to_string())
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
                    Some(a) => {
                        format!("NlE::Cat(Box::new({a}), Box::new({e}))")
                    }
                });
            }
            acc.ok_or_else(|| "an empty struct literal".to_string())
        }
        _ => {
            let last_group = matches!(ts.last(), Some(TokenTree::Group(_)));
            // A call, `f(args)` or `f::<K>(args)`: one of the runtime's
            // functions, or a function under `#[lower]` in this file,
            // which is inlined.
            let turbo = punct_at(ts, 1, ':')
                && punct_at(ts, 2, ':')
                && punct_at(ts, 3, '<');
            let call = match (ts.first(), ts.last()) {
                (Some(TokenTree::Ident(f)), Some(TokenTree::Group(g)))
                    if g.delimiter() == Delimiter::Parenthesis
                        && (ts.len() == 2 || turbo) =>
                {
                    Some((f.to_string(), g.clone()))
                }
                _ => None,
            };
            if let Some((f, g)) = call {
                let helper = HELPERS
                    .with(|h| h.borrow().iter().find(|x| x.name == f).cloned());
                if helper.is_some() || ts.len() == 2 {
                    let mut v = Vec::new();
                    for x in split_commas(&g) {
                        v.push(tr(&x, subst)?);
                    }
                    if let Some(h) = helper {
                        return inline_helper(&h, &v);
                    }
                    return Ok(match f.as_str() {
                        "sra" => ebin(">>>", &v[0], &v[1]),
                        "lt_signed" => ebin("<s", &v[0], &v[1]),
                        "mux" => format!(
                            "NlE::Cond(Box::new({}), Box::new({}), Box::new({}))",
                            v[0], v[1], v[2]
                        ),
                        other => {
                            return Err(format!(
                                "function `{other}` is not lowered"
                            ))
                        }
                    });
                }
            }
            if text == "Bit::One" || text == "true" {
                return Ok("NlE::Bits(1, \"1\".to_string())".into());
            }
            if text == "Bit::Zero" || text == "false" {
                return Ok("NlE::Bits(1, \"0\".to_string())".into());
            }
            if let Some(TokenTree::Group(g)) = ts.last() {
                // A bit from a truth value, or the reverse, is the value.
                if text.starts_with("Bit::from")
                    || text.starts_with("bool::from")
                {
                    let inner: Vec<TokenTree> =
                        g.stream().into_iter().collect();
                    return tr(&inner, subst);
                }
                // A sized literal keeps its width; a bare one is a number.
                if text.starts_with("U::<") {
                    return Ok(format!("::txhdl::netlist::lit({text})"));
                }
                if text.starts_with("U::from") {
                    return Ok(format!("NlE::Num(({}) as u128)", g.stream()));
                }
            }
            if text.contains("::") && !last_group {
                let leaf = text.rsplit("::").next().unwrap_or("");
                let upper = leaf.chars().all(|c| {
                    c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()
                });
                return Ok(if upper {
                    format!("NlE::Num(({text}) as u128)")
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
        acc =
            format!("NlE::Cond(Box::new({c}), Box::new({e}), Box::new({acc}))");
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
            "NlE::Bits(1, \"1\".to_string())".to_string()
        } else if digit {
            ebin("==", v, &format!("NlE::Num(({text}) as u128)"))
        } else {
            ebin("==", v, &format!("::txhdl::netlist::lit({text})"))
        };
        cond = Some(match cond {
            None => c,
            Some(prev) => ebin("||", &prev, &c),
        });
    }
    let mut cond =
        cond.unwrap_or_else(|| "NlE::Bits(1, \"1\".to_string())".to_string());
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
/// What the lowering of one loop body carries: the ports' names, the
/// wires of the unit, the names bound so far, the guard of the wait,
/// the clock and its edge, and the drives hoisted out of `if` arms.
struct Cx<'a> {
    pnames: &'a [String],
    wires: &'a mut Vec<(String, String)>,
    subst: Vec<(String, String)>,
    guard: Option<String>,
    clock: String,
    falling: bool,
    hoisted: Vec<String>,
}

/// The statements of a block, split on `;` at depth zero, and after
/// the last brace of an `if` chain, which Rust ends without one.
fn stmts_of(ts: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut out = Vec::new();
    let mut cur: Vec<TokenTree> = Vec::new();
    for (i, t) in ts.iter().enumerate() {
        match t {
            TokenTree::Punct(p) if p.as_char() == ';' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            TokenTree::Group(g)
                if g.delimiter() == Delimiter::Brace
                    && cur.first().is_some_and(|f| is_ident(f, "if")) =>
            {
                cur.push(t.clone());
                let more = ts.get(i + 1).is_some_and(|n| is_ident(n, "else"));
                if !more {
                    out.push(std::mem::take(&mut cur));
                }
            }
            t => cur.push(t.clone()),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The condition of a path through `if` arms: the enclosing path, the
/// failure of every arm above, and this arm's own condition.
fn conj(outer: Option<&str>, nots: &[String], own: Option<&str>) -> String {
    let mut acc: Option<String> = outer.map(|s| s.to_string());
    for n in nots.iter().map(|s| s.as_str()).chain(own) {
        acc = Some(match acc {
            None => n.to_string(),
            Some(a) => ebin("&&", &a, n),
        });
    }
    acc.unwrap_or_else(|| "NlE::Bits(1, \"1\".to_string())".into())
}

/// The statements of a `with!` block on `self`: a drive per entry, an
/// `if` with one arm for a predicated entry, an `if` with its `else`
/// for a group.
fn with_lowered(cx: &mut Cx, es: &[Entry]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for e in es {
        match e {
            Entry::Drive { pred, path, value } => {
                let mut t: Vec<TokenTree> = vec![
                    TokenTree::Ident(Ident::new("self", Span::call_site())),
                    TokenTree::Punct(Punct::new('.', Spacing::Alone)),
                ];
                t.extend(path.iter().cloned());
                let l = target_expr(&t, &cx.subst)?;
                let r = tr(value, &cx.subst)?;
                let d = format!("NlS::Drive({l}, {r})");
                out.push(match pred {
                    None => d,
                    Some(p) => {
                        let c = tr(p, &cx.subst)?;
                        format!("NlS::If(vec![({c}, vec![{d}])], vec![])")
                    }
                });
            }
            Entry::Group {
                pred,
                then,
                otherwise,
            } => {
                let c = tr(pred, &cx.subst)?;
                let yes = with_lowered(cx, then)?.join(", ");
                let no = with_lowered(cx, otherwise)?.join(", ");
                out.push(format!(
                    "NlS::If(vec![({c}, vec![{yes}])], vec![{no}])"
                ));
            }
        }
    }
    Ok(out)
}

/// The statements of a loop body, or of an arm of an `if` in it, as
/// the Rust source of the netlist's statements. `path` is the
/// condition under which an arm's statements happen, none at the top.
fn lower_stmts(
    cx: &mut Cx,
    toks: &[TokenTree],
    path: Option<String>,
) -> Result<Vec<String>, TokenStream> {
    let mut stmts: Vec<String> = Vec::new();
    for st in stmts_of(toks) {
        let ts: Vec<TokenTree> = st;
        let text: String = ts
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join("");
        let wait = text.ends_with(".await");
        if wait && path.is_some() {
            return Err(err(
                ts[0].span(),
                "a wait belongs at the top of the loop, not under `if`",
            ));
        }
        if let Some(c) = text.strip_suffix("::rising().await") {
            cx.clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
            continue;
        }
        if let Some(c) = text.strip_suffix("::falling().await") {
            cx.clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
            cx.falling = true;
            continue;
        }
        // until(C::rising, || cond).await: the wait, and the guard on
        // every register drive after it.
        if text.starts_with("until(") && text.ends_with(").await") {
            let TokenTree::Group(g) = &ts[1] else {
                return Err(err(ts[0].span(), "expected until(..)"));
            };
            let parts = split_commas(g);
            if parts.len() != 2 {
                return Err(err(
                    ts[0].span(),
                    "expected until(C::rising, || cond)",
                ));
            }
            let ctext: String = parts[0]
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join("");
            let c = if let Some(c) = ctext.strip_suffix("::rising") {
                c
            } else if let Some(c) = ctext.strip_suffix("::falling") {
                cx.falling = true;
                c
            } else {
                return Err(err(ts[0].span(), "expected until(C::rising, ..)"));
            };
            cx.clock = format!("<{c} as ::txhdl::comp::Clock>::NAME");
            let body = &parts[1][2..]; // past the `||` of the closure
            let cond = match tr(body, &cx.subst) {
                Ok(c) => c,
                Err(m) => return Err(err(ts[0].span(), &m)),
            };
            cx.guard = Some(cond.clone());
            stmts.push(format!("NlS::Guard({cond})"));
            continue;
        }
        // let v = rx.wait().await: a receive is the wait, and its guard.
        if text.starts_with("let") && text.ends_with(".wait().await") {
            let TokenTree::Ident(n) = &ts[1] else {
                return Err(err(ts[1].span(), "expected a name"));
            };
            let TokenTree::Ident(rx) = &ts[3] else {
                return Err(err(ts[3].span(), "expected a channel"));
            };
            let cond = ename(&format!("{rx}_valid"));
            cx.guard = Some(cond.clone());
            stmts.push(format!("NlS::Guard({cond})"));
            stmts.push(format!(
                "NlS::Drive(NlT::Name(\"{rx}_ready\".to_string()), \
                 NlE::Bits(1, \"1\".to_string()))"
            ));
            cx.subst.push((n.to_string(), ename(&format!("{rx}_data"))));
            continue;
        }
        // `name!(..)`: a macro call, `println!` and the like, which is
        // not hardware; `if !c` is not one.
        let is_macro = matches!(
            (&ts[0], ts.get(1)),
            (TokenTree::Ident(_), Some(TokenTree::Punct(p)))
                if p.as_char() == '!'
        ) && !is_ident(&ts[0], "if");
        if is_macro
            && !text.starts_with("when!")
            && !text.starts_with("case!")
            && !text.starts_with("with!")
        {
            continue;
        }
        // A receive under the guard: ready is asserted with the guard.
        // With no wait before it, the process takes whatever is
        // offered at every edge, and ready is valid: the take, which
        // is what the runtime's trace holds for ready.
        if path.is_some()
            && (text.contains(".recv()")
                || text.contains(".recv_if(")
                || text.contains(".take()"))
        {
            return Err(err(
                ts[0].span(),
                "a transaction is taken at the top of the loop, or with \
                     `recv_if`; not under `if`",
            ));
        }
        // `let (offered, v) = rx.take()`: whether one was offered is
        // `valid`, the value is `data`, and ready is the take, as for
        // `recv`.
        if text.starts_with("let") && text.ends_with(".take()") {
            let (TokenTree::Group(names), TokenTree::Ident(rx)) =
                (&ts[1], &ts[3])
            else {
                return Err(err(
                    ts[0].span(),
                    "expected `let (offered, v) = rx.take()`",
                ));
            };
            let ns = split_commas(names);
            if ns.len() != 2 {
                return Err(err(names.span(), "`take` gives (offered, value)"));
            }
            let g = cx
                .guard
                .clone()
                .unwrap_or_else(|| ename(&format!("{rx}_valid")));
            stmts.push(format!(
                "NlS::Drive(NlT::Name(\"{rx}_ready\".to_string()), {g})"
            ));
            cx.subst
                .push((ns[0][0].to_string(), ename(&format!("{rx}_valid"))));
            cx.subst
                .push((ns[1][0].to_string(), ename(&format!("{rx}_data"))));
            continue;
        }
        if text.contains(".recv()") {
            let TokenTree::Ident(rx) = &ts[3] else {
                return Err(err(ts[0].span(), "expected `let v = rx.recv()`"));
            };
            let g = cx
                .guard
                .clone()
                .unwrap_or_else(|| ename(&format!("{rx}_valid")));
            stmts.push(format!(
                "NlS::Drive(NlT::Name(\"{rx}_ready\".to_string()), {g})"
            ));
        }
        // A receive under a condition of its own: ready is the
        // condition, and the take happens when it holds and a
        // transaction is offered.
        if text.contains(".recv_if(") {
            let TokenTree::Ident(rx) = &ts[3] else {
                return Err(err(
                    ts[0].span(),
                    "expected `let v = rx.recv_if(c)`",
                ));
            };
            let after = ts
                .iter()
                .position(|t| match t {
                    TokenTree::Ident(i) => i.to_string() == "recv_if",
                    _ => false,
                })
                .and_then(|i| ts.get(i + 1));
            let Some(TokenTree::Group(g)) = after else {
                return Err(err(ts[0].span(), "expected `rx.recv_if(c)`"));
            };
            let ct: Vec<TokenTree> = g.stream().into_iter().collect();
            let c = match tr(&ct, &cx.subst) {
                Ok(c) => c,
                Err(m) => return Err(err(ts[0].span(), &m)),
            };
            // Ready is the take: the condition and a transaction
            // offered, which is what the runtime records.
            let v = ename(&format!("{rx}_valid"));
            stmts.push(format!(
                "NlS::Drive(NlT::Name(\"{rx}_ready\".to_string()), \
                     NlE::Bin(\"&\", Box::new({c}), Box::new({v})))"
            ));
        }
        if text.starts_with("let") {
            // `let a = e` or `let (a, b) = (e1, e2)`, bound pairwise.
            let (names, exprs): (Vec<Vec<TokenTree>>, Vec<Vec<TokenTree>>) =
                match (&ts[1], ts.get(3)) {
                    (TokenTree::Group(ng), Some(TokenTree::Group(eg))) => {
                        (split_commas(ng), split_commas(eg))
                    }
                    _ => (vec![vec![ts[1].clone()]], vec![ts[3..].to_vec()]),
                };
            if names.len() != exprs.len() {
                return Err(err(
                    ts[0].span(),
                    "a tuple let must bind pairwise",
                ));
            }
            for (n, e) in names.iter().zip(&exprs) {
                let name = n[0].to_string();
                let v = match tr(e, &cx.subst) {
                    Ok(v) => v,
                    Err(m) => return Err(err(ts[0].span(), &m)),
                };
                // A read of a port or a register, or a number, is an
                // alias; anything computed is a wire named for the let.
                let alias = v.starts_with("NlE::Name(")
                    || v.starts_with("NlE::Num(")
                    || v.starts_with("NlE::Bits(")
                    || v.starts_with("::txhdl::netlist::lit(");
                if alias || name == "_" {
                    cx.subst.push((name, v));
                    continue;
                }
                let mut w = name.clone();
                if cx.pnames.contains(&w) {
                    w.push_str("_w");
                }
                let taken = cx.wires.iter().filter(|(x, _)| *x == w).count();
                if taken > 0 {
                    w = format!("{w}_{}", taken + 1);
                }
                cx.wires.push((w.clone(), v));
                cx.subst.push((name, ename(&w)));
            }
            continue;
        }
        if text.starts_with("case!") {
            let TokenTree::Group(g) = &ts[2] else {
                return Err(err(ts[0].span(), "expected case!(..)"));
            };
            let ct: Vec<TokenTree> = g.stream().into_iter().collect();
            let Some((value, k)) = up_to_arrow(&ct, 0) else {
                return Err(err(ts[0].span(), "expected `value =>`"));
            };
            let vt: Vec<TokenTree> = value.into_iter().collect();
            let v = match tr(&vt, &cx.subst) {
                Ok(v) => v,
                Err(m) => return Err(err(ts[0].span(), &m)),
            };
            let TokenTree::Group(arms_g) = &ct[k] else {
                return Err(err(ts[0].span(), "expected the arms"));
            };
            let at: Vec<TokenTree> = arms_g.stream().into_iter().collect();
            let mut arms = Vec::new();
            let mut j = 0;
            while j < at.len() {
                let Some((pat, k2)) = up_to_arrow(&at, j) else {
                    return Err(err(
                        at[j].span(),
                        "expected `pattern => { .. }`",
                    ));
                };
                let pt: Vec<TokenTree> = pat.into_iter().collect();
                let cond = match pattern_cond(&pt, &v, &cx.subst) {
                    Ok(c) => c,
                    Err(m) => return Err(err(at[j].span(), &m)),
                };
                let TokenTree::Group(body) = &at[k2] else {
                    return Err(err(at[j].span(), "expected `{ .. }`"));
                };
                let mut drives = Vec::new();
                for d in statements(body) {
                    let Some((lhs, rhs)) = split_becomes(d) else {
                        return Err(err(
                            body.span(),
                            "expected `register <= value`",
                        ));
                    };
                    let lt: Vec<TokenTree> = lhs.into_iter().collect();
                    let rt: Vec<TokenTree> = rhs.into_iter().collect();
                    let l = match target_expr(&lt, &cx.subst) {
                        Ok(l) => l,
                        Err(m) => return Err(err(body.span(), &m)),
                    };
                    let r = match tr(&rt, &cx.subst) {
                        Ok(r) => r,
                        Err(m) => return Err(err(body.span(), &m)),
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
            stmts.push(format!("NlS::Case(vec![{}])", arms.join(", ")));
            continue;
        }
        // `if c { .. } else if d { .. } else { .. }`: a priority
        // chain, each arm's statements under its condition and the
        // failure of the arms above it; an arm may hold `let`s,
        // drives, `when!`, `case!` and another `if`. A `send` in an
        // arm is hoisted with the condition of its path as `valid`.
        if is_ident(&ts[0], "if") {
            let brace = |t: &TokenTree| {
                matches!(t, TokenTree::Group(g)
                        if g.delimiter() == Delimiter::Brace)
            };
            let mut arms: Vec<String> = Vec::new();
            let mut els = String::from("vec![]");
            let mut nots: Vec<String> = Vec::new();
            let mut i = 0;
            loop {
                let Some(b) = ts[i + 1..].iter().position(brace) else {
                    return Err(err(ts[i].span(), "expected `if c { .. }`"));
                };
                if b == 0 {
                    return Err(err(ts[i].span(), "`if` needs a condition"));
                }
                let c = match tr(&ts[i + 1..i + 1 + b], &cx.subst) {
                    Ok(c) => c,
                    Err(m) => return Err(err(ts[i].span(), &m)),
                };
                let TokenTree::Group(g) = &ts[i + 1 + b] else {
                    unreachable!()
                };
                let here = conj(path.as_deref(), &nots, Some(&c));
                let n = cx.subst.len();
                let gt: Vec<TokenTree> = g.stream().into_iter().collect();
                let body = lower_stmts(cx, &gt, Some(here))?;
                cx.subst.truncate(n);
                arms.push(format!("({c}, vec![{}])", body.join(", ")));
                nots.push(format!("NlE::Not(Box::new({c}))"));
                i += 2 + b;
                match (ts.get(i), ts.get(i + 1)) {
                    (Some(e), Some(f))
                        if is_ident(e, "else") && is_ident(f, "if") =>
                    {
                        i += 1;
                    }
                    (Some(e), Some(TokenTree::Group(g)))
                        if is_ident(e, "else")
                            && g.delimiter() == Delimiter::Brace =>
                    {
                        let here = conj(path.as_deref(), &nots, None);
                        let n = cx.subst.len();
                        let gt: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let body = lower_stmts(cx, &gt, Some(here))?;
                        cx.subst.truncate(n);
                        els = format!("vec![{}]", body.join(", "));
                        break;
                    }
                    (None, _) => break,
                    (Some(t), _) => {
                        return Err(err(
                            t.span(),
                            "expected `else`, `else if` or the end",
                        ))
                    }
                }
            }
            stmts.push(format!("NlS::If(vec![{}], {els})", arms.join(", ")));
            continue;
        }
        // `with!(self <= { .. })`: a drive per entry, under `if` for a
        // predicate or a group, the same chain `if` makes.
        if text.starts_with("when!") {
            let TokenTree::Group(g) = &ts[2] else {
                return Err(err(ts[0].span(), "expected when!(..)"));
            };
            let (cond, target, then, otherwise) = match when_parts(g.stream()) {
                Ok(x) => x,
                Err((s, m)) => return Err(err(s, &m)),
            };
            let unit = matches!(target.as_slice(),
                [t] if is_ident(t, "self") || is_ident(t, "this"));
            if !unit {
                return Err(err(
                    ts[0].span(),
                    "a lowered `when!` drives `self`; another target is not \
                     hardware of this unit",
                ));
            }
            let then = entries(&then).map_err(|(s, m)| err(s, &m))?;
            let otherwise = match otherwise {
                Some(g) => entries(&g).map_err(|(s, m)| err(s, &m))?,
                None => Vec::new(),
            };
            let group = Entry::Group {
                pred: cond,
                then,
                otherwise,
            };
            let lowered =
                with_lowered(cx, &[group]).map_err(|m| err(g.span(), &m))?;
            stmts.extend(lowered);
            continue;
        }
        if text.starts_with("with!") {
            let TokenTree::Group(g) = &ts[2] else {
                return Err(err(ts[0].span(), "expected with!(..)"));
            };
            let (target, block) = match with_parts(g.stream()) {
                Ok(x) => x,
                Err((s, m)) => return Err(err(s, &m)),
            };
            // `self`, or `this`, a process's name for the unit.
            let unit = matches!(target.as_slice(),
                [t] if is_ident(t, "self") || is_ident(t, "this"));
            if !unit {
                return Err(err(
                    ts[0].span(),
                    "a lowered `with!` drives `self`; another target is not \
                     hardware of this unit",
                ));
            }
            let es = match entries(&block) {
                Ok(e) => e,
                Err((s, m)) => return Err(err(s, &m)),
            };
            let lowered =
                with_lowered(cx, &es).map_err(|m| err(g.span(), &m))?;
            stmts.extend(lowered);
            continue;
        }
        // Under `if`: a send is hoisted, its valid the path's
        // condition; a drive is a register's or a memory word's,
        // since an output is a wire and takes a `mux`.
        if let Some(here) = path.as_deref() {
            if ts.len() >= 3 {
                let end = ts.len() - 3;
                if let (
                    TokenTree::Punct(dot),
                    TokenTree::Ident(m),
                    TokenTree::Group(g),
                ) = (&ts[end], &ts[end + 1], &ts[end + 2])
                {
                    if dot.as_char() == '.' && m.to_string() == "send" {
                        let tx = match target_name(&ts[..end]) {
                            Ok(t) => t,
                            Err(m) => return Err(err(ts[0].span(), &m)),
                        };
                        let at: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let e = match tr(&at, &cx.subst) {
                            Ok(e) => e,
                            Err(m) => return Err(err(ts[0].span(), &m)),
                        };
                        let data = format!("\"{tx}_data\"");
                        if cx.hoisted.iter().any(|h| h.contains(&data)) {
                            return Err(err(
                                ts[0].span(),
                                "one `send` per channel under `if`",
                            ));
                        }
                        let v = match &cx.guard {
                            Some(gd) => ebin("&&", gd, here),
                            None => here.to_string(),
                        };
                        cx.hoisted.push(format!(
                            "NlS::Drive(NlT::Name({data}.to_string()), {e})"
                        ));
                        cx.hoisted.push(format!(
                            "NlS::Drive(NlT::Name(\"{tx}_valid\"\
                                 .to_string()), {v})"
                        ));
                        continue;
                    }
                    if dot.as_char() == '.' && m.to_string() == "set" {
                        let l = match target_expr(&ts[..end], &cx.subst) {
                            Ok(l) => l,
                            Err(m) => return Err(err(ts[0].span(), &m)),
                        };
                        if cx.pnames.iter().any(|p| {
                            l == format!("NlT::Name(\"{p}\".to_string())")
                        }) {
                            return Err(err(
                                ts[0].span(),
                                "an output under `if` is a wire: drive \
                                     it once, with `mux`",
                            ));
                        }
                        let at: Vec<TokenTree> =
                            g.stream().into_iter().collect();
                        let e = match tr(&at, &cx.subst) {
                            Ok(e) => e,
                            Err(m) => return Err(err(ts[0].span(), &m)),
                        };
                        stmts.push(format!("NlS::Drive({l}, {e})"));
                        continue;
                    }
                }
            }
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
                    let Some(gd) = cx.guard.clone() else {
                        return Err(err(
                            ts[0].span(),
                            "send needs a wait before it",
                        ));
                    };
                    let tx = match target_name(&ts[..end]) {
                        Ok(t) => t,
                        Err(m) => return Err(err(ts[0].span(), &m)),
                    };
                    let at: Vec<TokenTree> = g.stream().into_iter().collect();
                    let e = match tr(&at, &cx.subst) {
                        Ok(e) => e,
                        Err(m) => return Err(err(ts[0].span(), &m)),
                    };
                    stmts.push(format!(
                        "NlS::Drive(NlT::Name(\"{tx}_data\".to_string()), {e})"
                    ));
                    stmts.push(format!(
                        "NlS::Drive(NlT::Name(\"{tx}_valid\".to_string()), {gd})"
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
                        Err(m) => return Err(err(ts[0].span(), &m)),
                    };
                    let at: Vec<TokenTree> = g.stream().into_iter().collect();
                    let e = match tr(&at, &cx.subst) {
                        Ok(e) => e,
                        Err(m) => return Err(err(ts[0].span(), &m)),
                    };
                    stmts.push(format!(
                        "NlS::Drive(NlT::Name(\"{target}\".to_string()), {e})"
                    ));
                    continue;
                }
            }
        }
        return Err(err(ts[0].span(), &format!("cannot lower `{text}`")));
    }
    Ok(stmts)
}

/// A unit of units: `run` makes the channels and wires between its
/// children with `chan()` and `signal()`, and joins the children's
/// `run`s. Read into the parent's nets and instances, as generated
/// text: a `(name, kind, width)` per net, and an
/// `instance(child_lowered(&me.FIELD, ..), "FIELD", &[..])` per child,
/// its ports joined in order to nets and to the parent's ports.
/// `ports` are the parent's, by name and kind.
fn lower_structural(
    body: &Group,
    ports: &[(String, String)],
) -> Result<(Vec<String>, Vec<String>), TokenStream> {
    // The ends made in `run`: the end, its net, whether a channel.
    let mut ends: Vec<(String, String, bool)> = Vec::new();
    let mut nets: Vec<String> = Vec::new();
    let mut instances: Vec<String> = Vec::new();
    // The channel ends and channel ports joined so far, each once.
    let mut used: Vec<String> = Vec::new();
    let is_chan = |k: &str| k == "Tx" || k == "Rx";
    for st in statements(body) {
        let ts: Vec<TokenTree> = st.into_iter().collect();
        if ts.is_empty() {
            continue;
        }
        if is_ident(&ts[0], "let") {
            let bad = |t: &TokenTree| {
                err(
                    t.span(),
                    "a net is `let (tx, rx) = chan::<T, C>()` or \
                     `let (out, inp) = signal::<T, C>()`",
                )
            };
            let Some(TokenTree::Group(names)) = ts.get(1) else {
                return Err(bad(&ts[0]));
            };
            let ns = split_commas(names);
            if ns.len() != 2 || ns.iter().any(|n| n.len() != 1) {
                return Err(bad(&ts[1]));
            }
            let (a, b) = (ns[0][0].to_string(), ns[1][0].to_string());
            let Some(f) = ts.get(3) else {
                return Err(bad(&ts[0]));
            };
            let chan = is_ident(f, "chan");
            if !chan && !is_ident(f, "signal") {
                return Err(bad(f));
            }
            // The payload: the first argument of the turbofish.
            let lt = ts.iter().position(
                |t| matches!(t, TokenTree::Punct(p) if p.as_char() == '<'),
            );
            let gt = ts.iter().rposition(
                |t| matches!(t, TokenTree::Punct(p) if p.as_char() == '>'),
            );
            let (Some(lt), Some(gt)) = (lt, gt) else {
                return Err(err(
                    f.span(),
                    "name the payload: `chan::<T, C>()`",
                ));
            };
            let inner = text_of(&ts[lt + 1..gt]);
            let (ty, clock) = match depth0_comma(&inner) {
                Some(c) => (
                    inner[..c].trim().to_string(),
                    inner[c + 1..].trim().to_string(),
                ),
                None => (inner, "::txhdl::comp::DefaultClock".to_string()),
            };
            // The net is the ends' common prefix, else the first end.
            let common: String = a
                .chars()
                .zip(b.chars())
                .take_while(|(x, y)| x == y)
                .map(|(x, _)| x)
                .collect();
            let net = match common.trim_end_matches('_') {
                "" => a.clone(),
                c => c.to_string(),
            };
            if ends.iter().any(|(_, n, _)| *n == net)
                || ports.iter().any(|(p, _)| *p == net)
            {
                return Err(err(
                    ts[1].span(),
                    &format!("net `{net}` is named twice"),
                ));
            }
            let kind = if chan { "Tx" } else { "Out" };
            nets.push(format!(
                "(\"{net}\".to_string(), ::txhdl::comp::trace::Kind::{kind}, \
                 <{ty} as ::txhdl::types::Value>::WIDTH, \
                 <{clock} as ::txhdl::comp::Clock>::NAME)"
            ));
            ends.push((a, net.clone(), chan));
            ends.push((b, net, chan));
            continue;
        }
        // A join of the children: every `self.FIELD.run(ins, outs)`.
        let mut calls: Vec<(String, Group, Span)> = Vec::new();
        find_runs(&ts, &mut calls);
        if calls.is_empty() {
            return Err(err(
                ts[0].span(),
                "a unit of units' `run` is lets of `chan()` or `signal()` \
                 and a join of the children's `run`, each `self.child.run(..)`",
            ));
        }
        for (field, args, span) in calls {
            let sides = split_commas(&args);
            if sides.len() != 2 {
                return Err(err(
                    span,
                    "a child's `run` takes its inputs and its outputs",
                ));
            }
            let mut names: Vec<String> = Vec::new();
            for side in &sides {
                match side.as_slice() {
                    [TokenTree::Group(g)]
                        if g.delimiter() == Delimiter::Parenthesis =>
                    {
                        for n in split_commas(g) {
                            let [TokenTree::Ident(id)] = n.as_slice() else {
                                return Err(err(
                                    g.span(),
                                    "a port passed to a child is a name",
                                ));
                            };
                            names.push(id.to_string());
                        }
                    }
                    [TokenTree::Ident(id)] => names.push(id.to_string()),
                    _ => {
                        return Err(err(
                            span,
                            "a port passed to a child is a name, a tuple \
                             of names, or `()`",
                        ))
                    }
                }
            }
            let mut joined: Vec<String> = Vec::new();
            for n in names {
                let (net, channel) = if let Some((_, net, ch)) =
                    ends.iter().find(|(e, _, _)| *e == n)
                {
                    (net.clone(), *ch)
                } else if let Some((p, k)) = ports.iter().find(|(p, _)| *p == n)
                {
                    (p.clone(), is_chan(k))
                } else {
                    return Err(err(
                        span,
                        &format!(
                            "`{n}` is neither a port of the unit nor an \
                                 end made in `run`"
                        ),
                    ));
                };
                if channel {
                    if used.contains(&n) {
                        return Err(err(
                            span,
                            &format!(
                                "`{n}` is joined twice; a channel has one \
                                 unit at each end"
                            ),
                        ));
                    }
                    used.push(n.clone());
                }
                joined.push(format!("\"{net}\""));
            }
            instances.push(format!(
                "::txhdl::netlist::instance(::txhdl::netlist::child_lowered(\
                 &me.{field}, &format!(\"{{name}}_{field}\")), \"{field}\", \
                 &[{}])",
                joined.join(", ")
            ));
        }
    }
    // Every channel end made, and every channel port, is joined.
    for (e, _, ch) in &ends {
        if *ch && !used.contains(e) {
            return Err(err(
                body.span(),
                &format!("`{e}` is made and joined to no child"),
            ));
        }
    }
    for (p, k) in ports {
        if is_chan(k) && !used.contains(p) {
            return Err(err(
                body.span(),
                &format!("port `{p}` is joined to no child"),
            ));
        }
    }
    if instances.is_empty() {
        return Err(err(
            body.span(),
            "run must be a `loop`, or `join2` of loops, or a join of the \
             children's `run`",
        ));
    }
    Ok((nets, instances))
}

/// Every `self.FIELD.run(ARGS)` in a token list, into any group.
fn find_runs(ts: &[TokenTree], out: &mut Vec<(String, Group, Span)>) {
    let mut i = 0;
    while i < ts.len() {
        if let (
            s,
            Some(TokenTree::Punct(d1)),
            Some(TokenTree::Ident(f)),
            Some(TokenTree::Punct(d2)),
            Some(r),
            Some(TokenTree::Group(g)),
        ) = (
            &ts[i],
            ts.get(i + 1),
            ts.get(i + 2),
            ts.get(i + 3),
            ts.get(i + 4),
            ts.get(i + 5),
        ) {
            if is_ident(s, "self")
                && d1.as_char() == '.'
                && d2.as_char() == '.'
                && is_ident(r, "run")
                && g.delimiter() == Delimiter::Parenthesis
            {
                out.push((f.to_string(), g.clone(), f.span()));
                i += 6;
                continue;
            }
        }
        if let TokenTree::Group(g) = &ts[i] {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            find_runs(&inner, out);
        }
        i += 1;
    }
}

#[proc_macro_attribute]
pub fn lower(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = item.clone().into_iter().collect();
    // On a function: the function is plain Rust for the simulation,
    // and the lowered units of its file inline it, see `find_helpers`.
    let first = toks.iter().find_map(|t| match t {
        TokenTree::Ident(id)
            if id.to_string() == "fn" || id.to_string() == "impl" =>
        {
            Some(id.to_string())
        }
        _ => None,
    });
    if first.as_deref() == Some("fn") {
        return item;
    }
    HELPERS.with(|h| {
        *h.borrow_mut() = find_helpers(Span::call_site().local_file())
    });
    PTYPES.with(|p| p.borrow_mut().clear());
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
    // The two sides' types as written, for the impl header when it
    // names none: `impl Unit for X` is `impl Unit<I, O> for X`.
    let mut sides: Vec<String> = Vec::new();
    for p in split_type_commas(params).into_iter().skip(1) {
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
        sides.push(text_of(&p[colon + 1..]));
        match (&p[0], &p[colon + 1]) {
            (TokenTree::Ident(n), _) => {
                pairs.push((n.to_string(), text(&p[colon + 1..]), n.span()));
            }
            (TokenTree::Group(names), TokenTree::Group(tys))
                if names.delimiter() == Delimiter::Parenthesis
                    && tys.delimiter() == Delimiter::Parenthesis =>
            {
                let ns = split_commas(names);
                let ts = split_type_commas(tys);
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
    // The ports by name and kind, for a unit of units' joins.
    let mut pkinds: Vec<(String, String)> = Vec::new();
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
        pkinds.push((pname.clone(), kind.to_string()));
        PTYPES
            .with(|p| p.borrow_mut().push((pname.clone(), inner.to_string())));
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
    // No loop: a unit of units, whose run joins its children.
    let mut nets: Vec<String> = Vec::new();
    let mut instances: Vec<String> = Vec::new();
    if loops.is_empty() {
        match lower_structural(fbody, &pkinds) {
            Ok((n, i)) => {
                nets = n;
                instances = i;
            }
            Err(e) => return e,
        }
    }
    // `let` names that became wires of the netlist, with what drives
    // each; a name bound twice gets a numbered second wire.
    let mut wires: Vec<(String, String)> = Vec::new();
    let mut procs: Vec<String> = Vec::new();
    for lbody in &loops {
        let toks: Vec<TokenTree> = lbody.stream().into_iter().collect();
        let mut cx = Cx {
            pnames: &pnames,
            wires: &mut wires,
            subst: Vec::new(),
            guard: None,
            clock: String::new(),
            falling: false,
            hoisted: Vec::new(),
        };
        let mut stmts = match lower_stmts(&mut cx, &toks, None) {
            Ok(s) => s,
            Err(e) => return e,
        };
        stmts.append(&mut cx.hoisted);
        let (clock, falling) = (cx.clock.clone(), cx.falling);
        drop(cx);
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
    // A unit of units reaches its children through a value of itself.
    let prelude = if instances.is_empty() {
        ""
    } else {
        "let me = Self::default();\n"
    };
    let generated_text = format!(
        "impl{generics} {unit} {{\n\
         /// This unit as `#[lower]` read it from `run`; `.verilog()` and\n\
         /// `.vhdl()` render it.\n\
         #[allow(unused_variables, clippy::all)]\n\
         pub fn lowered(name: &str) -> ::txhdl::netlist::Lowered {{\n\
         use ::txhdl::netlist::{{Expr as NlE, Stmt as NlS, Target as NlT}};\n\
         {prelude}\
         ::txhdl::netlist::Lowered {{\n\
         name: name.to_string(),\n\
         fields: <Self as ::txhdl::netlist::Fields>::fields(),\n\
         ports: vec![{ports}],\n\
         wires: vec![{wires}],\n\
         procs: vec![{procs}],\n\
         init: Vec::new(),\n\
         aliases: Vec::new(),\n\
         nets: vec![{nets}],\n\
         instances: vec![{instances}],\n\
         }}\n}}\n\
         /// The Verilog of this unit.\n\
         pub fn verilog(name: &str) -> String {{\n\
         Self::lowered(name).verilog() }}\n\
         /// The VHDL of this unit.\n\
         pub fn vhdl(name: &str) -> String {{ Self::lowered(name).vhdl() }}\n\
         }}\n\
         impl{generics} ::txhdl::netlist::Lower for {unit} {{\n\
         fn lowered_as(name: &str) -> ::txhdl::netlist::Lowered {{\n\
         Self::lowered(name) }}\n}}",
        ports = ports.join(", "),
        nets = nets.join(",\n"),
        instances = instances.join(",\n"),
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
    // `impl Unit for X`, the ports named once, in `run`: the header
    // takes them from there.
    let bare = toks[at..f].iter().enumerate().find_map(|(k, t)| {
        let next = toks.get(at + k + 1);
        (is_ident(t, "Unit")
            && !matches!(next, Some(TokenTree::Punct(p)) if p.as_char() == '<'))
        .then_some(at + k)
    });
    let mut out = TokenStream::new();
    if let Some(u) = bare {
        if sides.len() != 2 {
            return err(
                toks[u].span(),
                "`impl Unit for ..` needs `run(&mut self, inputs, outputs)`",
            );
        }
        let args: TokenStream =
            format!("<{}, {}>", sides[0], sides[1]).parse().unwrap();
        out.extend(toks[..=u].iter().cloned());
        out.extend(args);
        out.extend(toks[u + 1..].iter().cloned());
    } else {
        out = item;
    }
    out.extend(generated);
    out
}
