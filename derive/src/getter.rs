use crate::crate_;
use convert_case::ccase;
use darling::FromMeta;
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Block, Field, FieldMutability, FnArg, Generics, ImplItem, ItemImpl, Pat, PatIdent,
    PatType, PathArguments, ReturnType, Token, Type, TypePath, Visibility,
};

#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct GetterArgs {}

struct GetterContext {
    crate_: TokenStream,
    model_ty: Type,
    items: Vec<GetterItem>,
}

impl GetterContext {
    fn new(value: ItemImpl) -> syn::Result<Self> {
        Ok(Self {
            crate_: crate_(),
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
    data_ty: Type,
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

        let data_ty = match value.sig.output {
            ReturnType::Default => {
                return Err(syn::Error::new_spanned(
                    value.sig.output,
                    "Getter functions must have a return type",
                ));
            }
            ReturnType::Type(_, ty) => *ty,
        };

        Ok(Self {
            attrs: value.attrs,
            vis: value.vis,
            name: value.sig.ident,
            generics: value.sig.generics,
            inputs: value.sig.inputs,
            data_ty,
            block: value.block,
        })
    }
}

impl GetterItem {
    fn expand(self, crate_: &TokenStream, model_ty: &Type) -> syn::Result<TokenStream> {
        let fields = self
            .inputs
            .into_iter()
            .filter_map(|fn_arg| match fn_arg {
                // self type, skip
                FnArg::Receiver(_) => None,
                FnArg::Typed(pat_type) => {
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

        let field_names = fields
            .iter()
            .map(|f| f.ident.as_ref().expect("expected ident for field to exist"))
            .collect::<Vec<_>>();
        let name = Ident::new(&ccase!(pascal, self.name.to_string()), Span::call_site());
        let attrs = &self.attrs;
        let vis = &self.vis;
        let data_ty = &self.data_ty;
        let block = &self.block;
        Ok(quote! {
            #(#attrs)*
            #vis struct #name {
                // todo: add ability to specify visibility and attributes of fields
                #(#fields),*
            }
            impl #crate_::ModelGetterMessage for #name {
                type Data = #data_ty;
            }
            impl #crate_::ModelGetterHandler<#name> for #model_ty {
                fn getter(&self, #name { #(#field_names),* }: #name) -> #data_ty {
                    #block
                }
            }
        })
    }
}

pub fn build(value: ItemImpl) -> syn::Result<TokenStream> {
    GetterContext::new(value)?.expand()
}
