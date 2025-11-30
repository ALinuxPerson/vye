use proc_macro::TokenStream;

mod wrap_dispatcher;

#[proc_macro]
pub fn wrap_dispatcher(input: TokenStream) -> TokenStream {
    let def = syn::parse_macro_input!(input as wrap_dispatcher::DispatcherDef);
    match def.expand() {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
