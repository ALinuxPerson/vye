mod dispatcher;
mod command;
mod utils;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span};
use quote::quote;

fn crate_() -> proc_macro2::TokenStream {
    match crate_name("vye").expect("`vye` crate should be present in `Cargo.toml`") {
        FoundCrate::Itself => quote! { crate },
        FoundCrate::Name(name) => {
            let ident = Ident::new(&name, Span::call_site());
            quote! { #ident }
        }
    }
}

#[proc_macro_attribute]
pub fn dispatcher(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as dispatcher::DispatcherArgs);
    let input = syn::parse_macro_input!(input as utils::InterfaceImpl);
    match dispatcher::build(input, args) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn command(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as command::CommandArgs);
    let input = syn::parse_macro_input!(input as syn::ItemFn);
    match command::build(input, args) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
