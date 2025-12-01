use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span};
use quote::quote;
use std::sync::LazyLock;

mod wrap_dispatcher;
mod updater {
    use crate::crate_;
    use convert_case::ccase;
    use darling::FromMeta;
    use proc_macro2::{Ident, Span, TokenStream};
    use quote::quote;
    use syn::punctuated::Punctuated;
    use syn::spanned::Spanned;
    use syn::{
        Attribute, Block, Field, FieldMutability, FieldsNamed, FnArg, Generics, ImplItem, ItemImpl,
        Pat, PatIdent, PatType, PathArguments, Receiver, ReturnType, Signature, Token, Type,
        TypePath, Visibility,
    };

    #[derive(FromMeta)]
    #[darling(derive_syn_parse)]
    pub struct UpdaterArgs {}

    struct GetterContext {
        crate_: TokenStream,
        attrs: Vec<Attribute>,
        model_ty: Type,
        items: Vec<GetterItem>,
    }

    impl GetterContext {
        fn new(value: ItemImpl) -> syn::Result<Self> {
            Ok(Self {
                crate_: crate_(),
                attrs: value.attrs,
                model_ty: *value.self_ty,
                items: value
                    .items
                    .into_iter()
                    .map(GetterItem::new)
                    .collect::<syn::Result<Vec<_>>>()?,
            })
        }

        fn expand(self) -> syn::Result<TokenStream> {
            let model_ty = &self.model_ty;
            let getters = self
                .items
                .into_iter()
                .map(|item| item.expand(&self.crate_, model_ty))
                .collect::<syn::Result<Vec<_>>>()?;

            Ok(quote! {
                #(#getters)*
            })
        }
    }

    struct GetterItem {
        attrs: Vec<Attribute>,
        vis: Visibility,
        name: Ident,

        // todo: generics support for getters
        generics: Generics,

        inputs: Punctuated<FnArg, Token![,]>,
        block: Block,
    }

    impl GetterItem {
        fn new(value: ImplItem) -> syn::Result<Self> {
            let value = match value {
                ImplItem::Fn(value) => value,
                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        "Only functions are allowed in `#[vye::getter]` blocks",
                    ));
                }
            };

            if !matches!(value.sig.output, ReturnType::Default) {
                return Err(syn::Error::new_spanned(
                    value.sig.output,
                    "Getter functions must not have a return type",
                ));
            }

            Ok(Self {
                attrs: value.attrs,
                vis: value.vis,
                name: value.sig.ident,
                generics: value.sig.generics,
                inputs: value.sig.inputs,
                block: value.block,
            })
        }
    }

    // must be &mut UpdateContext<App>
    fn is_update_context(pat_ty: &PatType) -> Option<&Ident> {
        if let Pat::Ident(PatIdent { ident, .. }) = &*pat_ty.pat
            && let Type::Reference(ty) = &*pat_ty.ty
            && ty.mutability.is_some()
            && let Type::Path(TypePath { path, .. }) = &*ty.elem
            && let Some(segment) = path.segments.last()
            && matches!(segment.arguments, PathArguments::AngleBracketed(_))
            && segment.ident == "UpdateContext"
        {
            Some(ident)
        } else {
            None
        }
    }

    impl GetterItem {
        fn expand(self, crate_: &TokenStream, model_ty: &Type) -> syn::Result<TokenStream> {
            let mut ctx_name = None;
            let fields = self
                .inputs
                .into_iter()
                .filter_map(|fn_arg| match fn_arg {
                    // self type, skip
                    FnArg::Receiver(_) => None,
                    FnArg::Typed(pat_type) => {
                        // `&mut UpdateContext<App>`, skip
                        if let Some(ident) = is_update_context(&pat_type) {
                            ctx_name = Some(ident.clone());
                            return None;
                        }

                        // todo: more sophisticated error handling for this case
                        let Pat::Ident(ident) = *pat_type.pat else {
                            return None;
                        };
                        let ident_span = ident.span();
                        Some(Field {
                            attrs: pat_type.attrs,

                            // todo: make visibility configurable via proc macro attribute
                            vis: Visibility::Inherited,

                            mutability: FieldMutability::None,
                            ident: Some(ident.ident),
                            colon_token: Some(Token![:](ident_span)),
                            ty: *pat_type.ty,
                        })
                    }
                })
                .collect::<Vec<Field>>();

            let ctx_name = ctx_name.unwrap_or_else(|| Token![_](Span::call_site()).into());
            let field_names = fields
                .iter()
                .map(|f| f.ident.as_ref().expect("expected ident for field to exist"))
                .collect::<Vec<_>>();
            let name = Ident::new(&ccase!(pascal, self.name.to_string()), Span::call_site());
            let attrs = &self.attrs;
            let block = &self.block;
            Ok(quote! {
                // todo: add ability to specify visibility of #name
                #(#attrs)*
                struct #name {
                    // todo: add ability to specify visibility and attributes of fields
                    #(#fields),*
                }
                impl #crate_::ModelMessage for #name {}
                impl #crate_::ModelHandler<#name> for #model_ty {
                    fn update(
                        &mut self,
                        #name { #(#field_names),* }: #name,
                        #ctx_name: &mut #crate_::UpdateContext<<#model_ty as #crate_::Model>::ForApp>,
                    ) {
                        #block
                    }
                }
            })
        }
    }

    pub fn build(value: ItemImpl) -> syn::Result<TokenStream> {
        GetterContext::new(value)?.expand()
    }
}
mod getter {
    use darling::FromMeta;
    use proc_macro2::TokenStream;
    use syn::ItemImpl;

    #[derive(FromMeta)]
    #[darling(derive_syn_parse)]
    pub struct GetterArgs {}

    pub fn build(value: ItemImpl) -> syn::Result<TokenStream> {
        todo!()
    }
}
mod message {
    use darling::FromMeta;
    use proc_macro2::Ident;

    #[derive(FromMeta)]
    #[darling(derive_syn_parse)]
    pub struct MessageArgs {
        name: Option<Ident>,
    }
}

thread_local! {
    static CRATE: proc_macro2::TokenStream = match crate_name("vye").expect("`vye` crate should be present in `Cargo.toml`") {
        FoundCrate::Itself => quote! { crate },
        FoundCrate::Name(name) => {
            let ident = Ident::new(&name, Span::call_site());
            quote! { #ident }
        }
    };
}

fn crate_() -> proc_macro2::TokenStream {
    CRATE.with(|c| c.clone())
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
pub fn updater(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as updater::UpdaterArgs);
    let input = syn::parse_macro_input!(input as syn::ItemImpl);
    match updater::build(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn getter(args: TokenStream, input: TokenStream) -> TokenStream {
    let _args = syn::parse_macro_input!(args as getter::GetterArgs);
    let input = syn::parse_macro_input!(input as syn::ItemImpl);
    match getter::build(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn message(args: TokenStream, input: TokenStream) -> TokenStream {
    let _args = syn::parse_macro_input!(args as message::MessageArgs);
    input
}
