use crate::crate_;
use convert_case::ccase;
use darling::FromMeta;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Block, Field, FieldMutability, FnArg, Generics, ImplItem, ItemImpl,
    Pat, PatIdent, PatType, PathArguments, ReturnType, Token, Type,
    TypePath, Visibility,
};

#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct UpdaterArgs {}

struct UpdaterContext {
    crate_: TokenStream,
    attrs: Vec<Attribute>,
    model_ty: Type,
    items: Vec<UpdaterItem>,
}

impl UpdaterContext {
    fn new(value: ItemImpl) -> syn::Result<Self> {
        Ok(Self {
            crate_: crate_(),
            attrs: value.attrs,
            model_ty: *value.self_ty,
            items: value
                .items
                .into_iter()
                .map(UpdaterItem::new)
                .collect::<syn::Result<Vec<_>>>()?,
        })
    }

    fn expand(self) -> syn::Result<TokenStream> {
        let model_ty = &self.model_ty;
        let updaters = self
            .items
            .into_iter()
            .map(|item| item.expand(&self.crate_, model_ty))
            .collect::<syn::Result<Vec<_>>>()?;

        Ok(quote! {
            #(#updaters)*
        })
    }
}

struct UpdaterItem {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,

    // todo: generics support for updaters
    generics: Generics,

    inputs: Punctuated<FnArg, Token![,]>,
    block: Block,
}

impl UpdaterItem {
    fn new(value: ImplItem) -> syn::Result<Self> {
        let value = match value {
            ImplItem::Fn(value) => value,
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Only functions are allowed in `#[vye::updater]` blocks",
                ));
            }
        };

        if !matches!(value.sig.output, ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                value.sig.output,
                "Updater functions must not have a return type",
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

impl UpdaterItem {
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
        let vis = &self.vis;
        let block = &self.block;
        Ok(quote! {
            #(#attrs)*
            #vis struct #name {
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
    UpdaterContext::new(value)?.expand()
}
