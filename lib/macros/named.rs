// SPDX-License-Identifier: Apache-2.0
//! `#[named_type_params]`: a struct's type parameters given by name at
//! every use (#1899).
//!
//! Rust has no named type arguments, so two type parameters with the
//! same bounds can be swapped at a use and the program still compiles.
//! On a struct, the attribute makes, for each type parameter `P` with
//! bounds `B`:
//!
//! * a marker, `struct P<T>(PhantomData<T>)`, which names the argument
//!   at the use: `Bus<Addr<u32>, Data<u64>>`;
//! * a sealed role, `trait IsP { type Repr: B; }`, implemented by `P<T>`
//!   alone, with `Repr = T`;
//!
//! and renames the parameter to `__PParam: IsP`, so it does not hide
//! the marker, and reads each of its uses in the fields and the `where`
//! clause as `<__PParam as IsP>::Repr`. A field typed `P` is then of
//! the plain type `T` the use named, and a use that puts one marker
//! where another belongs, or a bare type where a marker belongs, is
//! refused by the compiler: `Q<u8>` does not implement `IsP`.
//!
//! A default carries over, as the marker: `C: Clock = DefaultClock` is
//! `__CParam: IsC = C<DefaultClock>`. Lifetimes and const parameters
//! are left as they are. The markers and roles take the struct's
//! visibility and are put beside it, so their names are the parameters'
//! and should say what each is.
//!
//! It is written on the tokens, as every macro of this crate is.
use proc_macro::{
    Delimiter, Group, Ident, Spacing, Span, TokenStream, TokenTree,
};

use super::{err, is_ident};

/// One type parameter: its name, its bounds' tokens, and its default's.
struct TypeParam {
    name: Ident,
    bounds: Vec<TokenTree>,
    default: Option<Vec<TokenTree>>,
}

/// A generic parameter as written: a type parameter to name, a const
/// parameter, named only at a use through the struct's macro, or a
/// lifetime, left as it is.
enum Param {
    Type(TypeParam),
    /// Its name, its tokens as written, and whether it has a default.
    Const(String, Vec<TokenTree>, bool),
    Lifetime(Vec<TokenTree>),
}

pub(crate) fn named_type_params(
    attr: TokenStream,
    item: TokenStream,
) -> TokenStream {
    if let Some(t) = attr.into_iter().next() {
        return err(t.span(), "`#[named_type_params]` takes no arguments");
    }
    let toks: Vec<TokenTree> = item.into_iter().collect();
    let Some(kw) = toks.iter().position(|t| {
        is_ident(t, "struct") || is_ident(t, "enum") || is_ident(t, "union")
    }) else {
        return err(
            Span::call_site(),
            "`#[named_type_params]` goes on a struct",
        );
    };
    if !is_ident(&toks[kw], "struct") {
        return err(
            toks[kw].span(),
            "`#[named_type_params]` goes on a struct, not an enum or a union",
        );
    }
    // What comes before `struct`: attributes, doc comments, and the
    // visibility, which the markers and roles take too.
    let head = &toks[..kw];
    let vis: Vec<TokenTree> = match head.iter().position(|t| is_ident(t, "pub"))
    {
        Some(p) => head[p..].to_vec(),
        None => Vec::new(),
    };
    let Some(TokenTree::Ident(name)) = toks.get(kw + 1) else {
        return err(toks[kw].span(), "a struct needs a name");
    };
    let Some(lt) = toks.get(kw + 2).filter(|t| is_punct(t, '<')) else {
        return err(
            name.span(),
            "`#[named_type_params]` names type and const parameters, and \
             this struct has none",
        );
    };
    let lt_span = lt.span();
    // The generic list: up to the `>` that closes it.
    let mut depth = 0i32;
    let mut close = None;
    for (k, t) in toks.iter().enumerate().skip(kw + 2) {
        if is_punct(t, '<') {
            depth += 1;
        } else if is_punct(t, '>') && !after_dash(&toks, k) {
            depth -= 1;
            if depth == 0 {
                close = Some(k);
                break;
            }
        }
    }
    let Some(close) = close else {
        return err(lt_span, "the generic parameters do not close");
    };
    let params = match parse_params(&toks[kw + 3..close]) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let types: Vec<&TypeParam> = params
        .iter()
        .filter_map(|p| match p {
            Param::Type(t) => Some(t),
            _ => None,
        })
        .collect();
    let consts = params
        .iter()
        .filter(|p| matches!(p, Param::Const(..)))
        .count();
    if types.is_empty() && consts == 0 {
        return err(
            name.span(),
            "`#[named_type_params]` names type and const parameters, and this \
             struct has none",
        );
    }
    let names: Vec<String> = types.iter().map(|t| t.name.to_string()).collect();
    // A bound or a default that names a parameter: the role's bounds
    // are a trait's, which has no parameters to name, so such a bound
    // goes in the `where` clause, which is read through the roles.
    for t in &types {
        let default = t.default.clone().unwrap_or_default();
        for (what, ts) in [("bound", &t.bounds), ("default", &default)] {
            if let Some(n) = names_in(ts, &names) {
                return err(
                    n.span(),
                    &format!(
                        "`{}`'s {what} names the parameter `{n}`: put it in \
                         the `where` clause, `where {}: ..`",
                        t.name, t.name
                    ),
                );
            }
        }
    }
    let vis_s = text(&vis);
    let sealed = format!("__named_{}", snake(&name.to_string()));
    let mut out = String::new();
    // The seal: a role is implemented by its marker alone.
    out.push_str(&format!(
        "#[doc(hidden)] #[allow(non_snake_case)] {vis_s} mod {sealed} {{ \
         pub trait Sealed {{}} }}\n"
    ));
    for t in &types {
        let p = t.name.to_string();
        let b = text(&t.bounds);
        let colon = if t.bounds.is_empty() { "" } else { ":" };
        out.push_str(&format!(
            "/// Names the `{p}` argument of [`{name}`]: `{p}<T>` where `T` is \
             the type it stands for.\n\
             {vis_s} struct {p}<T: ?Sized>(\
             ::core::marker::PhantomData<T>);\n\
             /// What may stand for `{p}` in [`{name}`]: its marker, `{p}<T>`, \
             alone.\n\
             {vis_s} trait Is{p}: {sealed}::Sealed {{ \
             /// The type `{p}` stands for.\n type Repr{colon} {b}; }}\n\
             impl<T{colon} {b}> {sealed}::Sealed for {p}<T> {{}}\n\
             impl<T: ?Sized> ::core::clone::Clone for {p}<T> {{ \
             fn clone(&self) -> Self {{ *self }} }}\n\
             impl<T: ?Sized> ::core::marker::Copy for {p}<T> {{}}\n\
             impl<T: ?Sized> ::core::default::Default for {p}<T> {{ \
             fn default() -> Self {{ {p}(::core::marker::PhantomData) }} }}\n\
             impl<T: ?Sized> ::core::fmt::Debug for {p}<T> {{ \
             fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> \
             ::core::fmt::Result {{ f.write_str(\"{p}\") }} }}\n\
             impl<T: ?Sized> ::core::cmp::PartialEq for {p}<T> {{ \
             fn eq(&self, _: &Self) -> bool {{ true }} }}\n\
             impl<T: ?Sized> ::core::cmp::Eq for {p}<T> {{}}\n\
             impl<T: ?Sized> ::core::hash::Hash for {p}<T> {{ \
             fn hash<H: ::core::hash::Hasher>(&self, _: &mut H) {{}} }}\n\
             impl<T{colon} {b}> Is{p} for {p}<T> {{ type Repr = T; }}\n"
        ));
    }
    // The struct: each type parameter renamed and bound to its role,
    // a default as its marker, and every use of a name in the rest read
    // through the role.
    let mut generics: Vec<String> = Vec::new();
    for p in &params {
        match p {
            Param::Lifetime(ts) | Param::Const(_, ts, _) => {
                generics.push(text(ts))
            }
            Param::Type(t) => {
                let n = t.name.to_string();
                let mut g = format!("__{n}Param: Is{n}");
                if let Some(d) = &t.default {
                    g.push_str(&format!(
                        " = {n}<{}>",
                        text(&match rewrite(d, &names, Place::Types) {
                            Ok(r) => r,
                            Err(e) => return e,
                        })
                    ));
                }
                generics.push(g);
            }
        }
    }
    out.push_str(&use_macro(&name.to_string(), &params, !vis.is_empty()));
    let before: TokenStream = toks[..=kw + 1].iter().cloned().collect();
    let rest = match rewrite(&toks[close + 1..], &names, Place::Top) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let mut ts: TokenStream = out.parse().expect("the markers parse");
    ts.extend(before);
    let g: TokenStream = format!("<{}>", generics.join(", "))
        .parse()
        .expect("the parameters parse");
    ts.extend(g);
    ts.extend(rest);
    ts
}

/// The struct's own macro, `Name![P = v, ..]`: every parameter given
/// by name, in any order, written out in the struct's order as
/// `Name<..>`, a type parameter's value in its marker, a const
/// parameter's in braces, and a lifetime's as it is, `'a = 'static`. A
/// lifetime left out is `'_`. A name given twice, one the struct does
/// not have, or one left out that has no default, is refused when the
/// macro is expanded.
///
/// It is how a const parameter is given by name (#1899): stable Rust
/// does not let a generic parameter stand in a const expression, so a
/// number cannot be read out of a marker the way a type is
/// (`experiments/rust_embedding/probe_named_const.rs`). The positional
/// form `Name<32, ..>` still compiles; the macro is what checks.
///
/// A public struct's macro is exported at the crate's root under the
/// struct's name, so from another crate it is imported by that name,
/// `use the_crate::Name;`; two public structs of one name in a crate
/// would export two macros of it, which Rust refuses. A private
/// struct's is not exported. In its own crate the macro is used after
/// the struct, as any `macro_rules!` is. It calls itself and writes
/// the struct and the markers by their bare names, so a use needs them
/// in scope: Rust lets no path reach a macro another macro made, from
/// inside its own crate.
fn use_macro(name: &str, params: &[Param], public: bool) -> String {
    #[derive(PartialEq)]
    enum Kind {
        Type,
        Const,
        Lifetime,
    }
    // A slot per parameter: `[]` empty, `[~ d]` empty with the default
    // `d` to fall back on, `[v]` given.
    struct Slot {
        name: String,
        kind: Kind,
        default: Option<String>,
    }
    let mut slots: Vec<Slot> = Vec::new();
    for p in params {
        match p {
            Param::Type(t) => slots.push(Slot {
                name: t.name.to_string(),
                kind: Kind::Type,
                default: t.default.as_ref().map(|d| text(d)),
            }),
            Param::Const(n, ts, has) => {
                let default = if *has {
                    let e = top_level(ts, '=').expect("a default");
                    Some(format!("{{ {} }}", text(&ts[e + 1..])))
                } else {
                    None
                };
                slots.push(Slot {
                    name: n.clone(),
                    kind: Kind::Const,
                    default,
                })
            }
            Param::Lifetime(ts) => {
                // `'a` is a `'` joined to `a`.
                let n: String =
                    ts.iter().take(2).map(|t| t.to_string()).collect();
                slots.push(Slot {
                    name: n,
                    kind: Kind::Lifetime,
                    default: Some("'_".to_string()),
                })
            }
        }
    }
    let k = slots.len();
    let var = |j: usize| format!("[$($s{j}:tt)*]");
    let pass = |j: usize| format!("[$($s{j})*]");
    let names: Vec<String> = slots.iter().map(|s| s.name.clone()).collect();
    let refusal = format!(
        "`{name}![..]` gives each of its parameters by name, once: {}",
        names.join(", ")
    );
    let mut rules: Vec<String> = Vec::new();
    // One argument at a time, into its slot, which must be empty.
    for (j, s) in slots.iter().enumerate() {
        let (frag, given) = match s.kind {
            Kind::Type => ("ty", format!("[{}<$v>]", s.name)),
            Kind::Const => ("expr", "[{ $v }]".to_string()),
            Kind::Lifetime => ("lifetime", "[$v]".to_string()),
        };
        for empty in ["[]", "[~ $($d:tt)*]"] {
            let lhs: Vec<String> = (0..k)
                .map(|i| if i == j { empty.to_string() } else { var(i) })
                .collect();
            let rhs: Vec<String> = (0..k)
                .map(|i| if i == j { given.clone() } else { pass(i) })
                .collect();
            rules.push(format!(
                "(@s {} ; {} = $v:{frag} $(, $($r:tt)*)?) => {{ \
                 {name}!(@s {} ; $($($r)*)?) }};",
                lhs.join(" "),
                s.name,
                rhs.join(" ")
            ));
        }
    }
    // Every argument read: a default not overridden is used, one slot
    // at a time, and then the struct is written out.
    for (j, s) in slots.iter().enumerate() {
        let fallback = if s.kind == Kind::Type {
            format!("[{}<$($d)*>]", s.name)
        } else {
            "[$($d)*]".to_string()
        };
        let lhs: Vec<String> = (0..k)
            .map(|i| {
                if i == j {
                    "[~ $($d:tt)*]".to_string()
                } else {
                    var(i)
                }
            })
            .collect();
        let rhs: Vec<String> = (0..k)
            .map(|i| if i == j { fallback.clone() } else { pass(i) })
            .collect();
        rules.push(format!(
            "(@s {} ;) => {{ {name}!(@s {} ;) }};",
            lhs.join(" "),
            rhs.join(" ")
        ));
    }
    let full: Vec<String> = (0..k).map(|j| format!("[$($s{j}:tt)+]")).collect();
    let args: Vec<String> = (0..k).map(|j| format!("$($s{j})+")).collect();
    rules.push(format!(
        "(@s {} ;) => {{ {name}<{}> }};",
        full.join(" "),
        args.join(", ")
    ));
    rules.push(format!(
        "(@s $($x:tt)*) => {{ ::core::compile_error!({refusal:?}) }};"
    ));
    // Last, so the steps above, which start with `@s`, are not read as
    // a fresh start.
    let start: Vec<String> = slots
        .iter()
        .map(|s| match &s.default {
            Some(d) => format!("[~ {d}]"),
            None => "[]".to_string(),
        })
        .collect();
    rules.push(format!(
        "($($a:tt)*) => {{ {name}!(@s {} ; $($a)*) }};",
        start.join(" ")
    ));
    let export = if public { "#[macro_export]\n" } else { "" };
    format!(
        "#[doc = \"`{name}![P = v, ..]`: [`{name}`] with every parameter \
         given by name, in any order (#1899).\"]\n\
         {export}macro_rules! {name} {{\n{}\n}}\n",
        rules.join("\n")
    )
}

/// The generic parameters between `<` and `>`, split at the top-level
/// commas.
fn parse_params(ts: &[TokenTree]) -> Result<Vec<Param>, TokenStream> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur: Vec<TokenTree> = Vec::new();
    let mut parts: Vec<Vec<TokenTree>> = Vec::new();
    for (k, t) in ts.iter().enumerate() {
        if is_punct(t, '<') {
            depth += 1;
        } else if is_punct(t, '>') && !after_dash(ts, k) {
            depth -= 1;
        } else if is_punct(t, ',') && depth == 0 {
            parts.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(t.clone());
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    for part in parts {
        match part.first() {
            Some(TokenTree::Punct(p)) if p.as_char() == '\'' => {
                out.push(Param::Lifetime(part))
            }
            Some(t) if is_ident(t, "const") => {
                let Some(TokenTree::Ident(n)) = part.get(1) else {
                    return Err(err(
                        t.span(),
                        "a const parameter is `const N: T`",
                    ));
                };
                let n = n.to_string();
                let default = top_level(&part, '=').is_some();
                out.push(Param::Const(n, part, default))
            }
            Some(TokenTree::Ident(n)) => {
                let eq = top_level(&part, '=');
                let (decl, default) = match eq {
                    Some(e) => (&part[..e], Some(part[e + 1..].to_vec())),
                    None => (&part[..], None),
                };
                let bounds = match decl.get(1) {
                    Some(t) if is_punct(t, ':') => decl[2..].to_vec(),
                    None => Vec::new(),
                    Some(t) => {
                        return Err(err(
                            t.span(),
                            "a type parameter is `Name: Bounds`",
                        ))
                    }
                };
                out.push(Param::Type(TypeParam {
                    name: n.clone(),
                    bounds,
                    default,
                }));
            }
            Some(t) => {
                return Err(err(
                    t.span(),
                    "a generic parameter this macro does not read",
                ))
            }
            None => {}
        }
    }
    Ok(out)
}

/// The first `c` at depth zero of angle brackets.
fn top_level(ts: &[TokenTree], c: char) -> Option<usize> {
    let mut depth = 0i32;
    for (k, t) in ts.iter().enumerate() {
        if is_punct(t, '<') {
            depth += 1;
        } else if is_punct(t, '>') && !after_dash(ts, k) {
            depth -= 1;
        } else if depth == 0 && is_punct(t, c) {
            return Some(k);
        }
    }
    None
}

/// Where a token list is: what follows the generic list, whose braced
/// group is the struct's body; the fields of that body, where a field's
/// own name, `name:`, is not a type; or anywhere else.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Top,
    Fields,
    Types,
}

/// Tokens with each use of a parameter's name read through its role,
/// into every group: `P` becomes `<__PParam as IsP>::Repr`. A name
/// after `::` or `.` is a path's or a field's, not the parameter; a
/// field's own name, and an attribute, are left as they are. `P::Name`
/// is refused, since a projection cannot be followed by `::`: it is
/// written `<P as Trait>::Name`, which reads through the role.
fn rewrite(
    ts: &[TokenTree],
    names: &[String],
    place: Place,
) -> Result<Vec<TokenTree>, TokenStream> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < ts.len() {
        let t = &ts[k];
        // An attribute, `#[..]`, is left as it is.
        if is_punct(t, '#') {
            if let Some(TokenTree::Group(g)) = ts.get(k + 1) {
                if g.delimiter() == Delimiter::Bracket {
                    out.push(t.clone());
                    out.push(ts[k + 1].clone());
                    k += 2;
                    continue;
                }
            }
        }
        match t {
            TokenTree::Ident(i)
                if names.contains(&i.to_string()) && !in_path(ts, k) =>
            {
                let colon =
                    |j: usize| ts.get(j).is_some_and(|t| is_punct(t, ':'));
                if colon(k + 1) && colon(k + 2) {
                    return Err(err(
                        i.span(),
                        &format!(
                            "`{i}::..` cannot be read through `{i}`'s marker: \
                             write `<{i} as Trait>::..`"
                        ),
                    ));
                }
                if place == Place::Fields && colon(k + 1) {
                    // A field named like the parameter: its name.
                    out.push(t.clone());
                    k += 1;
                    continue;
                }
                let n = i.to_string();
                let s = format!("<__{n}Param as Is{n}>::Repr");
                let mut r: Vec<TokenTree> = s
                    .parse::<TokenStream>()
                    .expect("a projection parses")
                    .into_iter()
                    .collect();
                for x in r.iter_mut() {
                    x.set_span(i.span());
                }
                out.extend(r);
            }
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                // The struct's braced body holds named fields; every
                // group inside it, and everywhere else, holds types.
                let within = if place == Place::Top
                    && g.delimiter() == Delimiter::Brace
                {
                    Place::Fields
                } else {
                    Place::Types
                };
                let mut ng = Group::new(
                    g.delimiter(),
                    rewrite(&inner, names, within)?.into_iter().collect(),
                );
                ng.set_span(g.span());
                out.push(TokenTree::Group(ng));
            }
            _ => out.push(t.clone()),
        }
        k += 1;
    }
    Ok(out)
}

/// The first of `names` among the tokens, into every group.
fn names_in(ts: &[TokenTree], names: &[String]) -> Option<Ident> {
    ts.iter().find_map(|t| match t {
        TokenTree::Ident(i) if names.contains(&i.to_string()) => {
            Some(i.clone())
        }
        TokenTree::Group(g) => {
            names_in(&g.stream().into_iter().collect::<Vec<_>>(), names)
        }
        _ => None,
    })
}

/// Whether the name at `k` follows `::` or `.`, so it is a path's
/// segment or a field, not a parameter.
fn in_path(ts: &[TokenTree], k: usize) -> bool {
    match (
        k.checked_sub(2).map(|j| &ts[j]),
        k.checked_sub(1).map(|j| &ts[j]),
    ) {
        (Some(a), Some(b)) if is_punct(a, ':') && is_punct(b, ':') => true,
        (_, Some(b)) if is_punct(b, '.') => true,
        _ => false,
    }
}

/// Whether the `>` at `k` is the head of `->`, not a closing bracket.
fn after_dash(ts: &[TokenTree], k: usize) -> bool {
    k > 0
        && matches!(&ts[k - 1], TokenTree::Punct(p)
            if p.as_char() == '-' && p.spacing() == Spacing::Joint)
}

fn is_punct(t: &TokenTree, c: char) -> bool {
    matches!(t, TokenTree::Punct(p) if p.as_char() == c)
}

/// Tokens as source text, for the parts that are written as text.
fn text(ts: &[TokenTree]) -> String {
    ts.iter().cloned().collect::<TokenStream>().to_string()
}

/// `SetTable` as `set_table`, for the seal's module.
pub(crate) fn snake(s: &str) -> String {
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if k > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
