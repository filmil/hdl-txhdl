// A derive for the marker traits, written without syn or quote so that it
// needs no crate registry. It only has to find the type's name, because a
// marker trait has no items.
extern crate proc_macro;
use proc_macro::{TokenStream, TokenTree};

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
    marker(input, "crate::types::Transaction")
}

#[proc_macro_derive(Bus)]
pub fn derive_bus(input: TokenStream) -> TokenStream {
    marker(input, "crate::comp::Bus")
}
