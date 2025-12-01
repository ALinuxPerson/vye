use crate::crate_;
use convert_case::ccase;
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, quote};
use std::mem;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Block, Field, FieldMutability, FnArg, Generics, ImplItem, ItemImpl, Pat, PatIdent,
    PatType, PathArguments, ReturnType, Token, Type, TypePath, Visibility,
};

fn parse_then_filter<T: FromAttributes>(
    attributes: Vec<Attribute>,
) -> syn::Result<(Vec<Attribute>, T)> {
    let value = T::from_attributes(&attributes)?;
    let attributes = attributes
        .into_iter()
        .filter(|attr| !attr.path().is_ident("vye"))
        .collect();
    Ok((attributes, value))
}

#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct DispatcherArgs {}

struct DispatcherContext {
    crate_: TokenStream,
    attrs: Vec<Attribute>,
    model_ty: Type,
    items: Vec<DispatcherItem>,
}

impl DispatcherContext {
    fn new(value: ItemImpl) -> syn::Result<Self> {
        Ok(Self {
            crate_: crate_(),
            attrs: value.attrs,
            model_ty: *value.self_ty,
            items: value
                .items
                .into_iter()
                .map(DispatcherItem::new)
                .collect::<syn::Result<Vec<_>>>()?,
        })
    }

    fn expand(self) -> syn::Result<TokenStream> {
        let model_ty = &self.model_ty;
        let items = self
            .items
            .into_iter()
            .map(|item| item.expand(&self.crate_, model_ty))
            .collect::<syn::Result<Vec<_>>>()?;

        Ok(quote! {
            #(#items)*
        })
    }
}

enum DispatcherItemKind {
    Updater,
    Getter { data_ty: Box<Type> },
}

#[derive(FromAttributes, Default)]
#[darling(attributes(vye))]
struct DispatcherItemArgs {
    #[darling(default)]
    name: Option<Ident>,
}

struct DispatcherItem {
    args: DispatcherItemArgs,
    kind: DispatcherItemKind,
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,

    // todo: generics support for dispatchers
    generics: Generics,

    inputs: Punctuated<FnArg, Token![,]>,
    block: Block,
}

impl DispatcherItem {
    fn new(value: ImplItem) -> syn::Result<Self> {
        let value = match value {
            ImplItem::Fn(value) => value,
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Only functions are allowed in `#[vye::dispatcher]` blocks",
                ));
            }
        };

        // if &mut self, then updater; if &self, then getter
        let mut kind = None;
        for input in &value.sig.inputs {
            if let FnArg::Receiver(receiver) = input {
                if receiver.mutability.is_some() {
                    kind = Some(DispatcherItemKind::Updater);
                    break;
                } else {
                    let data_ty = match &value.sig.output {
                        ReturnType::Type(_, ty) => ty.clone(),
                        ReturnType::Default => {
                            return Err(syn::Error::new_spanned(
                                value.sig.output,
                                "Getter functions must have a return type",
                            ));
                        }
                    };

                    kind = Some(DispatcherItemKind::Getter { data_ty });
                }
            }
        }
        let kind = kind.ok_or_else(|| {
            syn::Error::new_spanned(
                &value.sig.inputs,
                "Dispatcher functions must have a self parameter",
            )
        })?;
        let (attrs, args) = parse_then_filter(value.attrs)?;

        Ok(Self {
            kind,
            args,
            attrs,
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

#[derive(FromAttributes, Default)]
#[darling(attributes(vye))]
struct FieldArgs {
    #[darling(default)]
    vis: Option<Visibility>,
}

struct DispatcherField {
    args: FieldArgs,
    attrs: Vec<Attribute>,
    name: Ident,
    ty: Type,
}

impl DispatcherField {
    fn new(
        fn_arg: FnArg,
        kind: &DispatcherItemKind,
        ctx_name: &mut Option<Ident>,
    ) -> syn::Result<Option<Self>> {
        match fn_arg {
            // self type, skip
            FnArg::Receiver(_) => Ok(None),
            FnArg::Typed(pat_type) => {
                // `&mut UpdateContext<App>`, skip
                if let DispatcherItemKind::Updater = kind
                    && let Some(ident) = is_update_context(&pat_type)
                {
                    *ctx_name = Some(ident.clone());
                    return Ok(None);
                }

                // todo: more sophisticated error handling for this case
                let Pat::Ident(PatIdent { ident: name, .. }) = *pat_type.pat else {
                    return Ok(None);
                };

                let (attrs, field_args) = parse_then_filter(pat_type.attrs)?;
                Ok(Some(Self {
                    args: field_args,
                    attrs,
                    name,
                    ty: *pat_type.ty,
                }))
            }
        }
    }

    fn to_field(&self) -> Field {
        Field {
            attrs: self.attrs.clone(),
            vis: self.args.vis.clone().unwrap_or(Visibility::Inherited),
            mutability: FieldMutability::None,
            colon_token: Some(Token![:](self.name.span())),
            ident: Some(self.name.clone()),
            ty: self.ty.clone(),
        }
    }
}

impl ToTokens for DispatcherField {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.to_field().to_tokens(tokens)
    }
}

impl DispatcherItem {
    fn make_fields(&mut self, ctx_name: &mut Option<Ident>) -> syn::Result<Vec<DispatcherField>> {
        mem::take(&mut self.inputs)
            .into_iter()
            .filter_map(|fn_arg| DispatcherField::new(fn_arg, &self.kind, ctx_name).transpose())
            .collect::<syn::Result<Vec<_>>>()
    }

    fn expand(mut self, crate_: &TokenStream, model_ty: &Type) -> syn::Result<TokenStream> {
        let mut ctx_name = None;
        let fields = self.make_fields(&mut ctx_name)?;
        let field_names = fields.iter().map(|f| &f.name).collect::<Vec<_>>();
        let name = self.args.name.unwrap_or_else(|| {
            Ident::new(&ccase!(pascal, self.name.to_string()), Span::call_site())
        });
        let attrs = &self.attrs;
        let vis = &self.vis;
        let block = &self.block;
        let struct_decl = quote! {
            #(#attrs)*
            #vis struct #name {
                #(#fields),*
            }
        };
        match self.kind {
            DispatcherItemKind::Updater => Ok(quote! {
                #struct_decl
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
            }),
            DispatcherItemKind::Getter { data_ty } => Ok(quote! {
                #struct_decl
                impl #crate_::ModelGetterMessage for #name {
                    type Data = #data_ty;
                }
                impl #crate_::ModelGetterHandler<#name> for #model_ty {
                    fn getter(&self, #name { #(#field_names),* }: #name) -> #data_ty {
                        #block
                    }
                }
            }),
        }
    }
}

pub fn build(value: ItemImpl) -> syn::Result<TokenStream> {
    DispatcherContext::new(value)?.expand()
}
