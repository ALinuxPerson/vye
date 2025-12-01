mod wrap_dispatcher;
mod message {
    use darling::FromMeta;
    use proc_macro2::Ident;

    #[derive(FromMeta)]
    #[darling(derive_syn_parse)]
    pub struct MessageArgs {
        name: Option<Ident>,
    }
}
mod dispatcher;

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

#[proc_macro]
pub fn wrap_dispatcher(input: TokenStream) -> TokenStream {
    let def = syn::parse_macro_input!(input as wrap_dispatcher::DispatcherDef);
    match def.expand() {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn dispatcher(args: TokenStream, input: TokenStream) -> TokenStream {
    let _args = syn::parse_macro_input!(args as dispatcher::DispatcherArgs);
    let input = syn::parse_macro_input!(input as syn::ItemImpl);
    match dispatcher::build(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn message(args: TokenStream, input: TokenStream) -> TokenStream {
    let _args = syn::parse_macro_input!(args as message::MessageArgs);
    input
}
