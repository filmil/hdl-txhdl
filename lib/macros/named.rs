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
//! On a type alias of const parameters it makes no markers and changes
//! nothing: it writes the alias's macro only, so `Aw![ADDR = 32, ID = 2]`
//! names an alias's parameters as a struct's macro does (#1946).
//!
//! `path = crate::m`, the module the struct or alias is in, makes the
//! use-site macro reachable from anywhere (#1915). Rust lets no path
//! reach a macro another macro made, from inside its own crate
//! (`experiments/rust_embedding/probe_named_path.rs`), so the macro is
//! made under a hidden name, `__Name_named`, and re-exported beside the
//! struct as `Name`. It is then `crate::m::Name![..]`, or `Name![..]`
//! after `use crate::m::Name;`, which brings the struct too, in its own
//! crate and in others. It writes the struct and the markers it makes as
//! `$crate::m::..`, and a marker `reuse(..)` names by path by that path,
//! so a use imports nothing else.
//!
//! It is written on the tokens, as every macro of this crate is.
use proc_macro::{
    Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree,
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
    let (reused, prefix) = match args(attr) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let toks: Vec<TokenTree> = item.into_iter().collect();
    let Some(kw) = toks.iter().position(|t| {
        is_ident(t, "struct")
            || is_ident(t, "enum")
            || is_ident(t, "union")
            || is_ident(t, "type")
    }) else {
        return err(
            Span::call_site(),
            "`#[named_type_params]` goes on a struct, or on a type alias of \
             const parameters",
        );
    };
    if is_ident(&toks[kw], "type") {
        return alias(&reused, &prefix, &toks, kw);
    }
    if !is_ident(&toks[kw], "struct") {
        return err(
            toks[kw].span(),
            "`#[named_type_params]` goes on a struct or a type alias, not an \
             enum or a union",
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
    // A name `reuse(..)` lists that is no type parameter of the struct.
    for (r, span, _) in &reused {
        if !names.contains(r) {
            return err(
                *span,
                &format!("`reuse({r})` names no type parameter of `{name}`"),
            );
        }
    }
    let reuses = |n: &str| reused.iter().any(|(r, _, _)| r == n);
    // The seal: a role is implemented by its marker alone. A struct
    // that makes no marker of its own needs none.
    if types.iter().any(|t| !reuses(&t.name.to_string())) {
        out.push_str(&format!(
            "#[doc(hidden)] #[allow(non_snake_case)] {vis_s} mod {sealed} {{ \
             pub trait Sealed {{}} }}\n"
        ));
    }
    for t in &types {
        let p = t.name.to_string();
        if reuses(&p) {
            continue;
        }
        out.push_str(&marker(
            &vis_s,
            &p,
            &sealed,
            &text(&t.bounds),
            &format!("[`{name}`]"),
        ));
    }
    // The bounds of a reused parameter: its role is shared and has
    // none, so the struct states its own in its `where` clause, read
    // through the role, with the `Sized` a plain parameter has unless
    // it said `?Sized`.
    let mut preds: Vec<String> = Vec::new();
    for t in &types {
        let p = t.name.to_string();
        if !reuses(&p) {
            continue;
        }
        // The bounds, term by term at the top-level `+`: a relaxed one,
        // `?Sized`, is the shared role's already and is not allowed in
        // a `where` clause, so it drops out, and without it the term
        // `Sized` is added, as a plain parameter has it.
        let mut terms: Vec<String> = Vec::new();
        let mut relaxed = false;
        let mut depth = 0i32;
        let mut cur: Vec<TokenTree> = Vec::new();
        let mut flush = |cur: &mut Vec<TokenTree>, relaxed: &mut bool| {
            if cur.first().is_some_and(|x| is_punct(x, '?')) {
                *relaxed = true;
            } else if !cur.is_empty() {
                terms.push(text(cur));
            }
            cur.clear();
        };
        for (k, x) in t.bounds.iter().enumerate() {
            if is_punct(x, '<') {
                depth += 1;
            } else if is_punct(x, '>') && !after_dash(&t.bounds, k) {
                depth -= 1;
            } else if is_punct(x, '+') && depth == 0 {
                flush(&mut cur, &mut relaxed);
                continue;
            }
            cur.push(x.clone());
        }
        flush(&mut cur, &mut relaxed);
        if !relaxed {
            terms.insert(0, "::core::marker::Sized".to_string());
        }
        if !terms.is_empty() {
            preds.push(format!(
                "<__{p}Param as Is{p}>::Repr: {}",
                terms.join(" + ")
            ));
        }
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
    let reused_paths: Vec<(String, String)> = reused
        .iter()
        .map(|(n, _, p)| (n.clone(), p.clone()))
        .collect();
    out.push_str(&use_macro(
        &name.to_string(),
        &params,
        &vis_s,
        &prefix,
        &reused_paths,
    ));
    let before: TokenStream = toks[..=kw + 1].iter().cloned().collect();
    let rest = match rewrite(&toks[close + 1..], &names, Place::Top) {
        Ok(r) => with_preds(r, &preds),
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

/// `#[named_type_params]` on a type alias (#1946): the alias as it is,
/// and the macro of its name that takes its parameters by name, as a
/// struct's does. An alias makes no markers, so it names const
/// parameters only; a type parameter is refused, and so is `reuse(..)`.
fn alias(
    reused: &[(String, Span, String)],
    prefix: &str,
    toks: &[TokenTree],
    kw: usize,
) -> TokenStream {
    if let Some((_, span, _)) = reused.first() {
        return err(
            *span,
            "`reuse(..)` names markers, which a type alias does not take",
        );
    }
    let head = &toks[..kw];
    let alias_vis: Vec<TokenTree> =
        match head.iter().position(|t| is_ident(t, "pub")) {
            Some(p) => head[p..].to_vec(),
            None => Vec::new(),
        };
    let Some(TokenTree::Ident(name)) = toks.get(kw + 1) else {
        return err(toks[kw].span(), "a type alias needs a name");
    };
    let Some(lt) = toks.get(kw + 2).filter(|t| is_punct(t, '<')) else {
        return err(
            name.span(),
            "`#[named_type_params]` names const parameters, and this alias \
             has none",
        );
    };
    let mut depth = 0i32;
    let mut close = None;
    for (k, t) in toks.iter().enumerate().skip(kw + 2) {
        if is_punct(t, '<') {
            depth += 1;
        } else if is_punct(t, '>') && !after_dash(toks, k) {
            depth -= 1;
            if depth == 0 {
                close = Some(k);
                break;
            }
        }
    }
    let Some(close) = close else {
        return err(lt.span(), "the generic parameters do not close");
    };
    let params = match parse_params(&toks[kw + 3..close]) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if params.is_empty() {
        return err(
            name.span(),
            "`#[named_type_params]` names const parameters, and this alias \
             has none",
        );
    }
    if let Some(Param::Type(t)) =
        params.iter().find(|p| matches!(p, Param::Type(_)))
    {
        return err(
            t.name.span(),
            "on a type alias, `#[named_type_params]` names const \
             parameters only: an alias has no markers to name a type by",
        );
    }
    let mut ts: TokenStream =
        use_macro(&name.to_string(), &params, &text(&alias_vis), prefix, &[])
            .parse()
            .expect("the macro parses");
    ts.extend(toks.iter().cloned());
    ts
}

/// A marker `P<T>` and its role `IsP`, sealed by `sealed::Sealed`,
/// with `bounds` on what it names, and the std traits implemented with
/// no bound on `T`, so a derive on a struct that takes it is met.
/// `owner` is what its documentation says it names an argument of.
fn marker(
    vis: &str,
    p: &str,
    sealed: &str,
    bounds: &str,
    owner: &str,
) -> String {
    let colon = if bounds.is_empty() { "" } else { ":" };
    format!(
        "/// Names the `{p}` argument of {owner}: `{p}<T>` where `T` is \
         the type it stands for.\n\
         {vis} struct {p}<T: ?Sized>(::core::marker::PhantomData<T>);\n\
         /// What may stand for `{p}` in {owner}: its marker, `{p}<T>`, \
         alone.\n\
         {vis} trait Is{p}: {sealed}::Sealed {{ \
         /// The type `{p}` stands for.\n type Repr{colon} {bounds}; }}\n\
         impl<T{colon} {bounds}> {sealed}::Sealed for {p}<T> {{}}\n\
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
         impl<T{colon} {bounds}> Is{p} for {p}<T> {{ type Repr = T; }}\n"
    )
}

/// The attribute's arguments, in any order, each at most once:
/// `reuse(A, B)`, the markers reused (#1914), and `path = crate::m`,
/// the module the struct is in (#1915). The second is returned as the
/// prefix the macro writes before the struct's name and the names of the
/// markers it makes, `$crate::m::`, or empty without it. A reused marker
/// is written by the path `reuse(..)` gave it, or bare if given bare.
#[allow(clippy::type_complexity)]
fn args(
    attr: TokenStream,
) -> Result<(Vec<(String, Span, String)>, String), TokenStream> {
    let ts: Vec<TokenTree> = attr.into_iter().collect();
    let mut reuse: Option<Vec<(String, Span, String)>> = None;
    let mut prefix: Option<String> = None;
    let usage = "`#[named_type_params]` takes nothing, `reuse(A, B)`: the \
                 markers it reuses rather than makes, `path = crate::m`: \
                 the module it is in, or both, comma-separated";
    for part in ts.split(|t| is_punct(t, ',')) {
        match part {
            [] => {}
            [TokenTree::Ident(r), TokenTree::Group(_)]
                if *r.to_string() == *"reuse" && reuse.is_none() =>
            {
                reuse = Some(reused(part.iter().cloned().collect())?);
            }
            [TokenTree::Ident(p), TokenTree::Punct(eq), path @ ..]
                if *p.to_string() == *"path"
                    && eq.as_char() == '='
                    && prefix.is_none() =>
            {
                prefix = Some(module_prefix(p.span(), path)?);
            }
            [t, ..] => return Err(err(t.span(), usage)),
        }
    }
    Ok((reuse.unwrap_or_default(), prefix.unwrap_or_default()))
}

/// `crate::a::b` as the prefix `$crate::a::b::`: a path from the
/// crate's root, which reaches the struct from any module and, through
/// `$crate`, from any crate. The macro reaches itself by it too, through
/// the re-export beside the struct: Rust lets no other path reach a macro
/// another macro made, from inside its own crate
/// (`experiments/rust_embedding/probe_named_path.rs`).
fn module_prefix(
    span: Span,
    path: &[TokenTree],
) -> Result<String, TokenStream> {
    let bad = || {
        err(
            span,
            "`path = ..` is the module the struct is in, from the crate's \
             root: `path = crate::bus::axi`",
        )
    };
    let Some(first) = path.first() else {
        return Err(bad());
    };
    if !is_ident(first, "crate") {
        return Err(bad());
    }
    let mut out = String::from("$crate");
    let mut rest = path[1..].iter();
    loop {
        match (rest.next(), rest.next(), rest.next()) {
            (None, _, _) => break,
            (
                Some(TokenTree::Punct(a)),
                Some(TokenTree::Punct(b)),
                Some(TokenTree::Ident(n)),
            ) if a.as_char() == ':' && b.as_char() == ':' => {
                out.push_str("::");
                out.push_str(&n.to_string());
            }
            _ => return Err(bad()),
        }
    }
    out.push_str("::");
    Ok(out)
}

/// The prefix a reused marker is written with, from its path in
/// `reuse(..)`: none for a bare name, `$crate::a::` for `crate::a::M`,
/// `::k::a::` for `::k::a::M`; `None` if the tokens are no such path.
fn marker_path(part: &[TokenTree]) -> Option<String> {
    let colons =
        |a: &TokenTree, b: &TokenTree| is_punct(a, ':') && is_punct(b, ':');
    let mut out = String::new();
    let mut k = 0;
    if part.len() >= 2 && colons(&part[0], &part[1]) {
        out.push_str("::");
        k = 2;
    }
    let mut segs: Vec<String> = Vec::new();
    loop {
        match part.get(k) {
            Some(TokenTree::Ident(i)) => segs.push(i.to_string()),
            _ => return None,
        }
        k += 1;
        if k == part.len() {
            break;
        }
        if k + 1 < part.len() && colons(&part[k], &part[k + 1]) {
            k += 2;
        } else {
            return None;
        }
    }
    // A bare name, `crate::..` or `::krate::..`: anything else is
    // relative to the module, which a use elsewhere is not in.
    let leading = !out.is_empty();
    if leading && segs.first().is_some_and(|f| f == "crate") {
        return None;
    }
    if !leading && segs.len() > 1 && segs[0] != "crate" {
        return None;
    }
    segs.pop();
    for (j, seg) in segs.iter().enumerate() {
        if j == 0 && seg == "crate" && out.is_empty() {
            out.push_str("$crate");
        } else {
            out.push_str(seg);
        }
        out.push_str("::");
    }
    Some(out)
}

/// The markers `reuse(A, B)` lists, each with where it is named and the
/// prefix the use-site macro writes before it; none for no arguments
/// (#1914). A marker given by a path, `reuse(crate::bus::named::Map)` or
/// `reuse(::txhdl::comp::named::Data)`, is written by that path, with
/// `crate` as `$crate`, so a use needs no import of it (#1915); one given
/// by its bare name is written bare, and a use imports it.
#[allow(clippy::type_complexity)]
fn reused(
    attr: TokenStream,
) -> Result<Vec<(String, Span, String)>, TokenStream> {
    let ts: Vec<TokenTree> = attr.into_iter().collect();
    match ts.as_slice() {
        [] => Ok(Vec::new()),
        [TokenTree::Ident(r), TokenTree::Group(g)]
            if *r.to_string() == *"reuse"
                && g.delimiter() == Delimiter::Parenthesis =>
        {
            let mut out: Vec<(String, Span, String)> = Vec::new();
            let inner: Vec<TokenTree> = g.stream().into_iter().collect();
            let parts: Vec<&[TokenTree]> =
                inner.split(|t| is_punct(t, ',')).collect();
            let last = parts.len() - 1;
            for (k, part) in parts.iter().enumerate() {
                match *part {
                    [.., TokenTree::Ident(n)]
                        if marker_path(part).is_some() =>
                    {
                        if out.iter().any(|(m, _, _)| *m == n.to_string()) {
                            return Err(err(
                                n.span(),
                                &format!("`reuse(..)` names `{n}` twice"),
                            ));
                        }
                        let prefix = marker_path(part).unwrap_or_default();
                        out.push((n.to_string(), n.span(), prefix))
                    }
                    // A trailing comma, and nothing else, is allowed.
                    [] if k == last && k > 0 => {}
                    _ => {
                        return Err(err(
                            part.first().map_or(g.span(), |t| t.span()),
                            "`reuse(..)` names markers, once each, by name \
                             or by a path from a crate's root: \
                             `reuse(Data, crate::named::Domain)`",
                        ))
                    }
                }
            }
            if out.is_empty() {
                return Err(err(g.span(), "`reuse()` names no marker"));
            }
            Ok(out)
        }
        [t, ..] => Err(err(
            t.span(),
            "`#[named_type_params]` takes nothing, or `reuse(A, B)`: the \
             markers it reuses rather than makes",
        )),
    }
}

/// What follows the generic list with `preds` added to its `where`
/// clause, or a `where` clause made for them: before the body of a
/// braced struct, after the fields of a tuple struct.
fn with_preds(rest: Vec<TokenTree>, preds: &[String]) -> Vec<TokenTree> {
    if preds.is_empty() {
        return rest;
    }
    let group = |t: Option<&TokenTree>, d: Delimiter| match t {
        Some(TokenTree::Group(g)) => g.delimiter() == d,
        _ => false,
    };
    // A tuple struct's fields come straight after the generic list; a
    // braced struct's body is its last token. A group inside a `where`
    // clause, `Fn(u8) -> u8`, is neither. A unit struct, or a tuple
    // one, ends at its `;`.
    let tuple = group(rest.first(), Delimiter::Parenthesis);
    let braced = !tuple && group(rest.last(), Delimiter::Brace);
    let has_where = rest.iter().any(|t| is_ident(t, "where"));
    let clause_end = if braced {
        rest.len() - 1
    } else {
        rest.iter()
            .rposition(|t| is_punct(t, ';'))
            .unwrap_or(rest.len())
    };
    let mut add = String::new();
    if has_where {
        let ends_comma = clause_end > 0 && is_punct(&rest[clause_end - 1], ',');
        if !ends_comma {
            add.push(',');
        }
    } else {
        add.push_str(" where ");
    }
    add.push_str(&preds.join(", "));
    let add: Vec<TokenTree> = add
        .parse::<TokenStream>()
        .expect("the bounds parse")
        .into_iter()
        .collect();
    let mut out = rest[..clause_end].to_vec();
    out.extend(add);
    out.extend(rest[clause_end..].iter().cloned());
    out
}

/// `reuse(..)` is meant for these: a marker another struct made has
/// that struct's bounds on its role, which a struct reusing it would
/// then have as well as its own.
///
/// `named_markers!(pub Data, pub Domain)`: markers and roles declared
/// once, for every struct that names them in
/// `#[named_type_params(reuse(..))]` (#1914). A role here has no
/// bounds, since it serves structs with different ones; each struct
/// states its own in its `where` clause. The seal is a module of its
/// own beside them, named for the first marker.
pub(crate) fn named_markers(input: TokenStream) -> TokenStream {
    let ts: Vec<TokenTree> = input.into_iter().collect();
    let mut items: Vec<(String, String)> = Vec::new();
    let parts: Vec<&[TokenTree]> = ts.split(|t| is_punct(t, ',')).collect();
    let last = parts.len() - 1;
    let wrong = |span: Span| {
        err(
            span,
            "`named_markers!` lists markers, each once, with its visibility: \
             `named_markers!(pub Data, pub(crate) Domain)`",
        )
    };
    for (k, part) in parts.iter().enumerate() {
        // A visibility, `pub` or `pub(..)`, or none, then the name.
        let (vis, n) = match *part {
            [] if k == last && k > 0 => continue,
            [TokenTree::Ident(n)] => (&part[..0], n),
            [p, TokenTree::Ident(n)] if is_ident(p, "pub") => (&part[..1], n),
            [p, TokenTree::Group(g), TokenTree::Ident(n)]
                if is_ident(p, "pub")
                    && g.delimiter() == Delimiter::Parenthesis =>
            {
                (&part[..2], n)
            }
            _ => {
                return wrong(
                    part.first().map_or(Span::call_site(), |t| t.span()),
                )
            }
        };
        if items.iter().any(|(_, m)| *m == n.to_string()) {
            return wrong(n.span());
        }
        items.push((text(vis), n.to_string()));
    }
    let Some((_, first)) = items.first() else {
        return err(Span::call_site(), "`named_markers!` lists no marker");
    };
    // Public, so a role as public as any marker can name it; hidden,
    // and with nothing to use but the trait the roles are sealed by.
    let sealed = format!("__named_markers_{}", snake(first));
    let mut out = format!(
        "#[doc(hidden)] #[allow(non_snake_case)] pub mod {sealed} {{ \
         pub trait Sealed {{}} }}\n"
    );
    for (vis, n) in &items {
        out.push_str(&marker(
            vis,
            n,
            &sealed,
            "?Sized",
            "a struct that reuses it",
        ));
    }
    out.parse().expect("the markers parse")
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
fn use_macro(
    name: &str,
    params: &[Param],
    vis: &str,
    prefix: &str,
    reused: &[(String, String)],
) -> String {
    let public = vis.contains("pub");
    // The prefix of a marker: the struct's `path` for one it makes, and
    // for one it reuses the path `reuse(..)` gave it.
    let marker = |n: &str| match reused.iter().find(|(r, _)| r == n) {
        Some((_, p)) => p.clone(),
        None => prefix.to_string(),
    };
    // With `path`, the macro is made under a hidden name and re-exported
    // beside the struct under the struct's: a macro another macro made
    // is reached by that re-export alone, from its own crate, and calls
    // itself through it (`probe_named_path`). Without, under the
    // struct's name, calling itself by that name.
    // The hidden name has the module's path in it, so two structs of
    // one name in two modules export two macros of different names.
    let segs: Vec<&str> = prefix
        .split("::")
        .filter(|x| !x.is_empty() && *x != "$crate")
        .collect();
    let hidden = format!("__named_{}_{name}", segs.join("_"));
    let (mac, call) = if prefix.is_empty() {
        (name.to_string(), name.to_string())
    } else {
        (hidden, format!("{prefix}{name}"))
    };
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
                default: t.default.as_ref().map(|d| macro_text(d)),
            }),
            Param::Const(n, ts, has) => {
                let default = if *has {
                    let e = top_level(ts, '=').expect("a default");
                    Some(format!("{{ {} }}", macro_text(&ts[e + 1..])))
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
            Kind::Type => {
                ("ty", format!("[{}{}<$v>]", marker(&s.name), s.name))
            }
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
                 {call}!(@s {} ; $($($r)*)?) }};",
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
            format!("[{}{}<$($d)*>]", marker(&s.name), s.name)
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
            "(@s {} ;) => {{ {call}!(@s {} ;) }};",
            lhs.join(" "),
            rhs.join(" ")
        ));
    }
    let full: Vec<String> = (0..k).map(|j| format!("[$($s{j}:tt)+]")).collect();
    let args: Vec<String> = (0..k).map(|j| format!("$($s{j})+")).collect();
    rules.push(format!(
        "(@s {} ;) => {{ {prefix}{name}<{}> }};",
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
        "($($a:tt)*) => {{ {call}!(@s {} ; $($a)*) }};",
        start.join(" ")
    ));
    let export = if public { "#[macro_export]\n" } else { "" };
    let doc = format!(
        "#[doc = \"`{name}![P = v, ..]`: [`{name}`] with every parameter \
         given by name, in any order (#1899).\"]\n"
    );
    let rules = rules.join("\n");
    if prefix.is_empty() {
        format!("{doc}{export}macro_rules! {mac} {{\n{rules}\n}}\n")
    } else {
        // A `path` that is not the struct's module fails here, at the
        // struct, rather than at every use.
        let here = prefix.replacen("$crate", "crate", 1);
        format!(
            "#[doc(hidden)]\n{export}macro_rules! {mac} {{\n{rules}\n}}\n\
             {doc}{vis} use {mac} as {name};\n\
             const _: () = {{ #[allow(unused_imports)] use {here}{name} as _; }};\n"
        )
    }
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

/// A default as the use-site macro writes it: `crate` becomes `$crate`,
/// so a default the struct names by its own crate's path, such as
/// `WORDS = crate::dmem::DMEM_WORDS`, resolves where the macro is used,
/// in this crate or another. A bare `DMEM_WORDS` resolves only where it
/// is in scope (#1966).
fn macro_text(ts: &[TokenTree]) -> String {
    dollar_crate(ts).to_string()
}

/// The tokens with each `crate` preceded by a `$`, inside groups too.
fn dollar_crate(ts: &[TokenTree]) -> TokenStream {
    let mut out: Vec<TokenTree> = Vec::new();
    for t in ts {
        match t {
            TokenTree::Ident(i) if i.to_string() == "crate" => {
                out.push(Punct::new('$', Spacing::Joint).into());
                out.push(t.clone());
            }
            TokenTree::Group(g) => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                let mut ng = Group::new(g.delimiter(), dollar_crate(&inner));
                ng.set_span(g.span());
                out.push(ng.into());
            }
            t => out.push(t.clone()),
        }
    }
    out.into_iter().collect()
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
