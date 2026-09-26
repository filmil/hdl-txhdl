// SPDX-License-Identifier: Apache-2.0
//! The macros of the runtime: four derives, `interface!`, `when!`,
//! `case!` and `#[lower]`, on a unit's impl and on a function it
//! inlines. Written
//! against `proc_macro` alone, without syn or quote, because the
//! grammars are small and a crate registry would be the larger cost.
extern crate proc_macro;
mod regmap;
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
    field_idents(body).iter().map(|id| id.to_string()).collect()
}

/// The netlist name of each field of a braced struct body, in the
/// order `field_idents` gives them: the name a `#[rename("...")]`
/// attribute on the field asks for, or `None` where there is none.
///
/// A netlist name is not a Rust name: VHDL and Verilog reserve words
/// Rust does not, and a field whose name a target reserves is escaped
/// (issue 497). `#[rename("...")]` is how a field keeps the name that
/// reads best in Rust and takes another in the netlist (issue 222).
fn field_renames(body: &Group) -> Vec<Option<(String, Span)>> {
    let toks: Vec<TokenTree> = body.stream().into_iter().collect();
    let mut out = Vec::new();
    let mut pending: Option<(String, Span)> = None;
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        // An attribute: `#` and a bracketed group.
        if let (TokenTree::Punct(h), Some(TokenTree::Group(g))) =
            (&toks[i], toks.get(i + 1))
        {
            if h.as_char() == '#' && g.delimiter() == Delimiter::Bracket {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                if let [TokenTree::Ident(k), TokenTree::Group(a)] =
                    inner.as_slice()
                {
                    if k.to_string() == "rename" {
                        let text = a.stream().to_string();
                        let name = text.trim().trim_matches('"').to_string();
                        pending = Some((name, k.span()));
                    }
                }
                i += 2;
                continue;
            }
        }
        match &toks[i] {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p) if p.as_char() == '>' => depth -= 1,
            TokenTree::Ident(_) if depth == 0 => {
                if let Some(TokenTree::Punct(p)) = toks.get(i + 1) {
                    let path = matches!(
                        toks.get(i + 2),
                        Some(TokenTree::Punct(q)) if q.as_char() == ':'
                    );
                    if p.as_char() == ':' && !path {
                        out.push(pending.take());
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The fields of a braced struct body as their name tokens, in order,
/// so that a check can point at one.
fn field_idents(body: &Group) -> Vec<Ident> {
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
                            names.push(id.clone());
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

/// `#[derive(Ports)]`: a struct whose fields are the ends a unit takes,
/// so that a side of `run` is the struct under one name. The fields
/// are the ports in declaration order, each named for its field, and
/// what each is comes from its type through `PortEnd` when `lowered`
/// runs, so `#[lower]` needs nothing but the trait and the struct can
/// be declared in any file of any crate (issue 483).
#[proc_macro_derive(Ports)]
pub fn derive_ports(input: TokenStream) -> TokenStream {
    let item = parse_item(input);
    let (Some(body), "struct") = (&item.body, item.kind.as_str()) else {
        return err(
            Span::call_site(),
            "Ports needs a struct with named fields",
        );
    };
    let ports = field_names(body)
        .iter()
        .zip(field_types(body))
        .map(|(n, t)| {
            format!(
                "v.extend(<{t} as ::txhdl::netlist::PortField>\
                 ::ports_of(\"{n}\"));"
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    // A struct of ports is a port field too, so another struct of ports
    // may hold it: its ports are flattened, each `field_sub` (issue 498).
    format!(
        "impl{b} ::txhdl::netlist::Ports for {n}{a} {{\n\
         fn ports() -> Vec<::txhdl::netlist::BundlePort> {{ \
         let mut v = Vec::new(); {ports} v }}\n}}\n\
         impl{b} ::txhdl::netlist::PortField for {n}{a} {{\n\
         fn ports_of(name: &str) -> Vec<::txhdl::netlist::BundlePort> {{ \
         <Self as ::txhdl::netlist::Ports>::ports().into_iter().map(|mut p| \
         {{ p.name = format!(\"{{name}}_{{}}\", p.name); p }})\
         .collect() }}\n}}",
        b = item.bounds,
        n = item.name,
        a = item.args
    )
    .parse()
    .unwrap()
}

/// `#[derive(Trace)]`: every field is registered under its own name,
/// or under the name `#[rename("...")]` gives it in the netlist.
#[proc_macro_derive(Trace, attributes(rename))]
pub fn derive_trace(input: TokenStream) -> TokenStream {
    let item = parse_item(input);
    let Some(body) = &item.body else {
        return err(Span::call_site(), "Trace needs a braced struct");
    };
    let rust = field_names(body);
    let renames = field_renames(body);
    // The name each field takes in the netlist and in the trace: its
    // own, or the one it was renamed to, escaped where either target
    // reserves it (issue 497). The trace takes the netlist's name, so
    // the testbench made from the trace finds the register by it.
    let names: Vec<String> = rust
        .iter()
        .zip(&renames)
        .map(|(n, r)| match r {
            Some((net, _)) => escaped(net),
            None => escaped(n),
        })
        .collect();
    // Two fields cannot take one name in the netlist, which would
    // declare it twice. With escaping that includes a field whose
    // escaped name another field has, `next` beside `next_rw`.
    let mut refused = TokenStream::new();
    for (k, n) in names.iter().enumerate() {
        if names[..k].contains(n) {
            let span = match &renames[k] {
                Some((_, s)) => *s,
                None => field_idents(body)[k].span(),
            };
            refused.extend(err(
                span,
                &format!("two fields are called `{n}` in the netlist"),
            ));
        }
    }
    let calls = rust
        .iter()
        .zip(&names)
        .map(|(f, n)| {
            format!(
                "::txhdl::comp::trace::Traceable::trace_as(\
                 &self.{f}, scope, \"{n}\");"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let types = field_types(body);
    let fields = names
        .iter()
        .zip(&types)
        .map(|(n, t)| {
            format!("__v.extend(<{t} as ::txhdl::netlist::Port>::entries(\"{n}\"));")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let generated = format!(
        "impl{b} ::txhdl::comp::trace::Traceable for {n}{a} {{\n\
         fn trace(&self, scope: &::txhdl::comp::trace::Scope) {{\n\
         {calls} }}\n}}\n\
         impl{b} ::txhdl::netlist::Fields for {n}{a} {{\n\
         const NAMES: &'static [&'static str] = &[{quoted}];\n\
         const RENAMES: &'static [(&'static str, &'static str)] = \
         &[{pairs}];\n\
         fn fields() -> Vec<(&'static str, \
         Option<::txhdl::comp::trace::Kind>, usize, usize)> {{ \
         let mut __v = Vec::new(); {fields} __v }}\n}}\n\
         impl{b} ::txhdl::netlist::Port for {n}{a} {{}}",
        b = item.bounds,
        n = item.name,
        a = item.args,
        quoted = names
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(", "),
        pairs = rust
            .iter()
            .zip(&names)
            .filter(|(f, n)| f != n)
            .map(|(f, n)| format!("(\"{f}\", \"{n}\")"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    let mut out: TokenStream = generated.parse().unwrap();
    out.extend(refused);
    out
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

// ---------------------------------------------------------------------
// Names a netlist cannot hold

#[path = "../src/reserved.rs"]
mod reserved;
use reserved::{escaped, reserved_by};

// ---------------------------------------------------------------------
// A process of several waits

mod seq;

/// `ts` with every token given `span`: a check the macro writes then
/// reports at the name it checks. Every path in such a check is
/// absolute, so where its names resolve does not change.
fn placed_at(ts: TokenStream, span: Span) -> TokenStream {
    let at = span;
    ts.into_iter()
        .map(|t| match t {
            TokenTree::Group(g) => {
                let mut n =
                    Group::new(g.delimiter(), placed_at(g.stream(), span));
                n.set_span(at);
                TokenTree::Group(n)
            }
            mut t => {
                t.set_span(at);
                t
            }
        })
        .collect()
}

#[cfg(test)]
mod reserved_tests {
    use super::{escaped, reserved_by};

    #[test]
    fn the_names_that_broke_netlists_are_reserved() {
        // The names issue 77 lists, and those `eth` renamed.
        assert_eq!(reserved_by("next"), Some("VHDL"));
        assert_eq!(reserved_by("tri"), Some("Verilog"));
        assert_eq!(reserved_by("inside"), Some("Verilog"));
        for n in ["out", "begin", "end", "body", "buf", "byte"] {
            assert!(reserved_by(n).is_some(), "{n}");
        }
        assert_eq!(reserved_by("begin"), Some("VHDL and Verilog"));
    }

    #[test]
    fn vhdl_is_not_case_sensitive_and_verilog_is() {
        assert_eq!(reserved_by("Next"), Some("VHDL"));
        assert_eq!(reserved_by("TRI"), None);
    }

    #[test]
    fn ordinary_names_are_not() {
        for n in ["pend", "hit", "edged", "turn", "count", "irq", "data"] {
            assert_eq!(reserved_by(n), None, "{n}");
        }
    }
    /// A reserved name takes `_rw`, any other name is left as it is,
    /// and a name that has been escaped escapes to itself, which is
    /// what lets `trace_as` take a port's own name (issue 497).
    #[test]
    fn a_reserved_name_is_escaped_once() {
        assert_eq!(escaped("next"), "next_rw");
        assert_eq!(escaped("Signal"), "Signal_rw");
        assert_eq!(escaped("count"), "count");
        for n in ["next", "out", "begin", "inside", "shared", "count"] {
            assert_eq!(escaped(&escaped(n)), escaped(n), "{n}");
        }
    }
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
    let line_fields = each(
        &|i| {
            format!(
                "    /// What input {i} contributed to this line.\n\
                 \x20   pub v{i}: T{i},"
            )
        },
        "\n",
    );
    let state = each(
        &|i| {
            format!(
                "    /// Input {i}'s cell of every line: the value it\n\
                 \x20   /// contributed, by tag.\n\
                 \x20   pub mem{i}: Mem<T{i}, L>,\n\
                 \x20   /// Which of input {i}'s cells hold, a bit per line.\n\
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
        "/// A reservation station of {n} inputs, and the line it\n\
         /// sends. Written by `station!`; see [`{name}`].\n\
         pub mod {module} {{\n\
         use super::Tagged;\n\
         use ::txhdl::comp::{{mux, Clock, DefaultClock, Mem, Reg, Rx, Tx, Unit}};\n\
         use ::txhdl::types::{{Transaction, U, Value}};\n\
         use ::txhdl::{{lower, with, Trace}};\n\
         \n\
         /// A complete line of {n}: the tag and one value per input.\n\
         #[derive(::txhdl::Transaction, ::txhdl::Value, Clone, Copy, Default)]\n\
         pub struct {lname}<\n    const TB: usize,\n{tparams}\n> {{\n\
         \x20   /// The tag every input of this line carried.\n\
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
// lite_bridge!

/// The text of an AXI4 to AXI-Lite bridge of `n` peripherals, named
/// `name`. The template is written once with `@` markers, and the
/// parts that repeat per peripheral are put in by loops; a chain over
/// the peripherals is a wire each, as the router's is, so no line
/// grows with the count.
fn lite_bridge_text(name: &str, n: usize) -> String {
    let idx: Vec<usize> = (0..n).collect();
    let each = |f: &dyn Fn(usize) -> String, sep: &str| -> String {
        idx.iter().map(|&i| f(i)).collect::<Vec<_>>().join(sep)
    };
    // The peripheral select is one bit per peripheral, and two bits at
    // least, since a one-bit value is a `std_logic` in VHDL and cannot
    // be indexed.
    let sw = n.max(2);
    let module = format!("lite_bridge{n}");
    let ranges = each(
        &|i| format!("    const BASE{i}: usize,\n    const MASK{i}: usize,"),
        "\n",
    );
    let rargs = each(&|i| format!("        BASE{i},\n        MASK{i},"), "\n");
    let in_names =
        each(&|i| format!("            b{i},\n            r{i},"), "\n");
    let in_types = each(
        &|_| "            Rx<LiteB>,\n            Rx<LiteR<D>>,".to_string(),
        "\n",
    );
    let out_names = each(
        &|i| {
            format!("            aw{i},\n            ar{i},\n            w{i},")
        },
        "\n",
    );
    let out_types = each(
        &|_| {
            "            Tx<LiteAw<A>>,\n            \
             Tx<LiteAr<A>>,\n            \
             Tx<LiteW<D, S>>,"
                .to_string()
        },
        "\n",
    );
    // A choice over the peripherals by the bits of `pick`: `name{i}` is
    // `val(i)` when bit `i` is set, else the next; the last falls back
    // to `tail`. The choice is `name0`.
    let chain = |name: &str,
                 pick: &str,
                 val: &dyn Fn(usize) -> String,
                 tail: &str|
     -> String {
        let top = n - 1;
        let mut lines = vec![format!(
            "            let {name}{top} =\n                \
             mux({pick}.bit({top}), {}, {tail});",
            val(top)
        )];
        for i in (0..top).rev() {
            lines.push(format!(
                "            let {name}{i} =\n                \
                 mux({pick}.bit({i}), {}, {name}{});",
                val(i),
                i + 1
            ));
        }
        lines.join("\n")
    };
    // The decode of one address channel's head, as a one-hot, the
    // lowest range that matches winning.
    let decode = |ph: &str| -> String {
        let hits = each(
            &|i| {
                format!(
                    "            let {ph}_hit{i} =\n                \
                     ({ph}h.addr.raw() as usize & MASK{i}) == BASE{i};"
                )
            },
            "\n",
        );
        let top = n - 1;
        let mut sel = vec![format!(
            "            let {ph}_sel{top} = mux(\n                \
             {ph}_hit{top},\n                \
             U::<{sw}>::from({}u32),\n                \
             U::<{sw}>::from(0u8),\n            );",
            1u32 << top
        )];
        for i in (0..top).rev() {
            sel.push(format!(
                "            let {ph}_sel{i} = mux(\n                \
                 {ph}_hit{i},\n                \
                 U::<{sw}>::from({}u32),\n                \
                 {ph}_sel{},\n            );",
                1u32 << i,
                i + 1
            ));
        }
        format!("{hits}\n{}", sel.join("\n"))
    };
    let heads = each(
        &|i| {
            format!(
                "            let b{i}h = b{i}.head();\n            \
                 let r{i}h = r{i}.head();\n            \
                 let off_b{i} = Bit::from(b{i}.peek().is_some());\n            \
                 let off_r{i} = Bit::from(r{i}.peek().is_some());"
            )
        },
        "\n",
    );
    let sends = each(
        &|i| {
            format!(
                "            if (w_go & cur.bit({i})).to_bool() {{\n\
                 \x20               \
                 aw{i}.send(LiteAw {{\n                    \
                 addr: self.addr.get(),\n                    \
                 prot: self.prot.get(),\n                \
                 }});\n                \
                 w{i}.send(LiteW {{\n                    \
                 data: wh.data,\n                    \
                 strb: wh.strb,\n                \
                 }});\n            \
                 }}\n            \
                 if (ar_go & cur.bit({i})).to_bool() {{\n                \
                 ar{i}.send(LiteAr {{\n                    \
                 addr: self.addr.get(),\n                    \
                 prot: self.prot.get(),\n                \
                 }});\n            \
                 }}"
            )
        },
        "\n",
    );
    let takes = each(
        &|i| {
            format!(
                "            let _ = b{i}.recv_if(b_can & cur.bit({i}));\n\
                 \x20           \
                 let _ = r{i}.recv_if(r_can & cur.bit({i}));"
            )
        },
        "\n",
    );
    let template = r#"/// An AXI4 to AXI-Lite bridge of @N@ peripherals.
/// Written by `lite_bridge!`; see [`@NAME@`].
pub mod @MODULE@ {
use crate::bus::axi::{Ar, Aw, BurstKind, Resp, B, R, W};
use crate::bus::axi_lite::{LiteAr, LiteAw, LiteB, LiteR, LiteW};
use ::txhdl::comp::{mux, Clock, DefaultClock, Reg, Rx, Tx, Unit};
use ::txhdl::types::{Bit, U};
use ::txhdl::{lower, with, Trace};

/// An AXI4 to AXI-Lite bridge of @N@ peripherals. The five AXI4
/// channels of one link come in; five AXI-Lite channels go out per
/// peripheral, and a peripheral's range is its `BASE` and `MASK`,
/// matched on a burst's address. The bridge takes one burst at a
/// time, since AXI-Lite has no identifier to tell two apart. Each beat
/// of a burst is one AXI-Lite transaction, at the burst's address
/// moved on by a beat's width per beat, or not moved for a fixed
/// burst: a read's answers go up as the burst's beats, the last marked
/// last, and a write's answers are folded into one response, the first
/// error or `Okay`. A burst to no peripheral's range is answered
/// `DecErr` by the bridge itself, beat by beat, so a read still gets
/// every beat it asked for. Written in the lowered subset, so it is a
/// netlist too.
// begin{state}
#[derive(Trace, Default)]
pub struct @NAME@<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
@RANGES@
> {
    /// A burst is in progress, and the next waits for it.
    pub busy: Reg<Bit>,
    /// The burst is a read.
    pub rd: Reg<Bit>,
    /// The read address channel goes first when both offer.
    pub rfirst: Reg<Bit>,
    /// The burst's identifier, which every answer to it names.
    pub xid: Reg<U<I>>,
    /// The address of the current beat.
    pub addr: Reg<U<A>>,
    /// How far the address moves per beat: none for a fixed burst.
    pub stride: Reg<U<A>>,
    /// Beats left after this one.
    pub left: Reg<U<8>>,
    /// The burst's protection bits, given with every beat.
    pub prot: Reg<U<3>>,
    /// Which peripheral the burst decoded to, one bit each; none set
    /// is a hole, answered here.
    pub sel: Reg<U<@SW@>>,
    /// The beat's AXI-Lite request is out, and its answer awaited.
    pub sent: Reg<Bit>,
    /// A write burst's response so far: its first error, or `Okay`.
    pub wresp: Reg<Resp>,
}
// end{state}

// begin{ports}
#[lower]
impl<
    const A: usize,
    const D: usize,
    const S: usize,
    const I: usize,
@RANGES@
> Unit
    for @NAME@<
        A,
        D,
        S,
        I,
@RARGS@
    >
{
    async fn run(
        &mut self,
        (
            aw,
            ar,
            w,
@IN_NAMES@
        ): (
            Rx<Aw<A, I>>,
            Rx<Ar<A, I>>,
            Rx<W<D, S>>,
@IN_TYPES@
        ),
        (
@OUT_NAMES@
            b,
            r,
        ): (
@OUT_TYPES@
            Tx<B<I>>,
            Tx<R<D, I>>,
        ),
    ) {
        loop {
            DefaultClock::rising().await;
// end{ports}
// begin{accept}
            // A burst is taken when none is in progress, the two
            // address channels taking turns when both offer.
            let idle = !self.busy;
            let aw_off = aw.peek().is_some();
            let ar_off = ar.peek().is_some();
            let awh = aw.head();
            let arh = ar.head();
            let pick_ar = idle & (self.rfirst | !aw_off);
            let take_ar = pick_ar & ar_off;
            let take_aw = idle & aw_off & !take_ar;
            let _ = ar.recv_if(pick_ar);
            let _ = aw.recv_if(idle & !take_ar);
            // Where it goes: a bit per peripheral, the lowest range
            // that matches winning, and none for a hole.
@AR_DECODE@
@AW_DECODE@
            let new_sel = mux(take_ar, ar_sel0, aw_sel0);
            // How far the address moves per beat: a beat's width, or
            // nothing for a fixed burst.
            let size = mux(take_ar, arh.size, awh.size);
            let kind = mux(take_ar, arh.burst, awh.burst);
            let fixed = kind == BurstKind::Fixed;
            let width = U::<A>::from(1u8) << (size.raw() as usize);
            let new_stride = mux(fixed, U::<A>::from(0u8), width);
// end{accept}
// begin{beats}
            // A beat's request, to the peripheral the burst decoded
            // to. A write needs the burst's next beat and room on both
            // of that peripheral's channels; a read needs room on its
            // one. A hole needs no room, and a write's beat to a hole
            // is taken and dropped.
            let cur = self.sel.get();
            let wbeat = self.busy & !self.rd & !self.sent;
            let rbeat = self.busy & self.rd & !self.sent;
            let w_off = w.peek().is_some();
            let wh = w.head();
@W_ROOM@
@AR_ROOM@
            let w_go = wbeat & w_off & w_room0;
            let _ = w.recv_if(wbeat & w_room0);
            let ar_go = rbeat & ar_room0;
@SENDS@
// end{beats}
// begin{answers}
            // The answer to the beat, from that peripheral, or the
            // bridge's own `DecErr` for a hole, there at once.
@HEADS@
@B_OFF@
@B_RESP@
@R_OFF@
@R_DATA@
@R_RESP@
            let last = self.left == 0;
            // A write's answer is taken when the burst's own response
            // has room, or when more beats are to come.
            let wwait = self.busy & !self.rd & self.sent;
            let b_can = wwait & (b.ready() | !last);
            let b_done = b_can & b_off0;
            // A read's answer goes straight up as the burst's beat.
            let rwait = self.busy & self.rd & self.sent;
            let r_can = rwait & r.ready();
            let r_done = r_can & r_off0;
@TAKES@
            // The burst's response keeps its first error.
            let clean = self.wresp.get() == Resp::Okay;
            let merged = mux(clean, b_resp0, self.wresp.get());
            if (b_done & last).to_bool() {
                b.send(B {
                    id: self.xid.get(),
                    resp: merged,
                });
            }
            if r_done.to_bool() {
                r.send(R {
                    id: self.xid.get(),
                    data: r_data0,
                    resp: r_resp0,
                    last: Bit::from(last),
                });
            }
// end{answers}
// begin{drives}
            let done = b_done | r_done;
            with!(self <= {
                (take_ar | take_aw) ? {
                    busy: Bit::One,
                    rd: take_ar,
                    rfirst: !take_ar,
                    xid: mux(take_ar, arh.id, awh.id),
                    addr: mux(take_ar, arh.addr, awh.addr),
                    stride: new_stride,
                    left: mux(take_ar, arh.len, awh.len),
                    prot: mux(take_ar, arh.prot, awh.prot),
                    sel: new_sel,
                    sent: Bit::Zero,
                    wresp: Resp::Okay,
                },
                (w_go | ar_go) ? { sent: Bit::One },
                done ? {
                    sent: Bit::Zero,
                    addr: self.addr.get() + self.stride.get(),
                    left: self.left.get() - 1,
                    wresp: merged,
                },
                (done & last) ? { busy: Bit::Zero },
            });
// end{drives}
        }
    }
}
}
pub use @MODULE@::@NAME@;
"#;
    template
        .replace("@AR_DECODE@", &decode("ar"))
        .replace("@AW_DECODE@", &decode("aw"))
        .replace(
            "@W_ROOM@",
            &chain(
                "w_room",
                "cur",
                &|i| format!("aw{i}.ready() & w{i}.ready()"),
                "Bit::One",
            ),
        )
        .replace(
            "@AR_ROOM@",
            &chain("ar_room", "cur", &|i| format!("ar{i}.ready()"), "Bit::One"),
        )
        .replace("@SENDS@", &sends)
        .replace("@HEADS@", &heads)
        .replace(
            "@B_OFF@",
            &chain("b_off", "cur", &|i| format!("off_b{i}"), "Bit::One"),
        )
        .replace(
            "@B_RESP@",
            &chain("b_resp", "cur", &|i| format!("b{i}h.resp"), "Resp::DecErr"),
        )
        .replace(
            "@R_OFF@",
            &chain("r_off", "cur", &|i| format!("off_r{i}"), "Bit::One"),
        )
        .replace(
            "@R_DATA@",
            &chain(
                "r_data",
                "cur",
                &|i| format!("r{i}h.data"),
                "U::<D>::from(0u8)",
            ),
        )
        .replace(
            "@R_RESP@",
            &chain("r_resp", "cur", &|i| format!("r{i}h.resp"), "Resp::DecErr"),
        )
        .replace("@TAKES@", &takes)
        .replace("@IN_NAMES@", &in_names)
        .replace("@IN_TYPES@", &in_types)
        .replace("@OUT_NAMES@", &out_names)
        .replace("@OUT_TYPES@", &out_types)
        .replace("@RANGES@", &ranges)
        .replace("@RARGS@", &rargs)
        .replace("@MODULE@", &module)
        .replace("@NAME@", name)
        .replace("@SW@", &sw.to_string())
        .replace("@N@", &n.to_string())
}

/// `lite_bridge!(LiteBridge2, 2)`: an AXI4 to AXI-Lite bridge of two
/// peripherals, named. One AXI4 link's five channels come in and five
/// AXI-Lite channels go out per peripheral; a peripheral's address
/// range is a `BASE` and a `MASK` const parameter, as the router's
/// are. Written out per count, from one to eight, since the lowering
/// reads a body and not a loop over ports; `TXHDL_MACRO_DUMP` names a
/// directory to write the text to.
#[proc_macro]
pub fn lite_bridge(input: TokenStream) -> TokenStream {
    let toks: Vec<TokenTree> = input.into_iter().collect();
    let (name, n) = match toks.as_slice() {
        [TokenTree::Ident(name), TokenTree::Punct(c), TokenTree::Literal(n)]
            if c.as_char() == ',' =>
        {
            (name.to_string(), n.to_string().parse::<usize>().ok())
        }
        _ => return err(Span::call_site(), "expected `lite_bridge!(Name, N)`"),
    };
    let Some(n) = n.filter(|n| (1..=8).contains(n)) else {
        return err(
            toks[2].span(),
            "a bridge has from one to eight peripherals",
        );
    };
    let text = lite_bridge_text(&name, n);
    if let Ok(dir) = std::env::var("TXHDL_MACRO_DUMP") {
        let _ = std::fs::write(format!("{dir}/lite_bridge_{name}.rs"), &text);
    }
    let module = format!("lite_bridge{n}");
    let with_source = text.replacen(
        &format!("pub mod {module} {{\n"),
        &format!(
            "pub mod {module} {{\n\
             /// The text of this module, as `lite_bridge!` wrote it.\n\
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
/// The same as [`split_type_commas`], over a slice rather than a
/// group: the generic parameters of a function, which are tokens
/// between a `<` and its `>` and not a group of their own.
fn split_type_commas_slice(ts: &[TokenTree]) -> Vec<Vec<TokenTree>> {
    let mut out = vec![Vec::new()];
    let mut depth = 0usize;
    for t in ts {
        match t {
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
        out.last_mut().unwrap().push(t.clone());
    }
    out.retain(|v| !v.is_empty());
    out
}

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

/// The suffixes Rust allows on an integer literal, longest first so
/// that `u128` is not read as `u1` followed by `28`.
const INT_SUFFIXES: &[&str] = &[
    "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16",
    "u8", "i8",
];

/// A literal's value, read as Rust reads it: any of the four radices,
/// with or without a suffix. `str::parse` takes decimal digits and
/// nothing else, so it refused `0x10` and `16u32` alike, and this
/// macro needs the value at macro time rather than in generated code,
/// where a cast would do. See issue 200.
fn lit_value(text: &str) -> Option<u128> {
    let t = text.replace('_', "");
    let (radix, body) = match t.get(..2) {
        Some("0x" | "0X") => (16, &t[2..]),
        Some("0b" | "0B") => (2, &t[2..]),
        Some("0o" | "0O") => (8, &t[2..]),
        _ => (10, &t[..]),
    };
    let end = body
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(body.len());
    let (digits, suffix) = body.split_at(end);
    if digits.is_empty() {
        return None;
    }
    if !suffix.is_empty() && !INT_SUFFIXES.contains(&suffix) {
        return None;
    }
    u128::from_str_radix(digits, radix).ok()
}

/// `name`, `LIT`, `U::from(LIT)` or `U::<W>::from(LIT)`.
fn parse_arg(ts: &[TokenTree]) -> Result<Arg, String> {
    match ts {
        [TokenTree::Ident(id)] if id.to_string() != "U" => {
            Ok(Arg::Name(id.to_string()))
        }
        [TokenTree::Literal(l)] => {
            let t = l.to_string();
            let v = lit_value(&t).ok_or_else(|| {
                format!("`{t}` is not a whole number an argument can be")
            })?;
            Ok(Arg::Lit(v, None))
        }
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
                .and_then(lit_value)
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
    let r = text.trim_start_matches(['-', '>']).trim_start_matches("U<");
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
        [TokenTree::Ident(s), TokenTree::Punct(_), TokenTree::Ident(f), TokenTree::Group(g)]
            if matches!(s.to_string().as_str(), "self" | "this")
                && g.delimiter() == Delimiter::Bracket =>
        {
            indexed_reg(&f.to_string(), g, None)
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
    /// The names of its const parameters, in order, which a call binds
    /// by position through its turbofish.
    consts: Vec<String>,
    lets: Vec<(String, String)>,
    value: String,
    /// Why this helper cannot be inlined, if it cannot: a tuple `let`
    /// the scan could not take apart. The message is given where the
    /// helper is called, since that is the line a reader can act on.
    refused: Vec<String>,
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
    /// Each helper's parameters with the text of their types, by the
    /// helper's name, so a field of a struct it takes can be sliced out
    /// (issue 504). Kept beside `HELPERS` rather than in `Helper`,
    /// which the register maps build as well.
    static HELPER_TYPES: std::cell::RefCell<Vec<(String, Params)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The names in scope whose value is a struct of known type, with
    /// the type: a helper's parameters while its body is read, so that
    /// `p.f` is a field of `p` (issue 504).
    static TYPED: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The ports of the unit being lowered, each with the text of its
    /// value's type, so a field of a port's value can be sliced out.
    static PTYPES: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The sides of the unit that are structs implementing `Ports`,
    /// each by name with its type's text.
    static BUNDLES: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The ports the body reads through such a side, `side.f`, each
    /// with the side's type and the field, so a field of its value
    /// finds its layout.
    static BUNDLED: std::cell::RefCell<Vec<(String, String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The names of the ports of the unit being lowered, for the check
    /// below: an expression that names one of them is not a constant.
    static PNAMES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The wires an inlined call asked for, each a name and what
    /// drives it, waiting for the statement being lowered to take
    /// them. An inlined function used to paste an argument's whole
    /// expression wherever the body read it, so a body that read a
    /// parameter three times held three copies and eight nested calls
    /// held six thousand: issue 126. Now an argument the body reads
    /// more than once, and a `let` inside it read more than once,
    /// becomes a wire named here and read by name, so each expression
    /// is written once however deep the calls go.
    static INLINED: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// How many wires the inlining has named in this lowering, so the
    /// next one gets a name no other has.
    static INLINED_N: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A name no other wire of this lowering has, for a value an inlined
/// call has to hold: the function's name, the parameter's or `let`'s,
/// and a number.
fn inline_name(f: &str, n: &str) -> String {
    let k = INLINED_N.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    format!("{f}_{n}_{k}")
}

/// Whether an expression the lowering wrote costs so little to read
/// again that a wire would cost more: a name, a number, a constant of
/// the build, or a value of a node or two, such as a bit or a field
/// of one of those. A name for `h(12)` is longer than `h(12)` and
/// says less; a name for a tree is shorter than the tree and is read
/// in one place, which is the whole of issue 126.
fn is_cheap(e: &str) -> bool {
    e.starts_with("NlE::Name(")
        || e.starts_with("NlE::Num(")
        || e.starts_with("NlE::Bits(")
        || e.starts_with("::txhdl::netlist::lit(")
        || e.matches("NlE::").count() <= 2
}

/// How many times `name` is read as an identifier in `texts`.
fn reads_of(name: &str, texts: &[&str]) -> usize {
    fn walk(ts: TokenStream, name: &str, n: &mut usize) {
        for t in ts {
            match t {
                TokenTree::Ident(id) if id.to_string() == name => *n += 1,
                TokenTree::Group(g) => walk(g.stream(), name, n),
                _ => {}
            }
        }
    }
    let mut n = 0;
    for t in texts {
        if let Ok(ts) = t.parse::<TokenStream>() {
            walk(ts, name, &mut n);
        }
    }
    n
}

/// The name of a signal the expression mentions, if it mentions one: a
/// port of the unit, a name a `let` bound, or `self`, which reaches a
/// register. An expression the lowering does not recognise otherwise
/// becomes `lit`, a constant evaluated where `lowered` runs, and these
/// names mean nothing there: `rustc` then reports them as missing
/// variables, pointing at the `#[lower]` attribute with nothing to say
/// which expression it was. That is issue 128.
fn names_a_signal(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Option<String> {
    for (i, t) in ts.iter().enumerate() {
        match t {
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                if let Some(n) = names_a_signal(&inner, subst) {
                    return Some(n);
                }
            }
            TokenTree::Ident(id) => {
                let n = id.to_string();
                if n == "self" {
                    return Some(n);
                }
                // A path's segments are not signals: neither the
                // segment after `::` nor the one before it.
                let after = i >= 2
                    && matches!(&ts[i - 1], TokenTree::Punct(p) if p.as_char() == ':');
                let before = i + 2 < ts.len()
                    && matches!(&ts[i + 1], TokenTree::Punct(p) if p.as_char() == ':')
                    && matches!(&ts[i + 2], TokenTree::Punct(p) if p.as_char() == ':');
                if after || before {
                    continue;
                }
                // A loop's index is a number where `lowered` runs, since
                // the loop is unrolled there, so a constant may use it.
                let index = subst
                    .iter()
                    .rev()
                    .find(|(k, _)| *k == n)
                    .is_some_and(|(_, v)| v.starts_with("NlE::Num("));
                if index {
                    continue;
                }
                if subst.iter().any(|(k, _)| *k == n)
                    || PNAMES.with(|p| p.borrow().contains(&n))
                {
                    return Some(n);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether `ts` is `T::CONST[k]` or `T::CONST[k].f`: a path of at
/// least two segments whose last is written in capitals, an index in
/// brackets, and at most a tuple field after it. Such a constant is a
/// trait's associated array, an address map say (issue 593), and its
/// item is a number when `lowered` runs.
fn assoc_const_item(ts: &[TokenTree]) -> bool {
    let mut n = ts.len();
    if n >= 2 {
        if let (Some(TokenTree::Punct(d)), Some(TokenTree::Literal(_))) =
            (ts.get(n - 2), ts.get(n - 1))
        {
            if d.as_char() == '.' {
                n -= 2;
            }
        }
    }
    let Some(TokenTree::Group(g)) = ts.get(n.wrapping_sub(1)) else {
        return false;
    };
    if g.delimiter() != Delimiter::Bracket || n < 4 {
        return false;
    }
    let Some(TokenTree::Ident(leaf)) = ts.get(n - 2) else {
        return false;
    };
    let leaf = leaf.to_string();
    let upper = leaf.chars().any(|c| c.is_ascii_uppercase())
        && leaf
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit());
    upper && punct_at(ts, n - 3, ':') && punct_at(ts, n - 4, ':')
}

/// `lit(text)`, unless the text names a signal, in which case the
/// expression is refused here rather than by `rustc` at the attribute.
fn as_lit(
    ts: &[TokenTree],
    text: &str,
    subst: &[(String, String)],
) -> Result<String, String> {
    match names_a_signal(ts, subst) {
        Some(n) => Err(format!(
            "cannot lower `{text}`: it names `{n}`, which is a signal of \
             the unit, so the expression cannot be a constant"
        )),
        None => Ok(format!("::txhdl::netlist::lit({text})")),
    }
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
        let side = BUNDLED.with(|d| {
            d.borrow()
                .iter()
                .find(|(n, _, _)| n == port)
                .map(|(_, t, fld)| (t.clone(), fld.clone()))
        });
        return match side {
            Some((s, fld)) => Ok(format!(
                "::txhdl::netlist::bundle_field::<{s}>({base}, \"{fld}\", \
                 \"{f}\")"
            )),
            None => {
                Err(format!("`{port}` is not a port, so `.{f}` has no layout"))
            }
        };
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
    HELPER_TYPES.with(|t| t.borrow_mut().clear());
    scan_helpers(stream.into_iter().collect())
}

/// The functions under `#[lower]` among `ts`, a file's tokens or one
/// function's with its marker put back.
fn scan_helpers(ts: Vec<TokenTree>) -> Vec<Helper> {
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
        // Another attribute, or a doc comment, may stand between the
        // marker and the function: Rust takes attributes in any order,
        // and a scan that insisted on the marker being last left the
        // function uninlined, with the call refused as not lowered.
        // That is issue 234.
        while matches!(ts.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '#')
            && matches!(ts.get(j + 1), Some(TokenTree::Group(g))
                if g.delimiter() == Delimiter::Bracket)
        {
            j += 2;
        }
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
        // The generic parameters, if it has any: the const ones are
        // kept, in order, since a call binds them by position (issue
        // 127).
        let mut consts: Vec<String> = Vec::new();
        if matches!(ts.get(j), Some(TokenTree::Punct(p)) if p.as_char() == '<')
        {
            let start = j;
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
            let inner = &ts[start + 1..j.saturating_sub(1)];
            for g in split_type_commas_slice(inner) {
                if let [TokenTree::Ident(k), TokenTree::Ident(n), ..] =
                    g.as_slice()
                {
                    if k.to_string() == "const" {
                        consts.push(n.to_string());
                    }
                }
            }
        }
        let Some(TokenTree::Group(params)) = ts.get(j) else {
            i += 1;
            continue;
        };
        let typed: Vec<(String, String)> = split_commas(params)
            .iter()
            .filter_map(|p| match (p.first(), punct_at(p, 1, ':')) {
                (Some(TokenTree::Ident(n)), true) => {
                    Some((n.to_string(), text_of(&p[2..])))
                }
                (Some(TokenTree::Ident(n)), false) => {
                    Some((n.to_string(), String::new()))
                }
                _ => None,
            })
            .collect();
        let params: Vec<String> =
            typed.iter().map(|(n, _)| n.clone()).collect();
        HELPER_TYPES.with(|t| t.borrow_mut().push((name.to_string(), typed)));
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
        let mut refused = Vec::new();
        let mut value = String::new();
        for st in statements(body) {
            let st: Vec<TokenTree> = st.into_iter().collect();
            if st.len() > 3 && is_ident(&st[0], "let") {
                // A tuple `let` stands for one `let` per name, and it
                // is taken apart here: the name of a helper's `let` is
                // what its value is substituted for, so a name of
                // `(x, y)` would substitute for nothing and leave `x`
                // and `y` in the netlist with nothing declaring them,
                // which is issue 159.
                let names = match &st[1] {
                    TokenTree::Group(g)
                        if g.delimiter() == Delimiter::Parenthesis =>
                    {
                        Some(split_commas(g))
                    }
                    _ => None,
                };
                match names {
                    None => lets.push((st[1].to_string(), text_of(&st[3..]))),
                    Some(names) => {
                        let values = match &st[3] {
                            TokenTree::Group(g)
                                if g.delimiter() == Delimiter::Parenthesis
                                    && st.len() == 4 =>
                            {
                                Some(split_commas(g))
                            }
                            _ => None,
                        };
                        let single = |t: &Vec<TokenTree>| match t.as_slice() {
                            [TokenTree::Ident(id)] => Some(id.to_string()),
                            _ => None,
                        };
                        let named: Option<Vec<String>> =
                            names.iter().map(single).collect();
                        match (named, values) {
                            (Some(ns), Some(vs)) if ns.len() == vs.len() => {
                                for (n, v) in ns.iter().zip(&vs) {
                                    lets.push((n.clone(), text_of(v)));
                                }
                            }
                            _ => refused.push(format!(
                                "the tuple `let` of `{name}` binds {n} \
                                 names to something that is not a tuple of \
                                 {n} values: a helper's `let` is a \
                                 substitution, so write one `let` per name \
                                 (issue 159)",
                                n = names.len()
                            )),
                        }
                    }
                }
            } else {
                value = text_of(&st);
            }
        }
        out.push(Helper {
            name: name.to_string(),
            params,
            consts,
            lets,
            value,
            refused,
        });
        i = j + k + 1;
    }
    out.extend(regmap::helpers_in(&ts));
    out
}

/// A struct whose fields may be a unit's ports, as the file declares
/// it: its generic parameters, each by name with its default if it has
/// one, a lifetime left out, and its fields in declaration order, each
/// by name with its type as written.
struct PortStruct {
    name: String,
    params: Vec<(String, Option<Vec<TokenTree>>)>,
    fields: Vec<(String, Vec<TokenTree>)>,
}

/// The arguments between `<` at `ts[at]` and its `>`, split on the
/// commas at their own depth, and the index past the `>`.
fn angle_args(ts: &[TokenTree], at: usize) -> (Vec<Vec<TokenTree>>, usize) {
    let mut out = vec![Vec::new()];
    let mut depth = 0usize;
    let mut j = at;
    while j < ts.len() {
        let t = &ts[j];
        j += 1;
        match t {
            TokenTree::Punct(p) if p.as_char() == '<' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            TokenTree::Punct(p) if p.as_char() == '>' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 1 => {
                out.push(Vec::new());
                continue;
            }
            _ => {}
        }
        out.last_mut().unwrap().push(t.clone());
    }
    out.retain(|v| !v.is_empty());
    (out, j)
}

/// The structs of a file, and the roles of its `interface!`s, which
/// are structs too: a role's member of `Signal<T>` is an `Out<T>` where
/// the role drives it and an `In<T>` where it reads it, and of
/// `Chan<T>` a `Tx<T>` or an `Rx<T>`. The macro reads the file for the
/// same reason `find_helpers` does: the struct is not in the item.
fn find_port_structs(file: Option<std::path::PathBuf>) -> Vec<PortStruct> {
    let Some(text) = file.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let Ok(stream) = text.parse::<TokenStream>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    port_structs_in(stream, &mut out);
    out
}

fn port_structs_in(stream: TokenStream, out: &mut Vec<PortStruct>) {
    let ts: Vec<TokenTree> = stream.into_iter().collect();
    let brace = |t: Option<&TokenTree>| match t {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
            Some(g.clone())
        }
        _ => None,
    };
    // `#[doc]`s and `pub`, `pub(crate)` before a field's name.
    let past_attrs = |f: &[TokenTree]| -> usize {
        let mut k = 0;
        while punct_at(f, k, '#') {
            k += 2;
        }
        if f.get(k).is_some_and(|t| is_ident(t, "pub")) {
            k += 1;
            if matches!(f.get(k), Some(TokenTree::Group(g))
                if g.delimiter() == Delimiter::Parenthesis)
            {
                k += 1;
            }
        }
        k
    };
    for i in 0..ts.len() {
        if is_ident(&ts[i], "struct") {
            let Some(TokenTree::Ident(name)) = ts.get(i + 1) else {
                continue;
            };
            let mut j = i + 2;
            let mut params = Vec::new();
            if punct_at(&ts, j, '<') {
                let (args, end) = angle_args(&ts, j);
                j = end;
                for a in args {
                    if punct_at(&a, 0, '\'') {
                        continue;
                    }
                    let n = if is_ident(&a[0], "const") { 1 } else { 0 };
                    let Some(TokenTree::Ident(pn)) = a.get(n) else {
                        continue;
                    };
                    let default = a
                        .iter()
                        .position(|t| {
                            matches!(t, TokenTree::Punct(p)
                            if p.as_char() == '=')
                        })
                        .map(|e| a[e + 1..].to_vec());
                    params.push((pn.to_string(), default));
                }
            }
            // Past a `where` clause, to the braces; a tuple struct has
            // none and names no fields.
            while j < ts.len()
                && brace(ts.get(j)).is_none()
                && !punct_at(&ts, j, ';')
                && !matches!(&ts[j], TokenTree::Group(g)
                    if g.delimiter() == Delimiter::Parenthesis)
            {
                j += 1;
            }
            let Some(body) = brace(ts.get(j)) else {
                continue;
            };
            let mut fields = Vec::new();
            for f in split_type_commas(&body) {
                let k = past_attrs(&f);
                if let (Some(TokenTree::Ident(n)), true) =
                    (f.get(k), punct_at(&f, k + 1, ':'))
                {
                    fields.push((n.to_string(), f[k + 2..].to_vec()));
                }
            }
            out.push(PortStruct {
                name: name.to_string(),
                params,
                fields,
            });
        } else if is_ident(&ts[i], "interface") && punct_at(&ts, i + 1, '!') {
            if let Some(g) = brace(ts.get(i + 2)) {
                interface_roles(&g, out);
            }
        } else if let TokenTree::Group(g) = &ts[i] {
            port_structs_in(g.stream(), out);
        }
    }
}

/// The roles of one `interface! { Name { members } role R { .. } .. }`
/// as port structs, a role whose members are not all signals and
/// channels left out.
fn interface_roles(g: &Group, out: &mut Vec<PortStruct>) {
    let ts: Vec<TokenTree> = g.stream().into_iter().collect();
    let Some(TokenTree::Group(members)) = ts.get(1) else {
        return;
    };
    let members: Vec<(String, Vec<TokenTree>)> = split_type_commas(members)
        .into_iter()
        .filter_map(|m| match (m.first(), punct_at(&m, 1, ':')) {
            (Some(TokenTree::Ident(n)), true) => {
                Some((n.to_string(), m[2..].to_vec()))
            }
            _ => None,
        })
        .collect();
    let mut i = 2;
    while i + 2 < ts.len() && is_ident(&ts[i], "role") {
        let (Some(TokenTree::Ident(name)), Some(TokenTree::Group(ends))) =
            (ts.get(i + 1), ts.get(i + 2))
        else {
            break;
        };
        i += 3;
        let mut fields = Vec::new();
        for e in split_commas(ends) {
            let [TokenTree::Ident(dir), TokenTree::Ident(m)] = e.as_slice()
            else {
                break;
            };
            let Some((_, ty)) =
                members.iter().find(|(n, _)| *n == m.to_string())
            else {
                break;
            };
            let Some(lt) = ty.iter().position(|t| {
                matches!(t,
                TokenTree::Punct(p) if p.as_char() == '<')
            }) else {
                break;
            };
            let drives = dir.to_string() == "out";
            let kind = match (ty[..lt].last(), drives) {
                (Some(t), true) if is_ident(t, "Signal") => "Out",
                (Some(t), false) if is_ident(t, "Signal") => "In",
                (Some(t), true) if is_ident(t, "Chan") => "Tx",
                (Some(t), false) if is_ident(t, "Chan") => "Rx",
                _ => break,
            };
            let mut port = vec![TokenTree::Ident(Ident::new(kind, m.span()))];
            port.extend(ty[lt..].iter().cloned());
            fields.push((m.to_string(), port));
        }
        if fields.len() == split_commas(ends).len() {
            out.push(PortStruct {
                name: name.to_string(),
                params: Vec::new(),
                fields,
            });
        }
    }
}

/// Tokens with every name in `map` replaced by its tokens, into every
/// group: a struct's field types with the struct's parameters bound
/// to the arguments the side names.
fn substitute(
    ts: &[TokenTree],
    map: &[(String, Vec<TokenTree>)],
) -> Vec<TokenTree> {
    let mut out = Vec::new();
    for t in ts {
        match t {
            TokenTree::Ident(id) => {
                match map.iter().find(|(n, _)| *n == id.to_string()) {
                    Some((_, v)) => out.extend(v.iter().cloned()),
                    None => out.push(t.clone()),
                }
            }
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                let mut n = Group::new(
                    g.delimiter(),
                    substitute(&inner, map).into_iter().collect(),
                );
                n.set_span(g.span());
                out.push(TokenTree::Group(n));
            }
            _ => out.push(t.clone()),
        }
    }
    out
}

/// What a side whose type is not a port is: a struct of this file,
/// read here, each field with its type; or a struct of anywhere else,
/// which must implement `Ports`, its fields found when `lowered` runs.
enum StructSide {
    Read(Vec<(String, String)>),
    Ports,
}

/// The ports of a side whose type is a struct of the file, each field
/// with its type, the struct's parameters bound to the side's
/// arguments; `StructSide::Ports` for a struct declared elsewhere,
/// and `None` for a side that is a port or a tuple.
fn port_struct_fields(
    ty: &[TokenTree],
    structs: &[PortStruct],
) -> Result<Option<StructSide>, String> {
    // An array of ports, `[Rx<T>; N]`, is a struct of ports whose
    // fields are its indices (issue 500).
    if let [TokenTree::Group(g)] = ty {
        if g.delimiter() == Delimiter::Bracket {
            return Ok(Some(StructSide::Ports));
        }
    }
    let lt = ty
        .iter()
        .position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '<'))
        .unwrap_or(ty.len());
    let Some(TokenTree::Ident(head)) = ty[..lt].last() else {
        return Ok(None);
    };
    let head = head.to_string();
    if ["In", "Out", "Tx", "Rx", "Pad"].contains(&head.as_str()) {
        return Ok(None);
    }
    let Some(s) = structs.iter().find(|s| s.name == head) else {
        return Ok(Some(StructSide::Ports));
    };
    let args: Vec<Vec<TokenTree>> = if lt < ty.len() {
        angle_args(ty, lt)
            .0
            .into_iter()
            .filter(|a| !punct_at(a, 0, '\''))
            .collect()
    } else {
        Vec::new()
    };
    if args.len() > s.params.len() {
        return Err(format!("`{head}` takes {} parameters", s.params.len()));
    }
    let mut map = Vec::new();
    for (k, (pn, default)) in s.params.iter().enumerate() {
        let v = match (args.get(k), default) {
            (Some(a), _) => a.clone(),
            (None, Some(d)) => d.clone(),
            (None, None) => {
                return Err(format!("`{head}` needs its parameter `{pn}`"))
            }
        };
        map.push((pn.clone(), v));
    }
    Ok(Some(StructSide::Read(
        s.fields
            .iter()
            .map(|(n, t)| {
                let t: String =
                    substitute(t, &map).iter().map(|t| t.to_string()).collect();
                (n.clone(), t)
            })
            .collect(),
    )))
}

/// `run`'s body with every `side.field` of a side that is a port
/// struct written as the port `field`, so the lowering reads it as it
/// reads a port named in a tuple. `self.side.field` is left alone.
fn port_fields(
    ts: TokenStream,
    bound: &[(String, Vec<String>)],
) -> TokenStream {
    let ts: Vec<TokenTree> = ts.into_iter().collect();
    let mut out: Vec<TokenTree> = Vec::new();
    let mut i = 0;
    while i < ts.len() {
        // `ins[k]` of an array of ports is the port `ins_k`, and
        // `ins[i]` with `i` a loop's variable a name `dyn_names`
        // formats with `i` once `lowered` runs (issue 500).
        if let (TokenTree::Ident(b), Some(TokenTree::Group(g))) =
            (&ts[i], ts.get(i + 1))
        {
            let after_dot = i > 0 && punct_at(&ts, i - 1, '.');
            let b = b.to_string();
            let wild = bound.iter().any(|(n, fs)| *n == b && fs == &["*"]);
            if wild && !after_dot && g.delimiter() == Delimiter::Bracket {
                let it: Vec<TokenTree> = g.stream().into_iter().collect();
                let field = match it.as_slice() {
                    [TokenTree::Literal(k)] => Some(k.to_string()),
                    [TokenTree::Ident(v)] => Some(format!("{DYN}{v}{DYN_END}")),
                    _ => None,
                };
                if let Some(field) = field {
                    let port = if field.starts_with(DYN) {
                        format!("{b}{field}")
                    } else {
                        format!("{b}_{field}")
                    };
                    let ty = BUNDLES.with(|s| {
                        s.borrow()
                            .iter()
                            .find(|(n, _)| *n == b)
                            .map(|(_, t)| t.clone())
                    });
                    BUNDLED.with(|d| {
                        let mut d = d.borrow_mut();
                        if let (Some(ty), false) =
                            (ty, d.iter().any(|(p, _, _)| *p == port))
                        {
                            d.push((port.clone(), ty, field.clone()));
                        }
                    });
                    out.push(TokenTree::Ident(Ident::new(&port, g.span())));
                    i += 2;
                    continue;
                }
            }
        }
        if let (TokenTree::Ident(b), true, Some(TokenTree::Ident(f))) =
            (&ts[i], punct_at(&ts, i + 1, '.'), ts.get(i + 2))
        {
            let after_dot = i > 0 && punct_at(&ts, i - 1, '.');
            let (b, f) = (b.to_string(), f.to_string());
            let wild = bound.iter().any(|(n, fs)| *n == b && fs == &["*"]);
            // A side implementing `Ports`: `bus.aw` is the port
            // `bus_aw`, named for the side as well as the field, so a
            // unit may take two sides of one type and two units of one
            // run may take the same type without their ports meeting.
            if wild && !after_dot {
                // A struct of ports may hold another, so `bus.pins.awid`
                // is the port `bus_pins_awid`: every name of the chain up
                // to a method call is part of the port's (issue 498). A
                // port has no fields of its own to read without one.
                let mut f = f;
                let mut j = i + 3;
                while punct_at(&ts, j, '.') {
                    let (Some(TokenTree::Ident(g)), next) =
                        (ts.get(j + 1), ts.get(j + 2))
                    else {
                        break;
                    };
                    let call = matches!(next, Some(TokenTree::Group(p))
                        if p.delimiter() == Delimiter::Parenthesis)
                        || punct_at(&ts, j + 2, ':');
                    if call {
                        break;
                    }
                    f = format!("{f}_{g}");
                    j += 2;
                }
                let port = format!("{b}_{f}");
                let ty = BUNDLES.with(|s| {
                    s.borrow()
                        .iter()
                        .find(|(n, _)| *n == b)
                        .map(|(_, t)| t.clone())
                });
                BUNDLED.with(|d| {
                    let mut d = d.borrow_mut();
                    if let (Some(ty), false) =
                        (ty, d.iter().any(|(p, _, _)| *p == port))
                    {
                        d.push((port.clone(), ty, f.clone()));
                    }
                });
                out.push(TokenTree::Ident(Ident::new(&port, ts[i + 2].span())));
                i = j;
                continue;
            }
            if !after_dot
                && bound.iter().any(|(n, fs)| *n == b && fs.contains(&f))
            {
                // A field that is a struct of ports nested in the side:
                // what follows is read from it as from a side of its own.
                if bound.iter().any(|(n, fs)| *n == f && fs == &["*"]) {
                    let rest: TokenStream =
                        ts[i + 2..].iter().cloned().collect();
                    out.extend(port_fields(rest, bound));
                    return out.into_iter().collect();
                }
                out.push(ts[i + 2].clone());
                i += 3;
                continue;
            }
        }
        match &ts[i] {
            TokenTree::Group(g) => {
                let mut n =
                    Group::new(g.delimiter(), port_fields(g.stream(), bound));
                n.set_span(g.span());
                out.push(TokenTree::Group(n));
            }
            t => out.push(t.clone()),
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// A helper's parameters, each by name with the text of its type.
type Params = Vec<(String, String)>;

/// The parameters of a helper, typed in `TYPED` while its body is read,
/// and taken off when this is dropped (issue 504). A const parameter in
/// a type is replaced by what the call gave for it.
struct Typed(usize);

impl Typed {
    fn of(helper: &str, consts: &[(String, String)]) -> Typed {
        let params = HELPER_TYPES.with(|t| {
            t.borrow()
                .iter()
                .rev()
                .find(|(n, _)| n == helper)
                .map(|(_, p)| p.clone())
                .unwrap_or_default()
        });
        TYPED.with(|t| {
            let mut t = t.borrow_mut();
            let n = t.len();
            for (p, ty) in params.into_iter().filter(|(_, ty)| !ty.is_empty()) {
                t.push((p, with_consts(&ty, consts)));
            }
            Typed(n)
        })
    }
}

impl Drop for Typed {
    fn drop(&mut self) {
        TYPED.with(|t| t.borrow_mut().truncate(self.0));
    }
}

/// A call of a function under `#[lower]`, inlined: its parameters
/// bound to the arguments, its `let`s to expressions of their own,
/// and its value the call's.
fn inline_helper(
    h: &Helper,
    args: &[String],
    consts: &[String],
) -> Result<String, String> {
    if args.len() != h.params.len() {
        return Err(format!("`{}` takes {} arguments", h.name, h.params.len()));
    }
    if consts.len() != h.consts.len() {
        return Err(format!(
            "`{}` takes {} const parameters and the call gives {}",
            h.name,
            h.consts.len(),
            consts.len()
        ));
    }
    // The const parameters are bound by position, as the value ones
    // are, by putting the call's expression in place of the name in
    // the body. The name itself means nothing where the unit is, and
    // binding by name is what issue 127 was.
    let cs: Vec<(String, String)> = h
        .consts
        .iter()
        .cloned()
        .zip(consts.iter().cloned())
        .collect();
    if let Some(why) = h.refused.first() {
        return Err(why.clone());
    }
    // What the body still has to read, after the binding being made:
    // the `let`s below it and the value. A name read once is pasted
    // where it is read, which is one copy; a name read more often
    // becomes a wire, so the expression is written once whatever the
    // body does with it, and a chain of calls grows by a wire a call
    // rather than by a power of the reads. That is issue 126.
    let rest = |from: usize| -> Vec<&str> {
        let mut v: Vec<&str> =
            h.lets[from..].iter().map(|(_, e)| e.as_str()).collect();
        v.push(h.value.as_str());
        v
    };
    // The binding of one name: the expression itself when it is read
    // once or costs nothing to read again, and otherwise a wire that
    // holds it, which the body reads by name.
    let bind = |n: &str, v: String, reads: usize| -> String {
        if reads < 2 || is_cheap(&v) {
            return v;
        }
        let w = inline_name(&h.name, n);
        INLINED.with(|ws| ws.borrow_mut().push((w.clone(), v)));
        ename(&w)
    };
    let mut s: Vec<(String, String)> = Vec::new();
    for (p, a) in h.params.iter().zip(args) {
        let reads = reads_of(p, &rest(0));
        s.push((p.clone(), bind(p, a.clone(), reads)));
    }
    // The parameters' types, for a field of one of them; taken off
    // again when the body has been read, whatever it returns.
    let _typed = Typed::of(&h.name, &cs);
    let toks = |text: &str| -> Result<Vec<TokenTree>, String> {
        with_consts(text, &cs)
            .parse::<TokenStream>()
            .map(|t| t.into_iter().collect())
            .map_err(|_| format!("cannot parse `{text}` in `{}`", h.name))
    };
    for (i, (n, e)) in h.lets.iter().enumerate() {
        let v = tr(&toks(e)?, &s)?;
        let reads = reads_of(n, &rest(i + 1));
        s.push((n.clone(), bind(n, v, reads)));
    }
    if h.value.is_empty() {
        return Err(format!("`{}` has no value", h.name));
    }
    tr(&toks(&h.value)?, &s)
}

/// `text` with each const parameter's name replaced by the expression
/// the call gave for it, in brackets so that its parts stay together.
fn with_consts(text: &str, consts: &[(String, String)]) -> String {
    fn walk(ts: TokenStream, consts: &[(String, String)]) -> String {
        let mut out = String::new();
        for t in ts {
            match t {
                TokenTree::Ident(id) => {
                    let n = id.to_string();
                    match consts.iter().find(|(k, _)| *k == n) {
                        Some((_, v)) => out.push_str(&format!("({v})")),
                        None => out.push_str(&n),
                    }
                }
                TokenTree::Group(g) => {
                    let (open, close) = match g.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::None => ("", ""),
                    };
                    out.push_str(open);
                    out.push_str(&walk(g.stream(), consts));
                    out.push_str(close);
                }
                // A punctuation token keeps its spacing: `==` and
                // `>>` are two joint tokens each, and a space between
                // them would make them two operators.
                TokenTree::Punct(p) => {
                    out.push(p.as_char());
                    if p.spacing() == Spacing::Alone {
                        out.push(' ');
                    }
                    continue;
                }
                t => out.push_str(&t.to_string()),
            }
            out.push(' ');
        }
        out
    }
    if consts.is_empty() {
        return text.to_string();
    }
    match text.parse::<TokenStream>() {
        Ok(stream) => walk(stream, consts),
        Err(_) => text.to_string(),
    }
}

fn tr(ts: &[TokenTree], subst: &[(String, String)]) -> Result<String, String> {
    // A trailing comma: the tokens of a call's argument carry it when
    // the formatter puts the argument on its own line, and it made the
    // expression end in a comma rather than in whatever it is. A chain
    // then stopped being a chain and became a constant, which is half
    // of issue 128.
    if let [rest @ .., TokenTree::Punct(p)] = ts {
        if p.as_char() == ',' {
            return tr(rest, subst);
        }
    }
    if ts.is_empty() {
        return Err("empty expression".into());
    }
    // An `if` or a `match` that yields a value (issue 496).
    if let Some(TokenTree::Ident(kw)) = ts.first() {
        match kw.to_string().as_str() {
            "if" => return tr_if(ts, subst),
            "match" => return tr_match(ts, subst),
            _ => {}
        }
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
        &["*", "/", "%"],
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
                // After another operator, `-` and `*` are unary: `a + -b`
                // is a sum, not a difference (issue 496).
                if (*op == "-" || *op == "*")
                    && matches!(ts[i - 1], TokenTree::Punct(_))
                {
                    continue;
                }
                let l = tr(&ts[..i], subst)?;
                let r = tr(&ts[i + n..], subst)?;
                return Ok(ebin(op, &l, &r));
            }
        }
    }
    // `(lo..=hi).contains(&x)`: `x` within the range (issue 496).
    if let [TokenTree::Group(r), TokenTree::Punct(dot), TokenTree::Ident(m), TokenTree::Group(args)] =
        ts
    {
        if r.delimiter() == Delimiter::Parenthesis
            && dot.as_char() == '.'
            && m.to_string() == "contains"
        {
            let rt: Vec<TokenTree> = r.stream().into_iter().collect();
            let at: Vec<TokenTree> = args.stream().into_iter().collect();
            let x = match at.as_slice() {
                [TokenTree::Punct(amp), rest @ ..] if amp.as_char() == '&' => {
                    rest
                }
                rest => rest,
            };
            let x = tr(x, subst)?;
            if let Some(c) = range_cond(&x, &rt, subst)? {
                return Ok(c);
            }
            return Err("`contains` lowers on a range of constants, \
                 `(lo..=hi).contains(&x)`"
                .into());
        }
    }
    // The last `as`, so a chain, `x as u8 as usize`, is a cast of a cast.
    if let Some(p) = ts.iter().rposition(
        |t| matches!(t, TokenTree::Ident(id) if id.to_string() == "as"),
    ) {
        // `x as uN` keeps the low `N` bits, as Rust does: the netlist
        // renders the value when it is no wider and its low bits when
        // it is. It used to drop the cast, which gave the right value
        // only because nothing narrowed (issue 496).
        let ty: String = ts[p + 1..].iter().map(|t| t.to_string()).collect();
        let bits = match ty.as_str() {
            "u8" => 8,
            "u16" => 16,
            "u32" => 32,
            "u64" | "usize" => 64,
            "u128" => 128,
            _ => {
                return Err(format!(
                    "`as {ty}` does not lower: a cast in a lowered body is to \
                     an unsigned integer, `u8` to `u128` or `usize`, whose \
                     width it keeps the low bits of; for a value of another \
                     width write `.resize::<N>()` or `.sext::<N>()`"
                ))
            }
        };
        let x = tr(&ts[..p], subst)?;
        return Ok(format!("NlE::cast({x}, {bits})"));
    }
    if let TokenTree::Punct(p) = &ts[0] {
        if p.as_char() == '!' {
            return Ok(format!("NlE::Not(Box::new({}))", tr(&ts[1..], subst)?));
        }
        // Negation is the two's complement at the operand's width,
        // `~x + 1`, which is what `0 - x` wraps to in Rust. A number has
        // no width to wrap at, so a negative one is refused (issue 496).
        if p.as_char() == '-' {
            if matches!(ts.get(1), Some(TokenTree::Literal(_))) {
                return Err(
                    "a negative number has no width in a lowered body: \
                     write the value it wraps to at its width, or subtract \
                     from a value, `x - 3`"
                        .into(),
                );
            }
            let x = tr(&ts[1..], subst)?;
            return Ok(ebin(
                "+",
                &format!("NlE::Not(Box::new({x}))"),
                "NlE::Num(1)",
            ));
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
                                    "NlE::slice({l}, {}, {})",
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
            if dot.as_char() == '.' && g.delimiter() == Delimiter::Parenthesis {
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
                        format!("NlE::index({l}, {})", a[0])
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
        // A number. The netlist's numbers are all `u128`, and a
        // reader writes a suffix to say the width in Rust's terms, so
        // the literal is cast rather than passed on as it stands: a
        // cast drops the suffix and still leaves Rust to say whether
        // the literal fits the type it names, so `300u8` is refused
        // where it is written. `u128::from` would not do, because a
        // literal with no suffix is then ambiguous between the
        // implementations and falls back to `i32`. See issue 166.
        [TokenTree::Literal(l)] => Ok(format!(
            "NlE::Num(({}) as u128)",
            l.to_string().replace('_', "")
        )),
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
        // One register of an array of them, `self.prio[i]`: the register
        // `prio_i`, the index a number or a loop's variable (issue 594).
        [TokenTree::Ident(s), TokenTree::Punct(_), TokenTree::Ident(f), TokenTree::Group(g)]
            if matches!(s.to_string().as_str(), "self" | "this")
                && g.delimiter() == Delimiter::Bracket =>
        {
            Ok(ename(&indexed_reg(&f.to_string(), g, Some(subst))?))
        }
        // A field of a port's value, `p.tag`: a slice of the port's data.
        [base @ .., TokenTree::Punct(dot), TokenTree::Ident(f)]
            if dot.as_char() == '.' && !base.is_empty() =>
        {
            let b = tr(base, subst)?;
            // A field of a struct whose type is known where it is named,
            // a helper's parameter: its bits are found from the type's
            // layout, whatever computed the value (issue 504).
            let typed = match base {
                [TokenTree::Ident(n)] => TYPED.with(|t| {
                    t.borrow()
                        .iter()
                        .rev()
                        .find(|(m, _)| *m == n.to_string())
                        .map(|(_, ty)| ty.clone())
                }),
                _ => None,
            };
            match typed {
                Some(ty) => {
                    Ok(format!("::txhdl::netlist::field::<{ty}>({b}, \"{f}\")"))
                }
                None => field_of(&b, &f.to_string()),
            }
        }
        // A struct literal, `Name { f: e, .. }`: its fields concatenated
        // in the order written, which must be the declaration's, the
        // first field highest, as `#[derive(Value)]` lays them out.
        [TokenTree::Ident(_), TokenTree::Group(g)]
            if g.delimiter() == Delimiter::Brace =>
        {
            let mut acc: Option<String> = None;
            for f in split_commas(g) {
                let colon = f.iter().position(
                    |t| matches!(t, TokenTree::Punct(p) if p.as_char() == ':'),
                );
                let e = match (colon, f.as_slice()) {
                    (Some(c), _) => tr(&f[c + 1..], subst)?,
                    // The shorthand, `Name { id }`: the field's value
                    // is the name itself, which is what Rust makes of
                    // it, and what `clippy::redundant_field_names`
                    // asks for. That is issue 235.
                    (None, [TokenTree::Ident(_)]) => tr(&f, subst)?,
                    (None, _) => {
                        return Err(
                            "a struct literal's field is `name: value`, \
                             or `name` on its own"
                                .into(),
                        )
                    }
                };
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
                        // The turbofish, if there is one: the
                        // expressions between `<` and its `>`, which
                        // bind the function's const parameters.
                        let mut cs: Vec<String> = Vec::new();
                        if turbo {
                            let mut depth = 0usize;
                            let mut end = 3;
                            while end < ts.len() {
                                match &ts[end] {
                                    TokenTree::Punct(p)
                                        if p.as_char() == '<' =>
                                    {
                                        depth += 1
                                    }
                                    TokenTree::Punct(p)
                                        if p.as_char() == '>' =>
                                    {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                end += 1;
                            }
                            for a in split_type_commas_slice(&ts[4..end]) {
                                cs.push(text_of(&a));
                            }
                        }
                        return inline_helper(&h, &v, &cs);
                    }
                    return Ok(match f.as_str() {
                        "sra" => ebin(">>>", &v[0], &v[1]),
                        "lt_signed" => ebin("<s", &v[0], &v[1]),
                        "mux" => format!(
                            "NlE::Cond(Box::new({}), Box::new({}), Box::new({}))",
                            v[0], v[1], v[2]
                        ),
                        // A function the file does not hold: its own
                        // lowering, from wherever it is (issue 504).
                        other => format!("{other}::lowered({})", v.join(", ")),
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
                    return as_lit(ts, &text, subst);
                }
                if text.starts_with("U::from") {
                    return Ok(format!("NlE::Num(({}) as u128)", g.stream()));
                }
            }
            // An associated constant of a type parameter, indexed and
            // projected, `M::RANGES[i].0`: a number Rust works out when
            // `lowered` runs, as a const parameter is. `i` is a literal
            // or an unrolled loop's variable, which is a Rust value
            // there (issue 593).
            if assoc_const_item(ts) {
                return Ok(format!("NlE::Num(({text}) as u128)"));
            }
            if text.contains("::") && !last_group {
                let leaf = text.rsplit("::").next().unwrap_or("");
                let upper = leaf.chars().all(|c| {
                    c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()
                });
                if upper {
                    return Ok(format!("NlE::Num(({text}) as u128)"));
                }
                return as_lit(ts, &text, subst);
            }
            if let Some(r) = call_lowered(ts, subst) {
                return r;
            }
            Err(format!("cannot lower `{text}`"))
        }
    }
}

/// A branch of a value `if` or an arm of a `match`, `{ e }`, as the
/// one expression in it. A block that says more, a `let` or a drive,
/// is refused: the lowering reads a value there, and the `let`s belong
/// before the `if` (issue 496).
fn block_value(
    g: &Group,
    subst: &[(String, String)],
    what: &str,
) -> Result<String, String> {
    let bt: Vec<TokenTree> = g.stream().into_iter().collect();
    if bt
        .iter()
        .any(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';'))
    {
        return Err(format!(
            "{what} in a lowered body is one expression: put its `let`s \
             before it"
        ));
    }
    tr(&bt, subst)
}

/// `if c { a } else if d { b } else { e }` as a value: a chain of
/// conditions, as `mux` is (issue 496). An `if` with no `else` has no
/// value when its condition is false, so it is refused.
fn tr_if(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Result<String, String> {
    let brace = |t: &TokenTree| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace);
    let Some(b) = ts.iter().position(brace) else {
        return Err("an `if` needs a `{ .. }` after its condition".into());
    };
    let c = tr(&ts[1..b], subst)?;
    let TokenTree::Group(then) = &ts[b] else {
        unreachable!()
    };
    let a =
        block_value(then, subst, "a branch of an `if` that yields a value")?;
    let rest = &ts[b + 1..];
    let e = match rest {
        [TokenTree::Ident(el), TokenTree::Group(g)]
            if el.to_string() == "else"
                && g.delimiter() == Delimiter::Brace =>
        {
            block_value(g, subst, "a branch of an `if` that yields a value")?
        }
        [TokenTree::Ident(el), TokenTree::Ident(i), ..]
            if el.to_string() == "else" && i.to_string() == "if" =>
        {
            tr_if(&rest[1..], subst)?
        }
        _ => {
            return Err("an `if` that yields a value needs an `else`: without \
                one it has no value when its condition is false"
                .into())
        }
    };
    Ok(format!(
        "NlE::Cond(Box::new({c}), Box::new({a}), Box::new({e}))"
    ))
}

/// `match v { pat => e, pat => { e } .. }` as a chain of conditions,
/// as `select!` is (issue 496). An arm whose value is a block may end
/// without a comma, as rustfmt writes it, so arms are split here and
/// not on commas.
fn tr_match(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Result<String, String> {
    let Some(TokenTree::Group(arms)) = ts.last() else {
        return Err("a `match` needs `{ pattern => value, .. }`".into());
    };
    let v = tr(&ts[1..ts.len() - 1], subst)?;
    let at: Vec<TokenTree> = arms.stream().into_iter().collect();
    let mut split: Vec<(Vec<TokenTree>, Vec<TokenTree>)> = Vec::new();
    let mut i = 0;
    while i < at.len() {
        let Some((pat, k)) = up_to_arrow(&at, i) else {
            return Err("a `match` arm is `pattern => value`".into());
        };
        let pat: Vec<TokenTree> = pat.into_iter().collect();
        if let Some(TokenTree::Group(g)) = at.get(k) {
            if g.delimiter() == Delimiter::Brace {
                split.push((pat, vec![at[k].clone()]));
                i = k + 1;
                if matches!(at.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ',')
                {
                    i += 1;
                }
                continue;
            }
        }
        let mut j = k;
        while j < at.len()
            && !matches!(&at[j], TokenTree::Punct(p) if p.as_char() == ',')
        {
            j += 1;
        }
        split.push((pat, at[k..j].to_vec()));
        i = j + 1;
    }
    arm_chain(&v, split, subst, "a `match` arm")
}

/// Arms, each a pattern and a value, as a chain of conditions on `v`:
/// the first arm whose pattern holds gives the value, and the last arm
/// is the default, which is what a `select!` and an exhaustive `match`
/// both mean.
fn arm_chain(
    v: &str,
    arms: Vec<(Vec<TokenTree>, Vec<TokenTree>)>,
    subst: &[(String, String)],
    what: &str,
) -> Result<String, String> {
    let mut chain: Vec<(String, String)> = Vec::new();
    for (pat, val) in arms {
        let c = pattern_cond(&pat, v, subst)?;
        let e = match val.as_slice() {
            [TokenTree::Group(g)] if g.delimiter() == Delimiter::Brace => {
                block_value(g, subst, what)?
            }
            _ => tr(&val, subst)?,
        };
        chain.push((c, e));
    }
    let Some((_, mut acc)) = chain.pop() else {
        return Err(format!("{what} is missing: a `match` needs an arm"));
    };
    for (c, e) in chain.into_iter().rev() {
        acc =
            format!("NlE::Cond(Box::new({c}), Box::new({e}), Box::new({acc}))");
    }
    Ok(acc)
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
    let mut split = Vec::new();
    for arm in split_commas(arms) {
        let Some((pat, k)) = up_to_arrow(&arm, 0) else {
            return Err("a select! arm is `pattern => value`".into());
        };
        split.push((pat.into_iter().collect(), arm[k..].to_vec()));
    }
    arm_chain(&v, split, subst, "a select! arm")
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

/// A range's two ends and whether the upper one is included: `lo..hi`
/// or `lo..=hi`, each end a number or a constant. `None` for tokens
/// that are not a range.
fn range_ends(ts: &[TokenTree]) -> Option<(&[TokenTree], &[TokenTree], bool)> {
    let dot = |i: usize| matches!(ts.get(i), Some(TokenTree::Punct(p)) if p.as_char() == '.');
    let at = (0..ts.len()).find(|&i| dot(i) && dot(i + 1))?;
    let inclusive = matches!(ts.get(at + 2), Some(TokenTree::Punct(p)) if p.as_char() == '=');
    let hi = &ts[at + if inclusive { 3 } else { 2 }..];
    if at == 0 || hi.is_empty() {
        return None;
    }
    Some((&ts[..at], hi, inclusive))
}

/// One end of a range as the lowering writes a constant.
fn range_end(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Result<String, String> {
    let text: String = ts.iter().map(|t| t.to_string()).collect();
    if text.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Ok(format!("NlE::Num(({text}) as u128)"));
    }
    as_lit(ts, &text, subst)
}

/// `v` within a range: `lo <= v && v <= hi`, or `v < hi` for `lo..hi`
/// (issue 496).
fn range_cond(
    v: &str,
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Result<Option<String>, String> {
    let Some((lo, hi, inclusive)) = range_ends(ts) else {
        return Ok(None);
    };
    let lo = range_end(lo, subst)?;
    let hi = range_end(hi, subst)?;
    let upper = if inclusive {
        ebin("<=", v, &hi)
    } else {
        ebin("<", v, &hi)
    };
    Ok(Some(ebin("&&", &ebin(">=", v, &lo), &upper)))
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
        } else if let Some(r) = range_cond(v, a, subst)? {
            r
        } else if digit {
            ebin("==", v, &format!("NlE::Num(({text}) as u128)"))
        } else {
            // A pattern is a constant too, so the same check applies.
            ebin("==", v, &as_lit(a, &text, subst)?)
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
    /// The name each wire takes when a field has the one it wanted:
    /// chosen here, where the ports and the other wires are known,
    /// and used by the constant that makes the choice.
    wire_alts: &'a mut Vec<String>,
    /// Each wire as the `let` named it, as the netlist names it, and
    /// where the `let` names it, for the checks on names.
    named: &'a mut Vec<(String, String, Span)>,
    subst: Vec<(String, String)>,
    guard: Option<String>,
    clock: String,
    falling: bool,
    hoisted: Vec<String>,
    /// The variables of the `for` loops the statements are inside,
    /// innermost last (issue 500).
    loops: Vec<String>,
}

/// The marks around a loop's variable in a name built from it: `ins[i]`
/// is the port `ins__TXIDX_i_XDIT` while the macro reads the body, and
/// `dyn_names` turns every quoted name holding one into a `format!` of
/// `ins_{i}` in the text it writes (issue 500).
const DYN: &str = "__TXIDX_";
const DYN_END: &str = "_XDIT";

/// Every string literal in `text` that holds a loop's variable between
/// `DYN` and `DYN_END`, as `(&*format!(..))` of it: a port's name,
/// `ins_{i}_valid`, or an index alone, `{i}`. The parentheses make it a
/// `&str` either way, so `.to_string()` after it still gives a
/// `String` and a place that wants a `&str` has one.
fn dyn_names(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(DYN) {
        let Some(open) = rest[..at].rfind('"') else {
            break;
        };
        let Some(close) = rest[at..].find('"').map(|c| at + c) else {
            break;
        };
        let lit = &rest[open + 1..close];
        let m = lit.find(DYN).unwrap();
        let prefix = &lit[..m];
        let after = &lit[m + DYN.len()..];
        let Some(e) = after.find(DYN_END) else {
            break;
        };
        let (var, suffix) = (&after[..e], &after[e + DYN_END.len()..]);
        let sep = if prefix.is_empty() { "" } else { "_" };
        out.push_str(&rest[..open]);
        out.push_str(&format!(
            "(&*format!(\"{prefix}{sep}{{}}{suffix}\", {var}))"
        ));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// A statement that is Rust code for `lowered` to run rather than a
/// netlist statement to push: a `for` loop, a `let` whose value is
/// an expression built as the loop goes round (issue 500).
const RAW: &str = "\u{1}raw\u{1}";

/// The Rust code of a value a loop's `let`, or an assignment to a
/// `let mut`, holds: a wire of its own, `name_lN` with `N` counted as
/// `lowered` runs, so a value carried round a loop is written once a
/// turn rather than inside every turn after it. A name or a number
/// needs no wire (issue 500).
fn dyn_wire(name: &str, e: &str) -> String {
    let plain = e.starts_with("NlE::Name(")
        || e.starts_with("NlE::Num(")
        || e.starts_with("NlE::Bits(")
        || e.starts_with("::txhdl::netlist::lit(")
        || e.starts_with("__l_");
    if plain {
        return e.to_string();
    }
    format!(
        "{{ let __n = format!(\"{name}_l{{}}\", __wn); __wn += 1; \
         __dynw.push((__n.clone(), {e})); NlE::Name(__n) }}"
    )
}

/// Statements as the Rust code that pushes them, in order, onto `__b`,
/// with any raw code among them as it is.
fn items_code(items: &[String]) -> String {
    items
        .iter()
        .map(|s| match s.strip_prefix(RAW) {
            Some(code) => code.to_string(),
            None => format!("__b.push({s});"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Statements as a block that yields them: a process's body, or an
/// arm's, so a loop or a `let` of Rust among them has a scope as it
/// does in the source.
fn stmts_code(items: &[String]) -> String {
    format!(
        "{{ #[allow(unused_mut)] let mut __b: Vec<NlS> = Vec::new(); {} __b }}",
        items_code(items)
    )
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
            // A `for` loop ends at its body, with no `;` (issue 500).
            TokenTree::Group(g)
                if g.delimiter() == Delimiter::Brace
                    && cur.first().is_some_and(|f| is_ident(f, "for")) =>
            {
                cur.push(t.clone());
                out.push(std::mem::take(&mut cur));
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
        // The wires the statement before this one asked for while it
        // was inlining a call, taken into the unit's. They are
        // continuous assignments, so where they sit among the others
        // does not matter; that they are there does. See issue 126.
        cx.wires.extend(INLINED.with(|w| w.take()));
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
        // `for i in lo..hi { .. }`: unrolled when `lowered` runs, since
        // the count is usually a const parameter the macro cannot see.
        // The loop becomes a Rust `for` around the statements it
        // pushes, and inside it `i` is a number and every name built
        // from it is formatted with it (issue 500).
        if is_ident(&ts[0], "for") {
            if path.is_some() {
                return Err(err(
                    ts[0].span(),
                    "a `for` in a lowered body belongs at the top of the loop, \
                     not under an `if`: put the `if` inside the `for`",
                ));
            }
            let (
                Some(TokenTree::Ident(var)),
                true,
                Some(TokenTree::Group(body)),
            ) = (
                ts.get(1),
                ts.get(2).is_some_and(|t| is_ident(t, "in")),
                ts.last(),
            )
            else {
                return Err(err(
                    ts[0].span(),
                    "a `for` in a lowered body is `for i in lo..hi { .. }`",
                ));
            };
            let range = text_of(&ts[3..ts.len() - 1]);
            if !range.contains("..") {
                return Err(err(
                    ts[0].span(),
                    "a `for` in a lowered body goes over a range, `lo..hi`",
                ));
            }
            let var = var.to_string();
            let mark = cx.subst.len();
            cx.subst
                .push((var.clone(), format!("NlE::Num({var} as u128)")));
            cx.loops.push(var.clone());
            let inner: Vec<TokenTree> = body.stream().into_iter().collect();
            let hmark = cx.hoisted.len();
            let mut items = lower_stmts(cx, &inner, None)?;
            // A send under an `if` in the loop names the loop's index
            // and its `let`s, so it stays in the loop, one per turn,
            // rather than being hoisted past the loop's end (#500).
            items.extend(cx.hoisted.drain(hmark..));
            cx.loops.pop();
            // A `let` inside the loop is gone after it, as in Rust.
            cx.subst.truncate(mark);
            stmts.push(format!(
                "{RAW}for {var} in {range} {{ {} }}",
                items_code(&items)
            ));
            continue;
        }
        // Inside a loop, or for a `let mut`, a `let` is a Rust variable
        // holding the expression, built afresh on every turn, rather
        // than a wire: its value depends on the loop's index, or is
        // carried from one turn to the next by `x = ..` (issue 500).
        let is_mut = ts.get(1).is_some_and(|t| is_ident(t, "mut"));
        // A channel's receive, take or wait, and `let _`, keep their
        // own lowering below.
        let channel_op = is_ident(ts.get(1).unwrap_or(&ts[0]), "_")
            || text.contains(".recv")
            || text.ends_with(".take()")
            || text.contains(".wait()");
        // So is a `let` that reads such a variable after its loop: a
        // wire is defined outside the process, where the variable is
        // not (#500).
        let reads_var = ts.len() > 3 && reads_rust_var(&ts[3..], &cx.subst);
        if is_ident(&ts[0], "let")
            && !channel_op
            && (is_mut || !cx.loops.is_empty() || reads_var)
        {
            let at = if is_mut { 2 } else { 1 };
            let (Some(TokenTree::Ident(name)), true) = (
                ts.get(at),
                ts.get(at + 1).is_some_and(
                    |t| matches!(t, TokenTree::Punct(p) if p.as_char() == '='),
                ),
            ) else {
                return Err(err(
                    ts[0].span(),
                    "a `let` in a loop, or a `let mut`, binds one name: \
                     `let x = e`",
                ));
            };
            let e = match tr(&ts[at + 2..], &cx.subst) {
                Ok(e) => e,
                Err(m) => return Err(err(ts[0].span(), &m)),
            };
            let name = name.to_string();
            let var = format!("__l_{name}");
            let kw = if is_mut { "let mut" } else { "let" };
            let e = dyn_wire(&name, &e);
            stmts.push(format!("{RAW}{kw} {var}: NlE = {e};"));
            cx.subst.push((name, format!("{var}.clone()")));
            continue;
        }
        // `x = e` of a `let mut`: the next value of the Rust variable,
        // which is how a loop carries a value from one turn to the next.
        if let [TokenTree::Ident(name), TokenTree::Punct(eq), rest @ ..] =
            ts.as_slice()
        {
            let var = format!("__l_{name}");
            let bound = cx
                .subst
                .iter()
                .rev()
                .find(|(n, _)| *n == name.to_string())
                .is_some_and(|(_, v)| *v == format!("{var}.clone()"));
            if eq.as_char() == '='
                && eq.spacing() == proc_macro::Spacing::Alone
                && bound
            {
                let e = match tr(rest, &cx.subst) {
                    Ok(e) => e,
                    Err(m) => return Err(err(ts[0].span(), &m)),
                };
                let e = dyn_wire(&name.to_string(), &e);
                stmts.push(format!("{RAW}{var} = {e};"));
                continue;
            }
        }
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
        // not hardware; `if !c` is not one, and neither are the macros
        // below that lower: the drives, and the statements of what
        // must hold (issue 502).
        let is_macro = matches!(
            (&ts[0], ts.get(1)),
            (TokenTree::Ident(_), Some(TokenTree::Punct(p)))
                if p.as_char() == '!'
        ) && !is_ident(&ts[0], "if");
        if is_macro
            && !text.starts_with("when!")
            && !text.starts_with("case!")
            && !text.starts_with("with!")
            && !text.starts_with("check!")
            && !text.starts_with("assume!")
            && !text.starts_with("cover!")
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
                // The netlist has one namespace for fields, ports and
                // wires, and Rust's rules do not reach into it: a
                // `let` may take a port's name, another `let`'s, a
                // field's, or a word a target reserves. So the wire
                // takes the `let`'s name where it is free and `_w`,
                // `_w2`, `_w3` where it is not, by one rule (issue
                // 171). What is free here is what this attribute can
                // see: the ports and the wires before it, and the
                // reserved words. A field it cannot see, so the name
                // is chosen by a constant below, which the compiler
                // evaluates for every type the unit is lowered at.
                let taken = |w: &str, cx: &Cx| {
                    cx.pnames.iter().any(|p| p == w || escaped(p) == w)
                        || cx.wires.iter().any(|(x, _)| x == w)
                        || reserved_by(w).is_some()
                };
                let mut w = name.clone();
                let mut k = 1;
                while taken(&w, cx) {
                    k += 1;
                    w = match k {
                        2 => format!("{name}_w"),
                        _ => format!("{name}_w{}", k - 1),
                    };
                }
                // And the same again for the name the constant falls
                // back to, so that a field's name sends the wire
                // somewhere nothing else has taken.
                let mut alt = format!("{w}_w");
                while taken(&alt, cx) || alt == w {
                    alt = format!("{alt}_w");
                }
                let idx = cx.wires.len();
                let chosen = format!("Self::__TXHDL_WIRE_{idx}");
                cx.wires.push((w.clone(), v));
                cx.named.push((name.clone(), w.clone(), n[0].span()));
                cx.wire_alts.push(alt);
                cx.subst
                    .push((name, format!("NlE::Name({chosen}.to_string())")));
            }
            continue;
        }
        // `check!(c, "msg")`, `assume!(c, "msg")`, `cover!(c, "msg")`: a
        // condition stated at the edge, where the statement is, under
        // the conditions it is under (issue 502).
        let stated = [
            ("check!", "Assert"),
            ("assume!", "Assume"),
            ("cover!", "Cover"),
        ]
        .into_iter()
        .find(|(m, _)| text.starts_with(m));
        if let Some((mac, kind)) = stated {
            let Some(TokenTree::Group(g)) = ts.get(2) else {
                return Err(err(ts[0].span(), &format!("expected {mac}(..)")));
            };
            let args = split_commas(g);
            let (Some(cond), Some(msg)) = (args.first(), args.get(1)) else {
                return Err(err(
                    g.span(),
                    &format!("{mac} takes a condition and a message"),
                ));
            };
            let [TokenTree::Literal(msg)] = msg.as_slice() else {
                return Err(err(g.span(), "the message is a string literal"));
            };
            let c = match tr(cond, &cx.subst) {
                Ok(c) => c,
                Err(m) => return Err(err(ts[0].span(), &m)),
            };
            stmts.push(format!(
                "NlS::Check(::txhdl::netlist::Checked::{kind}, {c}, \
                 {msg}.to_string())"
            ));
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
                arms.push(format!("({c}, {})", stmts_code(&body)));
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
                        els = stmts_code(&body);
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
    // And the last statement's, which no next one would take.
    cx.wires.extend(INLINED.with(|w| w.take()));
    Ok(stmts)
}

/// The net of `h.f`, one channel of a bundle made whole whose side `h`
/// is one of `links`: `NET_f` (issue 498). `None` for anything else.
fn link_field(
    ts: &[TokenTree],
    links: &[(String, String, String)],
) -> Option<String> {
    let [TokenTree::Ident(h), d, TokenTree::Ident(f)] = ts else {
        return None;
    };
    if !matches!(d, TokenTree::Punct(p) if p.as_char() == '.') {
        return None;
    }
    let h = h.to_string();
    links
        .iter()
        .find(|(e, _, _)| *e == h)
        .map(|(_, _, net)| format!("{net}_{f}"))
}

/// The value of `tie(v)`, or of a path ending in `tie`, as text: what a
/// unit of units passes a child's input to hold it at a constant
/// (issue 498). `None` for anything else.
fn tied(ts: &[TokenTree]) -> Option<String> {
    match ts {
        [.., TokenTree::Ident(f), TokenTree::Group(g)]
            if f.to_string() == "tie"
                && g.delimiter() == Delimiter::Parenthesis
                && ts[..ts.len() - 2].iter().all(|t| match t {
                    TokenTree::Ident(_) => true,
                    TokenTree::Punct(p) => p.as_char() == ':',
                    _ => false,
                }) =>
        {
            Some(g.stream().to_string())
        }
        _ => None,
    }
}

/// What `#[lower]` adds to a function beside the function itself: a
/// type of the same name, in the other namespace, whose `lowered` builds
/// the function's expression when a unit is lowered (issue 504).
///
/// A unit inlines a helper of its own file from the file's text, which
/// is all a proc macro can read. A helper in another module or crate is
/// out of its sight, so the helper carries its own lowering instead,
/// and the unit calls it by the path it calls the function by: `f(x)`
/// lowers to `f::lowered(x)`, which Rust resolves wherever `f` resolves,
/// through a `use` or a path, since a `use` brings both namespaces. A
/// value read twice becomes a wire of the unit, as it does when inlined
/// from the file, and a helper it calls is reached the same way.
///
/// `None` when the function's body is not one the lowering reads: the
/// function is then plain Rust, and a unit calling it from elsewhere is
/// refused by the compiler, which finds no `lowered`.
fn companion(item: &TokenStream) -> Option<String> {
    let toks: Vec<TokenTree> = item.clone().into_iter().collect();
    let at = toks.iter().position(|t| is_ident(t, "fn"))?;
    let TokenTree::Ident(name) = toks.get(at + 1)? else {
        return None;
    };
    let name = name.to_string();
    // The function's visibility, which its lowering shares.
    let vis = match toks[..at].iter().position(|t| is_ident(t, "pub")) {
        Some(p) => text_of(&toks[p..at]),
        None => String::new(),
    };
    // Its const parameters, which the lowering takes as its own.
    let mut consts: Vec<String> = Vec::new();
    if punct_at(&toks, at + 2, '<') {
        let (args, _) = angle_args(&toks, at + 2);
        for a in args {
            if let [TokenTree::Ident(k), ..] = a.as_slice() {
                if k.to_string() == "const" {
                    consts.push(text_of(&a));
                }
            }
        }
    }
    let marked: TokenStream = format!("#[lower] {item}").parse().ok()?;
    let saved = HELPER_TYPES.with(|t| t.borrow().clone());
    let found = scan_helpers(marked.into_iter().collect());
    let h = found.into_iter().find(|h| h.name == name)?;
    let typed = HELPER_TYPES.with(|t| {
        let mut t = t.borrow_mut();
        let mine = t
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        *t = saved;
        mine
    });
    if !h.refused.is_empty() || h.value.is_empty() {
        return None;
    }
    // Another helper this one calls is reached through its own
    // `lowered` too, not inlined from a file this one cannot see.
    let helpers = HELPERS.with(|x| std::mem::take(&mut *x.borrow_mut()));
    let mark = TYPED.with(|t| {
        let mut t = t.borrow_mut();
        let n = t.len();
        for (p, ty) in typed.iter().filter(|(_, ty)| !ty.is_empty()) {
            t.push((p.clone(), ty.clone()));
        }
        n
    });
    let rest = |from: usize| -> Vec<&str> {
        let mut v: Vec<&str> =
            h.lets[from..].iter().map(|(_, e)| e.as_str()).collect();
        v.push(h.value.as_str());
        v
    };
    let mut body = String::new();
    let mut subst: Vec<(String, String)> = Vec::new();
    for (k, p) in h.params.iter().enumerate() {
        let reads = reads_of(p, &rest(0));
        body.push_str(&format!(
            "let __s{k} = ::txhdl::netlist::inline_bind(\"{name}_{p}\", \
             {p}, {reads});\n"
        ));
        subst.push((p.clone(), format!("__s{k}.clone()")));
    }
    let read = |text: &str| -> Option<Vec<TokenTree>> {
        text.parse::<TokenStream>()
            .ok()
            .map(|t| t.into_iter().collect())
    };
    let mut result = None;
    'body: {
        for (k, (n, e)) in h.lets.iter().enumerate() {
            let Some(ts) = read(e) else { break 'body };
            let Ok(v) = tr(&ts, &subst) else { break 'body };
            let reads = reads_of(n, &rest(k + 1));
            body.push_str(&format!(
                "let __l{k} = ::txhdl::netlist::inline_bind(\"{name}_{n}\", \
                 {v}, {reads});\n"
            ));
            subst.push((n.clone(), format!("__l{k}.clone()")));
        }
        let Some(ts) = read(&h.value) else {
            break 'body;
        };
        let Ok(v) = tr(&ts, &subst) else { break 'body };
        result = Some(v);
    }
    TYPED.with(|t| t.borrow_mut().truncate(mark));
    HELPERS.with(|x| *x.borrow_mut() = helpers);
    let value = result?;
    let generics = if consts.is_empty() {
        String::new()
    } else {
        format!("<{}>", consts.join(", "))
    };
    let params = h
        .params
        .iter()
        .map(|p| format!("{p}: ::txhdl::netlist::Expr"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "#[doc(hidden)]\n\
         #[allow(non_camel_case_types, dead_code)]\n\
         {vis} struct {name} {{}}\n\
         impl {name} {{\n\
         #[doc(hidden)]\n\
         #[allow(clippy::all, unused_variables, unused_mut)]\n\
         pub fn lowered{generics}({params}) -> ::txhdl::netlist::Expr {{\n\
         use ::txhdl::netlist::Expr as NlE;\n\
         {body}{value}\n}}\n}}\n"
    ))
}

/// A call `path(args)` or `path::<K>(args)` of a function the unit's
/// file does not hold, as a call of the function's own lowering:
/// `path::lowered(args)` (issue 504).
fn call_lowered(
    ts: &[TokenTree],
    subst: &[(String, String)],
) -> Option<Result<String, String>> {
    let (TokenTree::Ident(_), Some(TokenTree::Group(g))) = (&ts[0], ts.last())
    else {
        return None;
    };
    if g.delimiter() != Delimiter::Parenthesis {
        return None;
    }
    let callee = &ts[..ts.len() - 1];
    let turbo = (0..callee.len()).find(|&k| {
        punct_at(callee, k, ':')
            && punct_at(callee, k + 1, ':')
            && punct_at(callee, k + 2, '<')
    });
    let (path, turbofish) = match turbo {
        Some(k) => (text_of(&callee[..k]), text_of(&callee[k..])),
        None => (text_of(callee), String::new()),
    };
    let mut args = Vec::new();
    for a in split_commas(g) {
        match tr(&a, subst) {
            Ok(v) => args.push(v),
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok(format!(
        "{path}::lowered{turbofish}({})",
        args.join(", ")
    )))
}

/// A unit of units: `run` makes the channels and wires between its
/// children with `chan()` and `signal()`, and joins the children's
/// `run`s. Read into the parent's nets and instances, as generated
/// text: a `(name, kind, width)` per net, and an
/// `instance(child_lowered(&me.FIELD, ..), "FIELD", &[..])` per child,
/// its ports joined in order to nets and to the parent's ports, or by
/// name where the parent passes a struct literal `S { a: x, b }`.
/// `ports` are the parent's, by name and kind, and `bound` its sides
/// that are structs of ports, each passed whole as its fields.
fn lower_structural(
    body: &Group,
    ports: &[(String, String)],
    bound: &[(String, Vec<String>)],
) -> Result<(Vec<String>, Vec<String>, Vec<String>), TokenStream> {
    // The wires a constant is tied to, each a `(name, lit(v))`: what a
    // child's input passed `tie(v)` is joined to (issue 498).
    let mut ties: Vec<String> = Vec::new();
    // The ends made in `run`: the end, its net, whether a channel.
    let mut ends: Vec<(String, String, bool)> = Vec::new();
    // The two sides of each bundle made whole, `let (h, p) = link::<B>()`:
    // the side, `B`, and the net its channels are named for (issue 498).
    let mut links: Vec<(String, String, String)> = Vec::new();
    let mut nets: Vec<String> = Vec::new();
    let mut instances: Vec<String> = Vec::new();
    // The channel ends and channel ports joined so far, each once.
    let mut used: Vec<String> = Vec::new();
    // Another name for a wire's reading end or an input port, made by
    // `let b = a.clone();`: the name and what it stands for.
    let mut aliases: Vec<(String, String)> = Vec::new();
    // The ends handed out by index, `Ends`: the name bound and the net,
    // or the array port, whose element `e` is `net_e` (issue 635).
    let mut arr_ends: Vec<(String, String)> = Vec::new();
    let is_chan = |k: &str| k == "Tx" || k == "Rx";
    for st in statements(body) {
        let ts: Vec<TokenTree> = st.into_iter().collect();
        if ts.is_empty() {
            continue;
        }
        // `let mut x = Ends::from(ins);`: an array port handed out by
        // index, its element `e` the port `ins_e` (issue 635).
        if let (
            true,
            true,
            Some(TokenTree::Ident(x)),
            true,
            true,
            true,
            Some(TokenTree::Group(g)),
        ) = (
            is_ident(&ts[0], "let"),
            ts.get(1).is_some_and(|t| is_ident(t, "mut")),
            ts.get(2),
            ts.get(4).is_some_and(|t| is_ident(t, "Ends")),
            punct_at(&ts, 5, ':') && punct_at(&ts, 6, ':'),
            ts.get(7).is_some_and(|t| is_ident(t, "from")),
            ts.get(8),
        ) {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            let [TokenTree::Ident(p)] = inner.as_slice() else {
                return Err(err(
                    g.span(),
                    "`Ends::from` takes an array port by its name",
                ));
            };
            arr_ends.push((x.to_string(), p.to_string()));
            continue;
        }
        // `let b = a.clone();`: a wire read in two places, which is
        // how a design fans an input out to several children. A clone
        // is another name for the same net; a channel has one receiver
        // and is not cloned this way.
        if let (
            true,
            Some(TokenTree::Ident(b)),
            Some(TokenTree::Punct(eq)),
            Some(TokenTree::Ident(a)),
            Some(TokenTree::Punct(dot)),
            Some(cl),
            Some(TokenTree::Group(args)),
        ) = (
            is_ident(&ts[0], "let"),
            ts.get(1),
            ts.get(2),
            ts.get(3),
            ts.get(4),
            ts.get(5),
            ts.get(6),
        ) {
            if eq.as_char() == '='
                && dot.as_char() == '.'
                && is_ident(cl, "clone")
                && args.stream().is_empty()
            {
                let (a, b) = (a.to_string(), b.to_string());
                let a = aliases
                    .iter()
                    .find(|(x, _)| *x == a)
                    .map(|(_, t)| t.clone())
                    .unwrap_or(a);
                let wire_end = ends.iter().any(|(e, _, ch)| *e == a && !*ch);
                let in_port = ports.iter().any(|(p, k)| *p == a && k == "In");
                if !wire_end && !in_port {
                    return Err(err(
                        ts[3].span(),
                        &format!(
                            "`{a}` is not a wire's end or an input port; \
                             only those are cloned"
                        ),
                    ));
                }
                aliases.push((b, a));
                continue;
            }
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
            // A name may be `mut`, as the ends `chans` hands out are, since
            // taking one changes them (issue 635).
            let ns: Vec<Vec<TokenTree>> = split_commas(names)
                .into_iter()
                .map(|n| match n.as_slice() {
                    [m, rest @ ..] if is_ident(m, "mut") => rest.to_vec(),
                    _ => n,
                })
                .collect();
            if ns.len() != 2 || ns.iter().any(|n| n.len() != 1) {
                return Err(bad(&ts[1]));
            }
            let (a, b) = (ns[0][0].to_string(), ns[1][0].to_string());
            let Some(f) = ts.get(3) else {
                return Err(bad(&ts[0]));
            };
            let chan = is_ident(f, "chan");
            let whole = is_ident(f, "link");
            // `chans::<T, C, N>()` or `signals::<T, C, N>()`: N nets,
            // `net_0` onward, their ends handed out by index (issue 635).
            let many = is_ident(f, "chans") || is_ident(f, "signals");
            if many {
                let lt = ts.iter().position(
                    |t| matches!(t, TokenTree::Punct(p) if p.as_char() == '<'),
                );
                let gt = ts.iter().rposition(
                    |t| matches!(t, TokenTree::Punct(p) if p.as_char() == '>'),
                );
                let (Some(lt), Some(gt)) = (lt, gt) else {
                    return Err(err(
                        f.span(),
                        "name the payload, clock and count: \
                         `chans::<T, C, N>()`",
                    ));
                };
                let parts: Vec<String> =
                    split_type_commas_slice(&ts[lt + 1..gt])
                        .iter()
                        .map(|p| text_of(p))
                        .collect();
                let [ty, clock, count] = parts.as_slice() else {
                    return Err(err(
                        f.span(),
                        "name the payload, clock and count: \
                         `chans::<T, C, N>()`",
                    ));
                };
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
                let kind = if is_ident(f, "chans") { "Tx" } else { "Out" };
                nets.push(format!(
                    "@raw for __k in 0..({count}) {{ n.push((format!(\"{net}_{{}}\", __k), \
                     ::txhdl::comp::trace::Kind::{kind}, \
                     <{ty} as ::txhdl::types::Value>::WIDTH, \
                     <{clock} as ::txhdl::comp::Clock>::NAME)); }}"
                ));
                arr_ends.push((a, net.clone()));
                arr_ends.push((b, net));
                continue;
            }
            if !chan && !whole && !is_ident(f, "signal") {
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
            // A bundle made whole: its nets are listed when `lowered`
            // runs, one per port of the type the turbofish names.
            if whole {
                nets.push(format!("@link {ty}|{net}"));
                links.push((a, ty.clone(), net.clone()));
                links.push((b, ty, net));
                continue;
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
        let mut arrays: Vec<(String, String, Group, Span)> = Vec::new();
        find_array_runs(&ts, &mut arrays);
        // An array of children: an instance per child, `F_i`, made when
        // `lowered` runs, each joined by what its index names (issue 635).
        for (field, idx, args, span) in &arrays {
            let sides = split_commas(args);
            if sides.len() != 2 {
                return Err(err(
                    *span,
                    "a child's `run` takes its inputs and its outputs",
                ));
            }
            let mut joined: Vec<String> = Vec::new();
            let mut nties = 0usize;
            for side in &sides {
                let items: Vec<Vec<TokenTree>> = match side.as_slice() {
                    [TokenTree::Group(g)]
                        if g.delimiter() == Delimiter::Parenthesis =>
                    {
                        split_commas(g)
                    }
                    other => vec![other.to_vec()],
                };
                for it in items {
                    if it.is_empty() {
                        continue;
                    }
                    // `x.take(e)`: element `e` of an array of ends.
                    if let (
                        Some(TokenTree::Ident(x)),
                        true,
                        true,
                        Some(TokenTree::Group(e)),
                    ) = (
                        it.first(),
                        punct_at(&it, 1, '.'),
                        it.get(2).is_some_and(|t| is_ident(t, "take")),
                        it.get(3),
                    ) {
                        let Some((_, net)) =
                            arr_ends.iter().find(|(n, _)| *n == x.to_string())
                        else {
                            return Err(err(
                                x.span(),
                                &format!(
                                    "`{x}` is not an array of ends made in \
                                     `run` by `chans`, `signals` or \
                                     `Ends::from`"
                                ),
                            ));
                        };
                        joined.push(format!(
                            "a.push((String::new(), format!(\"{net}_{{}}\", {})));",
                            text_of(&e.stream().into_iter().collect::<Vec<_>>())
                        ));
                        continue;
                    }
                    // `tie(v)`: an input held at a constant, which may
                    // name the index, so each child has a wire of its
                    // own, `F_i_tieK` (issue 635).
                    if let Some(v) = tied(&it) {
                        let k = nties;
                        nties += 1;
                        joined.push(format!(
                            "{{ let __t = format!(\"{field}_{{}}_tie{k}\", \
                             {idx}); __dynw.push((__t.clone(), \
                             ::txhdl::netlist::lit({v}))); \
                             a.push((String::new(), __t)); }}"
                        ));
                        continue;
                    }
                    // A name: a wire read by every child, or `()`.
                    match it.as_slice() {
                        [TokenTree::Group(g)] if g.stream().is_empty() => {}
                        [TokenTree::Ident(n)] => {
                            let n = n.to_string();
                            let n = aliases
                                .iter()
                                .find(|(x, _)| *x == n)
                                .map(|(_, t)| t.clone())
                                .unwrap_or(n);
                            let net = if let Some((_, net, ch)) =
                                ends.iter().find(|(e, _, _)| *e == n)
                            {
                                (!*ch).then(|| net.clone())
                            } else {
                                ports
                                    .iter()
                                    .find(|(p, k)| *p == n && k == "In")
                                    .map(|(p, _)| p.clone())
                            };
                            let Some(net) = net else {
                                return Err(err(
                                    *span,
                                    &format!(
                                        "`{n}` goes to every child of the \
                                         array; only a wire or an input \
                                         port can, and a channel is handed \
                                         out by index with `take`"
                                    ),
                                ));
                            };
                            joined.push(format!(
                                "a.push((String::new(), \"{net}\".to_string()));"
                            ));
                        }
                        _ => {
                            return Err(err(
                                *span,
                                "a port passed to a child of an array is \
                                 `x.take(e)`, a wire's name, `tie(v)` or `()`",
                            ))
                        }
                    }
                }
            }
            instances.push(format!(
                "@stmt for {idx} in 0..me.{field}.0.len() {{ \
                 __ins.push(::txhdl::netlist::instance_of(\
                 ::txhdl::netlist::child_lowered(&me.{field}.0[{idx}], \
                 &format!(\"{{}}_{field}_{{}}\", name, {idx})), \
                 &format!(\"{field}_{{}}\", {idx}), \
                 {{ let mut a: Vec<(String, String)> = Vec::new(); {} a }})); }}",
                joined.join(" ")
            ));
        }
        if calls.is_empty() && !arrays.is_empty() {
            continue;
        }
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
            // The names passed, each with the child's port it joins
            // when the parent named that port, by a struct literal.
            let mut names: Vec<(String, String)> = Vec::new();
            for side in &sides {
                match side.as_slice() {
                    [TokenTree::Group(g)]
                        if g.delimiter() == Delimiter::Parenthesis =>
                    {
                        for n in split_commas(g) {
                            // An array port's ends, `[a, b, c]`: its
                            // ports are `x_0` to `x_2`, in that order,
                            // so the names join them in order (#500).
                            if let [TokenTree::Group(a)] = n.as_slice() {
                                if a.delimiter() == Delimiter::Bracket {
                                    for m in split_commas(a) {
                                        let [TokenTree::Ident(id)] =
                                            m.as_slice()
                                        else {
                                            return Err(err(
                                                a.span(),
                                                "an array passed to a child \
                                                 is of names, `[a, b, c]`",
                                            ));
                                        };
                                        names.push((
                                            String::new(),
                                            id.to_string(),
                                        ));
                                    }
                                    continue;
                                }
                            }
                            // One channel of a bundle made whole: `h.aw`.
                            if let Some(net) = link_field(&n, &links) {
                                names.push((
                                    String::new(),
                                    format!("@net {net}"),
                                ));
                                continue;
                            }
                            if let Some(v) = tied(&n) {
                                let net = format!("{field}_tie{}", ties.len());
                                ties.push(format!(
                                    "(\"{net}\".to_string(), \
                                     ::txhdl::netlist::lit({v}))"
                                ));
                                names.push((
                                    String::new(),
                                    format!("@tie {net}"),
                                ));
                                continue;
                            }
                            let [TokenTree::Ident(id)] = n.as_slice() else {
                                return Err(err(
                                    g.span(),
                                    "a port passed to a child is a name, or \
                                     `tie(v)` for an input held at a constant",
                                ));
                            };
                            names.push((String::new(), id.to_string()));
                        }
                    }
                    // A side of the parent's that is a struct of ports,
                    // passed whole, is its fields in order.
                    [TokenTree::Ident(id)] => {
                        let id = id.to_string();
                        match bound.iter().find(|(b, _)| *b == id) {
                            // A struct declared in another file, whose
                            // fields cannot be seen here: its ports are
                            // joined in order when `lowered` runs.
                            Some((_, fs)) if fs == &["*"] => {
                                let ty = BUNDLES.with(|b| {
                                    b.borrow()
                                        .iter()
                                        .find(|(n, _)| *n == id)
                                        .map(|(_, t)| t.clone())
                                });
                                let Some(ty) = ty else {
                                    return Err(err(
                                        span,
                                        &format!("`{id}`'s type is not known"),
                                    ));
                                };
                                names.push((
                                    String::new(),
                                    format!("@bundle {ty} {id}"),
                                ));
                            }
                            // Its fields in order; one that is a struct of
                            // ports nested in it is joined whole, as a side
                            // declared elsewhere is (issue 498).
                            Some((_, fs)) => {
                                for f in fs {
                                    let ty = BUNDLES.with(|b| {
                                        b.borrow()
                                            .iter()
                                            .find(|(n, _)| n == f)
                                            .map(|(_, t)| t.clone())
                                    });
                                    let nested = bound
                                        .iter()
                                        .any(|(n, x)| n == f && x == &["*"]);
                                    match (ty, nested) {
                                        (Some(ty), true) => names.push((
                                            String::new(),
                                            format!("@bundle {ty} {f}"),
                                        )),
                                        _ => names
                                            .push((String::new(), f.clone())),
                                    }
                                }
                            }
                            None => names.push((String::new(), id)),
                        }
                    }
                    [TokenTree::Ident(_), .., TokenTree::Group(g)]
                        if g.delimiter() == Delimiter::Brace =>
                    {
                        for e in split_commas(g) {
                            // A field that is itself a struct of ports,
                            // given as a literal: `pins: S { a, b: x }`
                            // joins the child's `pins_a` and `pins_b`
                            // (issue 579).
                            if let (
                                Some(TokenTree::Ident(f)),
                                true,
                                Some(TokenTree::Group(inner)),
                            ) = (e.first(), punct_at(&e, 1, ':'), e.last())
                            {
                                if inner.delimiter() == Delimiter::Brace
                                    && e.len() > 3
                                {
                                    for s in split_commas(inner) {
                                        let (sub, net) = match s.as_slice() {
                                            [TokenTree::Ident(a)] => (a, a),
                                            [TokenTree::Ident(a), _, TokenTree::Ident(n)]
                                                if punct_at(&s, 1, ':') =>
                                            {
                                                (a, n)
                                            }
                                            _ => return Err(err(
                                                inner.span(),
                                                "a field of a nested struct \
                                                     passed to a child is \
                                                     `field: name` or `field`",
                                            )),
                                        };
                                        names.push((
                                            format!("{f}_{sub}"),
                                            net.to_string(),
                                        ));
                                    }
                                    continue;
                                }
                            }
                            let (port, net) = match e.as_slice() {
                                [TokenTree::Ident(f)] => (f, f),
                                [TokenTree::Ident(f), _, TokenTree::Ident(n)]
                                    if punct_at(&e, 1, ':') =>
                                {
                                    (f, n)
                                }
                                _ => {
                                    return Err(err(
                                        g.span(),
                                        "a field of a struct passed to a \
                                         child is `field: name` or `field`",
                                    ))
                                }
                            };
                            names.push((port.to_string(), net.to_string()));
                        }
                    }
                    side if link_field(side, &links).is_some() => {
                        let net = link_field(side, &links).unwrap_or_default();
                        names.push((String::new(), format!("@net {net}")));
                    }
                    side if tied(side).is_some() => {
                        let v = tied(side).unwrap_or_default();
                        let net = format!("{field}_tie{}", ties.len());
                        ties.push(format!(
                            "(\"{net}\".to_string(), \
                             ::txhdl::netlist::lit({v}))"
                        ));
                        names.push((String::new(), format!("@tie {net}")));
                    }
                    _ => {
                        return Err(err(
                            span,
                            "a port passed to a child is a name, a tuple \
                             of names, a struct of names, `tie(v)`, or `()`",
                        ))
                    }
                }
            }
            let mut joined: Vec<String> = Vec::new();
            for (port, n) in names {
                if let Some(net) = n.strip_prefix("@net ") {
                    joined.push(format!(
                        "a.push((\"{port}\".to_string(), \
                         \"{net}\".to_string()));"
                    ));
                    continue;
                }
                if let Some((_, ty, net)) =
                    links.iter().find(|(e, _, _)| *e == n)
                {
                    joined.push(format!(
                        "a.extend(::txhdl::netlist::link_args::<{ty}>\
                         (\"{net}\"));"
                    ));
                    continue;
                }
                if let Some(net) = n.strip_prefix("@tie ") {
                    joined.push(format!(
                        "a.push((\"{port}\".to_string(), \
                         \"{net}\".to_string()));"
                    ));
                    continue;
                }
                // `side.path` of a side declared in another file: one of
                // its ports, or a struct of ports nested in it, which only
                // `lowered` can tell apart (issue 498).
                let at = BUNDLED.with(|d| {
                    d.borrow().iter().find(|(p, _, _)| *p == n).map(
                        |(p, ty, path)| {
                            let side =
                                p[..p.len() - path.len() - 1].to_string();
                            (ty.clone(), side, path.clone())
                        },
                    )
                });
                // Joined in order, that is: a struct literal naming the
                // child's port joins one port, as below.
                if let (Some((ty, side, path)), true, false) =
                    (at, port.is_empty(), ends.iter().any(|(e, _, _)| *e == n))
                {
                    joined.push(format!(
                        "a.extend(::txhdl::netlist::bundle_args_at::<{ty}>\
                         (\"{side}\", \"{path}\"));"
                    ));
                    continue;
                }
                // A struct of ports nested in the unit's side, given
                // whole to a field of the child's: `pins: jtag` joins
                // the child's `pins_*` to the side's `jtag_*` in order,
                // which only `lowered` can list (issue 579).
                let nested = BUNDLES.with(|b| {
                    b.borrow()
                        .iter()
                        .find(|(x, _)| *x == n)
                        .map(|(_, t)| t.clone())
                });
                if let (Some(ty), false) = (nested, port.is_empty()) {
                    joined.push(format!(
                        "a.extend(::txhdl::netlist::bundle_args_named::<{ty}>\
                         (\"{port}\", \"{n}\"));"
                    ));
                    continue;
                }
                if let Some(b) = n.strip_prefix("@bundle ") {
                    let (ty, side) = b.rsplit_once(' ').unwrap_or((b, ""));
                    joined.push(format!(
                        "a.extend(::txhdl::netlist::bundle_args::<{ty}>\
                         (\"{side}\"));"
                    ));
                    continue;
                }
                let n = aliases
                    .iter()
                    .find(|(x, _)| *x == n)
                    .map(|(_, t)| t.clone())
                    .unwrap_or(n);
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
                joined.push(format!(
                    "a.push((\"{port}\".to_string(), \"{net}\".to_string()));"
                ));
            }
            instances.push(format!(
                "::txhdl::netlist::instance_of(::txhdl::netlist::child_lowered(\
                 &me.{field}, &format!(\"{{name}}_{field}\")), \"{field}\", \
                 {{ let mut a: Vec<(String, String)> = Vec::new(); {} a }})",
                joined.join(" ")
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
    Ok((nets, instances, ties))
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
        let mut out = item.clone();
        if let Some(c) = companion(&item) {
            if let Ok(ts) = c.parse::<TokenStream>() {
                out.extend(ts);
            }
        }
        return out;
    }
    HELPERS.with(|h| {
        *h.borrow_mut() = find_helpers(Span::call_site().local_file())
    });
    PTYPES.with(|p| p.borrow_mut().clear());
    BUNDLES.with(|p| p.borrow_mut().clear());
    BUNDLED.with(|p| p.borrow_mut().clear());
    PNAMES.with(|p| p.borrow_mut().clear());
    // The wires the inlining names are numbered from one in each
    // unit, so the same source lowers to the same netlist whatever
    // was lowered before it on this thread.
    INLINED.with(|w| w.borrow_mut().clear());
    INLINED_N.with(|c| c.set(0));
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
    // A side may be a struct of this file whose fields are the ports,
    // `inp: S<..>` or `S { a, b }: S<..>`: each field is a port named
    // for the field, in the order the struct declares them, and
    // `inp.a` in `run` is the port `a`.
    let structs = find_port_structs(Span::call_site().local_file());
    let mut bound: Vec<(String, Vec<String>)> = Vec::new();
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
        let fields = match port_struct_fields(&p[colon + 1..], &structs) {
            Ok(f) => f,
            Err(e) => return err(p[colon + 1].span(), &e),
        };
        // A struct declared elsewhere: its fields are not visible here,
        // so each `side.f` the body reads is taken as the port `f`,
        // and the ports themselves come from `Ports` when `lowered`
        // runs. A marker in `pairs` holds its place among the sides.
        if let Some(StructSide::Ports) = fields {
            let n = match &p[..colon] {
                [TokenTree::Ident(n)] => n.to_string(),
                [m, TokenTree::Ident(n)] if is_ident(m, "mut") => n.to_string(),
                _ => {
                    return err(
                        p[0].span(),
                        "a side whose struct is declared in another file \
                         is `name: S`, and `S` implements `Ports`",
                    )
                }
            };
            let ty = text_of(&p[colon + 1..]);
            bound.push((n.clone(), vec!["*".to_string()]));
            BUNDLES.with(|b| b.borrow_mut().push((n.clone(), ty.clone())));
            pairs.push((n, format!("@ports {ty}"), p[0].span()));
            continue;
        }
        if let Some(StructSide::Read(fields)) = fields {
            let names: Vec<String> =
                fields.iter().map(|(n, _)| n.clone()).collect();
            match &p[..colon] {
                [TokenTree::Ident(n)] => bound.push((n.to_string(), names)),
                [m, TokenTree::Ident(n)] if is_ident(m, "mut") => {
                    bound.push((n.to_string(), names))
                }
                [TokenTree::Ident(_), TokenTree::Group(g)]
                    if g.delimiter() == Delimiter::Brace =>
                {
                    let given = split_commas(g);
                    let each = given.iter().all(|f| {
                        matches!(f.as_slice(), [TokenTree::Ident(f)]
                            if names.contains(&f.to_string()))
                    });
                    if !each || given.len() != names.len() {
                        return err(
                            g.span(),
                            "a struct of ports is taken apart whole, each \
                             field by its own name: `S { a, b }: S`",
                        );
                    }
                }
                _ => {
                    return err(
                        p[0].span(),
                        "a side that is a struct of ports is `name: S` or \
                         `S { a, b }: S`",
                    )
                }
            }
            for (n, t) in fields {
                // A field that is itself a struct of ports: its ports are
                // the netlist's under the field's name, `field_sub`, and
                // `side.field.sub` in the body is that port (issue 498).
                let end = ["In<", "Out<", "Tx<", "Rx<", "Pad<"]
                    .iter()
                    .any(|e| t.starts_with(e));
                if !end {
                    bound.push((n.clone(), vec!["*".to_string()]));
                    BUNDLES
                        .with(|b| b.borrow_mut().push((n.clone(), t.clone())));
                    pairs.push((n, format!("@ports {t}"), p[0].span()));
                    continue;
                }
                pairs.push((n, t, p[0].span()));
            }
            continue;
        }
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
                    // An array of ports among the others, `[Rx<T>; N]`:
                    // its ports come from `Ports` when `lowered` runs,
                    // `n_0` to `n_{N-1}`, as for a side that is one
                    // array (#500).
                    if let [TokenTree::Group(a)] = t.as_slice() {
                        if a.delimiter() == Delimiter::Bracket {
                            let ty = text_of(t);
                            let n = n.to_string();
                            bound.push((n.clone(), vec!["*".to_string()]));
                            BUNDLES.with(|b| {
                                b.borrow_mut().push((n.clone(), ty.clone()))
                            });
                            pairs.push((n, format!("@ports {ty}"), a.span()));
                            continue;
                        }
                    }
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
    for (k, (n, t, s)) in pairs.iter().enumerate() {
        if t != "()" && pairs[..k].iter().any(|(m, _, _)| m == n) {
            return err(*s, &format!("port `{n}` is named twice"));
        }
    }
    let mut pnames: Vec<String> = pairs
        .iter()
        .filter(|(_, t, _)| !t.starts_with("@ports "))
        .map(|(n, _, _)| n.clone())
        .collect();
    let mut ports: Vec<String> = Vec::new();
    // The ports by name and kind, for a unit of units' joins.
    let mut pkinds: Vec<(String, String)> = Vec::new();
    // The names the netlist gives the ports, each with where it is
    // declared.
    let mut port_nets: Vec<(String, String, Span)> = Vec::new();
    for (pname, ty, span) in pairs {
        if ty == "()" {
            continue;
        }
        if let Some(b) = ty.strip_prefix("@ports ") {
            ports.push(format!(
                "p.extend(::txhdl::netlist::bundle_ports::<{b}>(\"{pname}\"));"
            ));
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
        } else if let Some(x) = ty.strip_prefix("Pad<") {
            ("Pad", x)
        } else {
            return err(span, "a port must be an Out, In, Tx, Rx or Pad");
        };
        // The transaction type: up to the clock argument, if any.
        // What follows that comma is the port's clock, and it is the
        // one place the domain is still written down: by the time the
        // netlist is read the type is gone, and a testbench that does
        // not know a port's clock checks it against the wrong edge
        // (issue 131). A port that names none is on the default clock,
        // as the type's own default argument says.
        let inner = inner.strip_suffix('>').unwrap_or(inner);
        let (inner, clock) = match depth0_comma(inner) {
            Some(c) => (&inner[..c], inner[c + 1..].trim().to_string()),
            None => (inner, "::txhdl::comp::DefaultClock".to_string()),
        };
        ports.push(format!(
            "p.push((\"{pname}\".to_string(), ::txhdl::comp::trace::Kind::{kind}, \
             <{inner} as ::txhdl::types::Value>::WIDTH, \
             <{clock} as ::txhdl::comp::Clock>::NAME));"
        ));
        pkinds.push((pname.clone(), kind.to_string()));
        if kind == "Tx" || kind == "Rx" {
            for end in ["data", "valid", "ready"] {
                port_nets.push((pname.clone(), format!("{pname}_{end}"), span));
            }
        } else {
            // A port a target reserves is escaped where the netlist is
            // written, so what the netlist holds is the escaped name,
            // and that is the name no field may take (issue 497).
            port_nets.push((pname.clone(), escaped(&pname), span));
        }
        PTYPES
            .with(|p| p.borrow_mut().push((pname.clone(), inner.to_string())));
        PNAMES.with(|p| p.borrow_mut().push(pname.clone()));
    }
    // run's body: the brace group after its parameters; inside it, the loop.
    fn is_brace(t: &&TokenTree) -> bool {
        matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Brace)
    }
    let Some(TokenTree::Group(fbody)) = bt[r + 2..].iter().find(is_brace)
    else {
        return err(body.span(), "expected run's body");
    };
    let mut fbody_ports =
        Group::new(fbody.delimiter(), port_fields(fbody.stream(), &bound));
    fbody_ports.set_span(fbody.span());
    let fbody = &fbody_ports;
    // The ports read through a `Ports` side are names of the unit's
    // ports as much as any other, for the checks on wires' names.
    // A unit of units joins them to its children as it joins any port,
    // but their kinds are the struct's and cannot be seen here, so the
    // checks that a channel is joined once, and joined at all, are made
    // by `Lowered::checked` when `lowered` runs.
    for (f, _, _) in BUNDLED.with(|d| d.borrow().clone()) {
        if !pnames.contains(&f) {
            PNAMES.with(|p| p.borrow_mut().push(f.clone()));
            pkinds.push((f.clone(), "Ports".to_string()));
            pnames.push(f);
        }
    }
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
    let mut ties: Vec<String> = Vec::new();
    if loops.is_empty() {
        match lower_structural(fbody, &pkinds, &bound) {
            Ok((n, i, t)) => {
                nets = n;
                instances = i;
                ties = t;
            }
            Err(e) => return e,
        }
    }
    // `let` names that became wires of the netlist, with what drives
    // each; a name bound twice gets a numbered second wire.
    let mut wires: Vec<(String, String)> = Vec::new();
    let mut wire_alts: Vec<String> = Vec::new();
    let mut named: Vec<(String, String, Span)> = Vec::new();
    let mut procs: Vec<String> = Vec::new();
    // The hidden registers of the processes of several waits: the
    // name and the width of each (issue 501).
    let mut hidden: Vec<(String, String, Span)> = Vec::new();
    for lbody in &loops {
        let toks: Vec<TokenTree> = lbody.stream().into_iter().collect();
        let mut cx = Cx {
            pnames: &pnames,
            wires: &mut wires,
            wire_alts: &mut wire_alts,
            named: &mut named,
            subst: Vec::new(),
            guard: None,
            clock: String::new(),
            falling: false,
            hoisted: Vec::new(),
            loops: Vec::new(),
        };
        // A loop of one wait is one clocked block; a loop of several
        // is a state machine, a state per wait, its number in a
        // register the unit does not declare (issue 501).
        let sts = stmts_of(&toks);
        let waits = seq::waits_in(&sts);
        let mut stmts = if waits > 1 {
            let machines = hidden
                .iter()
                .filter(|(r, _, _)| r.starts_with("at_wait"))
                .count();
            let reg = seq::reg_name(machines + 1);
            match seq::lower(&mut cx, &sts, &reg, &pnames, lbody.span()) {
                Ok((s, w, regs)) => {
                    hidden.push((reg, w.to_string(), lbody.span()));
                    for (r, w) in regs {
                        hidden.push((r, w, lbody.span()));
                    }
                    s
                }
                Err(e) => return e,
            }
        } else {
            match lower_stmts(&mut cx, &toks, None) {
                Ok(s) => s,
                Err(e) => return e,
            }
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
         body: {} }}",
            stmts_code(&stmts)
        ));
    }
    // A `let` whose name a target reserves is not refused: the wire
    // takes another name, by the rule above, and the netlist says so
    // (issue 171). A port and a field are not refused either: the
    // netlist escapes them, and the trace and the testbench follow
    // (issue 497).
    // A wire or a port that takes the name of a field is declared twice
    // in the netlist. The fields are the struct's, which this attribute
    // does not see, so the check is a constant the compiler evaluates,
    // written at each name, which `lowered` uses.
    // One constant a name, placed wholly at the name, since the
    // compiler reports a failed constant at the constant.
    let mut named_nets: Vec<(String, String, Span)> = Vec::new();
    for (pname, net, span) in &port_nets {
        named_nets.push((
            net.clone(),
            format!(
                "port `{pname}` is `{net}` in the netlist, and the unit has a \
                 field `{net}`: the netlist would declare `{net}` twice, so \
                 rename one (see issue 77)"
            ),
            *span,
        ));
    }
    // The hidden register of a process of several waits is a field of
    // the netlist the struct does not have, so a field of that name
    // would be declared twice (issue 501).
    for (reg, _, span) in &hidden {
        named_nets.push((
            reg.clone(),
            format!(
                "the loop waits more than once, so the netlist keeps the \
                 wait it is at in a register `{reg}`, and the unit has a \
                 field `{reg}`: rename the field"
            ),
            *span,
        ));
    }
    let mut checks = TokenStream::new();
    let mut uses = String::new();
    // The name of each wire: what the `let` asked for, unless a field
    // of the unit has that name, in which case the alternative chosen
    // above. A constant, so that the answer is the same for every
    // type the unit is lowered at, and so that a generic unit needs no
    // instantiation to be right.
    // One per wire, and the wires are not only the `let`s: the
    // inlining of a call makes a wire of its own for a value the body
    // reads more than once, and those are in `wires` and not in
    // `named`. The index is the wire's, which is what the references
    // were written with.
    let taken_names: Vec<String> = pnames
        .iter()
        .cloned()
        .chain(wires.iter().map(|(w, _)| w.clone()))
        .collect();
    for (k, (w, _)) in wires.iter().enumerate() {
        // The name the constant falls back to when a field has taken
        // this one: `_w` until nothing else has it.
        let mut alt = match wire_alts.get(k) {
            Some(a) => a.clone(),
            None => format!("{w}_w"),
        };
        while taken_names.contains(&alt) {
            alt = format!("{alt}_w");
        }
        let span = named
            .iter()
            .find(|(_, net, _)| net == w)
            .map(|(_, _, s)| *s)
            .unwrap_or_else(Span::call_site);
        let text = format!(
            "#[doc(hidden)] const __TXHDL_WIRE_{k}: &'static str = \
             ::txhdl::netlist::wire_name(\
             <Self as ::txhdl::netlist::Fields>::NAMES, {w:?}, {alt:?});"
        );
        checks.extend(placed_at(text.parse().unwrap(), span));
    }
    for (k, (net, msg, span)) in named_nets.iter().enumerate() {
        uses.push_str(&format!("let () = Self::__TXHDL_NAME_{k};\n"));
        let text = format!(
            "#[doc(hidden)] const __TXHDL_NAME_{k}: () = \
             if ::txhdl::netlist::has_name(\
             <Self as ::txhdl::netlist::Fields>::NAMES, {net:?}) \
             {{ ::core::panic!({msg:?}) }};"
        );
        checks.extend(placed_at(text.parse().unwrap(), *span));
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
         {uses}\
         {prelude}\
         // The wires the helpers of other files ask for (issue 504).\n\
         ::txhdl::netlist::inlined_begin();\n\
         #[allow(unused_mut)] let mut __dynw: Vec<(String, NlE)> = Vec::new();\n\
         #[allow(unused_mut)] let mut __wn: usize = 0;\n\
         let __procs: Vec<::txhdl::netlist::Process> = vec![{procs}];\n\
         // The children, before the literal, since a child of an array\n\
         // held at a constant adds a wire of its own (issue 635).\n\
         #[allow(unused_mut)] let mut __ins: Vec<::txhdl::netlist::Instance> = Vec::new();\n\
         {instances}\n\
         ::txhdl::netlist::Lowered {{\n\
         name: name.to_string(),\n\
         fields: {{ let mut f = <Self as ::txhdl::netlist::Fields>::fields();\n\
         {hidden_fields} f }},\n\
         ports: {{ let mut p = Vec::new(); {ports} p }},\n\
         wires: {{ let mut __w = vec![{wires}]; __w.extend(__dynw); __w }},\n\
         wire_names: vec![{wire_names}],\n\
         procs: __procs,\n\
         init: Vec::new(),\n\
         init_regs: Vec::new(),\n\
         aliases: Vec::new(),\n\
         nets: {{ let mut n: Vec<(String, ::txhdl::comp::trace::Kind, \
         usize, &'static str)> = Vec::new(); {nets} n }},\n\
         instances: __ins,\n\
         foreign: None,\n\
         }}\n\
         .with_inlined(::txhdl::netlist::inlined_end())\n\
         // A field the struct renamed is referred to here under the\n\
         // name Rust knows, since `#[lower]` reads the `impl` and\n\
         // never sees the struct; this writes the netlist's name in.\n\
         .renamed(<Self as ::txhdl::netlist::Fields>::RENAMES)\n\
         // And nothing may take a clock's name, which is a port of the\n\
         // module the netlist adds by itself (issue 367).\n\
         .checked()\n\
         }}\n\
         /// The Verilog of this unit.\n\
         pub fn verilog(name: &str) -> String {{\n\
         Self::lowered(name).verilog() }}\n\
         /// The VHDL of this unit.\n\
         pub fn vhdl(name: &str) -> String {{ Self::lowered(name).vhdl() }}\n\
         }}\n\
         impl{generics} ::txhdl::netlist::Lower for {unit} {{\n\
         fn lowered_as(name: &str) -> ::txhdl::netlist::Lowered {{\n\
         Self::lowered(name) }}\n}}",
        ports = ports.join(" "),
        nets = nets
            .iter()
            .map(|n| match n.strip_prefix("@link ") {
                None if n.starts_with("@raw ") => n[5..].to_string(),
                Some(l) => {
                    let (ty, net) = l.rsplit_once('|').unwrap_or((l, ""));
                    format!(
                        "n.extend(::txhdl::netlist::link_nets::<{ty}>\
                         (\"{net}\"));"
                    )
                }
                None => format!("n.push({n});"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        instances = instances
            .iter()
            .map(|s| match s.strip_prefix("@stmt ") {
                Some(t) => t.to_string(),
                None => format!("__ins.push({s});"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        wires = wires
            .iter()
            .enumerate()
            .map(|(k, (_, e))| {
                format!("(Self::__TXHDL_WIRE_{k}.to_string(), {e})")
            })
            .chain(ties)
            .collect::<Vec<_>>()
            .join(",\n"),
        wire_names = named
            .iter()
            .enumerate()
            .map(|(k, (n, _, _))| {
                format!(
                    "(\"{n}\".to_string(), Self::__TXHDL_WIRE_{k}.to_string())"
                )
            })
            .collect::<Vec<_>>()
            .join(",\n"),
        procs = procs.join(",\n"),
        hidden_fields = hidden
            .iter()
            .map(|(reg, w, _)| {
                format!(
                    "f.push((\"{reg}\", Some(::txhdl::comp::trace::Kind::Reg), \
                     {w}, 0));"
                )
            })
            .collect::<Vec<_>>()
            .join(" "),
    );
    // A name built from a loop's variable, formatted with it (issue 500).
    let generated_text = dyn_names(&generated_text);
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
    // The constants of the checks on names, in an impl of their own,
    // since their tokens are placed at the names they check.
    let header: TokenStream = format!("impl{generics} {unit}").parse().unwrap();
    out.extend(header);
    out.extend([TokenTree::Group(Group::new(Delimiter::Brace, checks))]);
    out
}

/// A register map declared once: see `regmap.rs` (issues 499 and 569).
#[proc_macro]
pub fn regmap(input: TokenStream) -> TokenStream {
    regmap::regmap(input)
}

/// Whether an expression names a `let` the lowering keeps as a Rust
/// variable, `__l_x`, rather than as a wire: one inside a loop, or a
/// `let mut` (#500).
fn reads_rust_var(ts: &[TokenTree], subst: &[(String, String)]) -> bool {
    ts.iter().any(|t| match t {
        TokenTree::Ident(id) => {
            let n = id.to_string();
            subst
                .iter()
                .rev()
                .find(|(k, _)| *k == n)
                .is_some_and(|(_, v)| v.starts_with("__l_"))
        }
        TokenTree::Group(g) => {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            reads_rust_var(&inner, subst)
        }
        _ => false,
    })
}

/// The name of one register of an array of them, `self.f[k]`: `f_k`
/// for a number, and for a loop's variable `f` with the variable
/// between the marks `dyn_names` formats when `lowered` runs. Any other
/// index is a choice among registers made in hardware, which is a
/// multiplexer to write out, not a name (issue 594). `subst`, where it
/// is known, says which names are a loop's variables.
fn indexed_reg(
    f: &str,
    g: &Group,
    subst: Option<&[(String, String)]>,
) -> Result<String, String> {
    let it: Vec<TokenTree> = g.stream().into_iter().collect();
    match it.as_slice() {
        [TokenTree::Literal(k)] => Ok(format!("{f}_{k}")),
        [TokenTree::Ident(v)] => {
            let v = v.to_string();
            let index = subst.is_none_or(|s| {
                s.iter()
                    .rev()
                    .find(|(k, _)| *k == v)
                    .is_some_and(|(_, e)| e.starts_with("NlE::Num("))
            });
            if !index {
                return Err(format!(
                    "`self.{f}[{v}]`: a register of an array is chosen by a \
                     number or by a loop's variable, not by a signal"
                ));
            }
            Ok(format!("{f}{DYN}{v}{DYN_END}"))
        }
        _ => Err(format!(
            "`self.{f}[..]`: the index is a number or a loop's variable"
        )),
    }
}

/// Every join of an array of children in a token list, into any group:
/// `join_all(self.F.iter_mut().enumerate().map(|(i, c)| c.run(ARGS)))`,
/// as the field, the index's name and the arguments (issue 635).
fn find_array_runs(
    ts: &[TokenTree],
    out: &mut Vec<(String, String, Group, Span)>,
) {
    let mut k = 0;
    while k < ts.len() {
        if let (TokenTree::Ident(j), Some(TokenTree::Group(g))) =
            (&ts[k], ts.get(k + 1))
        {
            if j.to_string() == "join_all"
                && g.delimiter() == Delimiter::Parenthesis
            {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                if let Some(found) = array_run(&inner) {
                    out.push(found);
                    k += 2;
                    continue;
                }
            }
        }
        if let TokenTree::Group(g) = &ts[k] {
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            find_array_runs(&inner, out);
        }
        k += 1;
    }
}

/// `self.F.iter_mut().enumerate().map(|(i, c)| c.run(ARGS))`, read.
fn array_run(ts: &[TokenTree]) -> Option<(String, String, Group, Span)> {
    let dot = |k: usize| punct_at(ts, k, '.');
    let empty = |k: usize| {
        matches!(ts.get(k), Some(TokenTree::Group(g))
            if g.delimiter() == Delimiter::Parenthesis && g.stream().is_empty())
    };
    let (Some(s), Some(TokenTree::Ident(f))) = (ts.first(), ts.get(2)) else {
        return None;
    };
    if !(is_ident(s, "self")
        && dot(1)
        && dot(3)
        && ts.get(4).is_some_and(|t| is_ident(t, "iter_mut"))
        && empty(5)
        && dot(6)
        && ts.get(7).is_some_and(|t| is_ident(t, "enumerate"))
        && empty(8)
        && dot(9)
        && ts.get(10).is_some_and(|t| is_ident(t, "map"))
        && ts.len() == 12)
    {
        return None;
    }
    let Some(TokenTree::Group(m)) = ts.get(11) else {
        return None;
    };
    let mut c: Vec<TokenTree> = m.stream().into_iter().collect();
    // `| (i, v) | v . run (ARGS)`, or the call in braces, as rustfmt
    // writes a closure whose call runs over a line.
    if let [a, b, d, TokenTree::Group(body)] = c.as_slice() {
        if body.delimiter() == Delimiter::Brace {
            let mut flat = vec![a.clone(), b.clone(), d.clone()];
            flat.extend(body.stream());
            c = flat;
        }
    }
    let (
        Some(TokenTree::Group(pair)),
        Some(TokenTree::Ident(v2)),
        Some(TokenTree::Group(args)),
    ) = (c.get(1), c.get(3), c.get(6))
    else {
        return None;
    };
    let pv = split_commas(pair);
    let (Some([TokenTree::Ident(i)]), Some([TokenTree::Ident(v)])) = (
        pv.first().map(|x| x.as_slice()),
        pv.get(1).map(|x| x.as_slice()),
    ) else {
        return None;
    };
    if !(punct_at(&c, 0, '|')
        && punct_at(&c, 2, '|')
        && v.to_string() == v2.to_string()
        && punct_at(&c, 4, '.')
        && c.get(5).is_some_and(|t| is_ident(t, "run"))
        && c.len() == 7)
    {
        return None;
    }
    Some((f.to_string(), i.to_string(), args.clone(), f.span()))
}
