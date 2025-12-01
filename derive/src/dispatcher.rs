use crate::crate_;
use convert_case::ccase;
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, quote};
use std::ops::ControlFlow;
use syn::{
    Attribute, Block, Field, FieldMutability, FnArg, Generics, ImplItem, ItemImpl, Pat, PatIdent,
    PatType, PathArguments, ReturnType, Signature, Token, Type, TypePath, Visibility,
};

fn parse_then_filter<T: FromAttributes>(
    attributes: &[Attribute],
) -> syn::Result<(Vec<&Attribute>, T)> {
    let value = T::from_attributes(&attributes)?;
    let attributes = attributes
        .iter()
        .filter(|attr| !attr.path().is_ident("vye"))
        .collect();
    Ok((attributes, value))
}

#[derive(FromMeta)]
struct GenerateDispatcherArgs {
    #[darling(default)]
    dispatcher: Option<Ident>,

    #[darling(default)]
    vis: Option<Visibility>,

    #[darling(multiple, rename = "attr")]
    attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "updater_attr")]
    updater_attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "getter_attr")]
    getter_attrs: Vec<syn::Meta>,
}

impl GenerateDispatcherArgs {
    pub fn dispatcher(&self, model_name: &Ident) -> Ident {
        self.dispatcher
            .clone()
            .unwrap_or_else(|| Ident::new(&format!("{model_name}Dispatcher"), Span::call_site()))
    }
}

#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct DispatcherArgs {
    generate: Option<GenerateDispatcherArgs>,
}

struct DispatcherContext<'a> {
    args: DispatcherArgs,
    new_fn_vis: &'a Visibility,
    new_fn_attrs: &'a [Attribute],
    has_split_fn: bool,
    split_fn_vis: &'a Visibility,
    split_fn_attrs: &'a [Attribute],
    updater_name: Option<Ident>,
    getter_name: Option<Ident>,
    crate_: TokenStream,
    model_ty: &'a Type,
    model_name: Ident,
    items: Vec<DispatcherItem<'a>>,
}

impl<'a> DispatcherContext<'a> {
    fn new(value: &'a ItemImpl, args: DispatcherArgs) -> syn::Result<Self> {
        let model_name = match &*value.self_ty {
            Type::Path(TypePath { path, .. }) => path
                .segments
                .last()
                .ok_or_else(|| {
                    syn::Error::new_spanned(&value.self_ty, "Provided type path has no segments")
                })?
                .ident
                .clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    &value.self_ty,
                    "Expected a type path for the model type",
                ));
            }
        };
        let mut new_fn_vis = &Visibility::Inherited;
        let mut new_fn_attrs = &[][..];
        let mut has_split_fn = false;
        let mut split_fn_vis = &Visibility::Inherited;
        let mut split_fn_attrs = &[][..];
        let mut updater_name = None;
        let mut getter_name = None;
        let items = value
            .items
            .iter()
            .filter_map(|value| match DispatcherItem::new(value) {
                Ok(MaybeDispatcherItem::Is(item)) => Some(Ok(item)),
                Ok(MaybeDispatcherItem::New { vis, attrs }) => {
                    new_fn_vis = vis;
                    new_fn_attrs = attrs;
                    None
                }
                Ok(MaybeDispatcherItem::Split {
                    vis,
                    attrs,
                    updater,
                    getter,
                }) => {
                    has_split_fn = true;
                    split_fn_vis = vis;
                    split_fn_attrs = attrs;
                    updater_name = updater;
                    getter_name = getter;
                    None
                }
                Err(error) => Some(Err(error)),
            })
            .collect::<syn::Result<_>>()?;
        Ok(Self {
            args,
            new_fn_vis,
            new_fn_attrs,
            has_split_fn,
            split_fn_vis,
            split_fn_attrs,
            updater_name,
            getter_name,
            crate_: crate_(),
            model_ty: &value.self_ty,
            model_name,
            items,
        })
    }

    fn generate(&self) -> syn::Result<TokenStream> {
        let model_ty = &self.model_ty;
        let items = self
            .items
            .iter()
            .map(|item| item.generate(&self.crate_, model_ty))
            .collect::<syn::Result<Vec<_>>>()?;
        let items = quote! { #(#items)* };

        if let Some(args) = &self.args.generate {
            let wrapped_dispatcher = self.generate_wrapped_dispatcher(args);

            Ok(quote! {
                #items
                #wrapped_dispatcher
            })
        } else {
            Ok(items)
        }
    }
}

impl<'a> DispatcherContext<'a> {
    fn updater_getter_idents(&self) -> (Ident, Ident) {
        let updater = self.updater_name.clone().unwrap_or_else(|| {
            Ident::new(&format!("{}Updater", self.model_name), Span::call_site())
        });
        let getter = self.getter_name.clone().unwrap_or_else(|| {
            Ident::new(&format!("{}Getter", self.model_name), Span::call_site())
        });
        (updater, getter)
    }

    fn generate_dispatcher_fns(&self) -> (Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>) {
        self.items
            .iter()
            .map(|item| item.generate_dispatcher_fns())
            .fold(
                (Vec::new(), Vec::new(), Vec::new()),
                |(mut dispatcher_fns, mut updater_fns, mut getter_fns), fns| {
                    dispatcher_fns.push(fns.dispatcher_fn);
                    match fns.wrapper_fn {
                        GeneratedDispatcherWrapperFn::Updater(fn_) => updater_fns.push(fn_),
                        GeneratedDispatcherWrapperFn::Getter(fn_) => getter_fns.push(fn_),
                    }
                    (dispatcher_fns, updater_fns, getter_fns)
                },
            )
    }

    fn generate_wrapped_dispatcher(&self, args: &GenerateDispatcherArgs) -> TokenStream {
        let crate_ = &self.crate_;
        let dispatcher_name = args.dispatcher(&self.model_name);
        let vis = args.vis.as_ref().unwrap_or(&Visibility::Inherited);
        let attrs = &args.attrs;
        let new_fn_vis = self.new_fn_vis;
        let new_fn_attrs = self.new_fn_attrs;
        let model_ty = self.model_ty;
        let mut ret = quote! {
            #(#attrs)*
            #vis struct #dispatcher_name(#crate_::Dispatcher<#model_ty>);
            impl #dispatcher_name {
                #(#new_fn_attrs)*
                #new_fn_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedDispatcher::__new(dispatcher, #crate_::dispatcher::__private::Token::new())
                }
            }
            impl #crate_::WrappedDispatcher for #dispatcher_name {
                type Model = #model_ty;

                fn __new(dispatcher: #crate_::Dispatcher<#model_ty>, _: #crate_::dispatcher::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::dispatcher::__private::Sealed for #dispatcher_name {}
        };
        let (dispatcher_fns, updater_fns, getter_fns) = self.generate_dispatcher_fns();
        ret.extend(quote! {
            impl #dispatcher_name { #(#dispatcher_fns)* }
        });
        if self.has_split_fn {
            ret.extend(self.generate_updater_getter(
                args,
                &dispatcher_name,
                updater_fns,
                getter_fns,
            ))
        }
        ret
    }

    fn generate_updater_getter(
        &self,
        args: &GenerateDispatcherArgs,
        dispatcher_name: &Ident,
        updater_fns: Vec<TokenStream>,
        getter_fns: Vec<TokenStream>,
    ) -> TokenStream {
        let crate_ = &self.crate_;
        let updater_attrs = &args.updater_attrs;
        let getter_attrs = &args.getter_attrs;
        let (updater_name, getter_name) = self.updater_getter_idents();
        let new_fn_vis = self.new_fn_vis;
        let split_fn_vis = self.split_fn_vis;
        let split_fn_attrs = self.split_fn_attrs;
        let model_ty = self.model_ty;
        quote! {
            impl #dispatcher_name {
                #(#split_fn_attrs)*
                #split_fn_vis fn split(self) -> (#updater_name, #getter_name) {
                    #crate_::SplittableWrappedDispatcher::__split(self, #crate_::dispatcher::__private::Token::new())
                }
            }
            impl #crate_::SplittableWrappedDispatcher for #dispatcher_name {
                type Updater = #updater_name;
                type Getter = #getter_name;
            }

            #(#updater_attrs)*
            #split_fn_vis struct #updater_name(#dispatcher_name);
            impl #crate_::WrappedUpdater for #updater_name {
                type WrappedDispatcher = #dispatcher_name;
                fn __new(dispatcher: #dispatcher_name, _: #crate_::dispatcher::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::dispatcher::__private::Sealed for #updater_name {}
            impl #updater_name {
                // todo: add ability to add attributes to new
                #new_fn_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedUpdater::__new(#dispatcher_name::new(dispatcher), #crate_::dispatcher::__private::Token::new())
                }
                #(#updater_fns)*
            }

            #(#getter_attrs)*
            #split_fn_vis struct #getter_name(#dispatcher_name);
            impl #crate_::WrappedGetter for #getter_name {
                type WrappedDispatcher = #dispatcher_name;
                fn __new(dispatcher: #dispatcher_name, _: #crate_::dispatcher::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::dispatcher::__private::Sealed for #getter_name {}
            impl #getter_name {
                // todo: add ability to add attributes to new
                #new_fn_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedGetter::__new(#dispatcher_name::new(dispatcher), #crate_::dispatcher::__private::Token::new())
                }
                #(#getter_fns)*
            }
        }
    }
}

enum DispatcherItemKind {
    Updater { ctx_name: Option<Ident> },
    Getter { data_ty: Box<Type> },
}

#[derive(FromAttributes, Default)]
#[darling(attributes(vye))]
struct DispatcherItemArgs {
    #[darling(default)]
    name: Option<Ident>,

    #[darling(default)]
    dispatcher: Option<Visibility>,
}

enum MaybeDispatcherItem<'a> {
    Is(DispatcherItem<'a>),
    New {
        vis: &'a Visibility,
        attrs: &'a [Attribute],
    },
    Split {
        vis: &'a Visibility,
        attrs: &'a [Attribute],
        updater: Option<Ident>,
        getter: Option<Ident>,
    },
}

struct DispatcherItem<'a> {
    args: DispatcherItemArgs,
    kind: DispatcherItemKind,
    attrs: Vec<&'a Attribute>,
    vis: &'a Visibility,
    name: &'a Ident,
    generics: &'a Generics,
    fields: Vec<DispatcherField<'a>>,
    block: &'a Block,
}

// if &mut self, then updater; if &self, then getter
fn find_kind(sig: &Signature) -> syn::Result<Option<DispatcherItemKind>> {
    let mut kind = None;
    for input in &sig.inputs {
        if let FnArg::Receiver(receiver) = input {
            if receiver.mutability.is_some() {
                kind = Some(DispatcherItemKind::Updater { ctx_name: None });
                break;
            } else {
                let data_ty = match &sig.output {
                    ReturnType::Type(_, ty) => ty.clone(),
                    ReturnType::Default => {
                        return Err(syn::Error::new_spanned(
                            &sig.output,
                            "Getter functions must have a return type",
                        ));
                    }
                };

                kind = Some(DispatcherItemKind::Getter { data_ty });
            }
        }
    }
    Ok(kind)
}

fn assert_signature_shape(sig: &Signature) -> syn::Result<()> {
    if sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Dispatcher functions cannot be const",
        ));
    }

    if sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Dispatcher functions cannot be async",
        ));
    }

    if sig.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Dispatcher functions cannot be unsafe",
        ));
    }

    if sig.abi.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Dispatcher functions cannot have a custom ABI",
        ));
    }

    if sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Dispatcher functions cannot be variadic",
        ));
    }

    Ok(())
}

fn find_new_or_split<'a>(
    vis: &'a Visibility,
    attrs: &'a [Attribute],
    sig: &Signature,
    block: &Block,
) -> syn::Result<Option<MaybeDispatcherItem<'a>>> {
    // must not have any function arguments nor does its body contain anything i.e. fn new() {}
    if !sig.inputs.is_empty() || !block.stmts.is_empty() {
        return Ok(None);
    }

    if sig.ident == "new" && matches!(sig.output, ReturnType::Default) {
        Ok(Some(MaybeDispatcherItem::New { vis, attrs }))
    } else if sig.ident == "split"
        && let ReturnType::Type(_, ty) = &sig.output
        && let Type::Tuple(tuple) = &**ty
        && tuple.elems.len() == 2
    {
        let get_ident = |ty: &Type| match ty {
            Type::Path(ty) => match ty.path.segments.last() {
                Some(segment) => ControlFlow::Continue(Some(segment.ident.clone())),
                None => ControlFlow::Break(Ok(None)),
            },
            Type::Infer(_) => ControlFlow::Continue(None),
            _ => ControlFlow::Break(Ok(None)),
        };
        let updater = match get_ident(&tuple.elems[0]) {
            ControlFlow::Continue(ident) => ident,
            ControlFlow::Break(other) => return other,
        };
        let getter = match get_ident(&tuple.elems[1]) {
            ControlFlow::Continue(ident) => ident,
            ControlFlow::Break(other) => return other,
        };
        Ok(Some(MaybeDispatcherItem::Split {
            vis,
            attrs,
            updater,
            getter,
        }))
    } else {
        Ok(None)
    }
}

impl<'a> DispatcherItem<'a> {
    #![allow(clippy::new_ret_no_self)]
    fn new(value: &'a ImplItem) -> syn::Result<MaybeDispatcherItem<'a>> {
        let value = match value {
            ImplItem::Fn(value) => value,
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    format!(
                        "Only functions are allowed in `#[vye::dispatcher]` blocks, got {other:?}"
                    ),
                ));
            }
        };

        assert_signature_shape(&value.sig)?;
        let (attrs, args) = parse_then_filter(&value.attrs)?;
        let kind = match find_kind(&value.sig)? {
            Some(kind) => kind,
            None => {
                return match find_new_or_split(&value.vis, &value.attrs, &value.sig, &value.block)?
                {
                    Some(value) => Ok(value),
                    None => Err(syn::Error::new_spanned(
                        &value.sig.inputs,
                        "Dispatcher functions must have a self parameter",
                    )),
                };
            }
        };
        let mut ctx_name = None;
        let fields = value
            .sig
            .inputs
            .iter()
            .filter_map(|fn_arg| {
                DispatcherField::new(
                    fn_arg,
                    matches!(kind, DispatcherItemKind::Updater { .. }),
                    &mut ctx_name,
                )
                .transpose()
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let kind = match kind {
            DispatcherItemKind::Updater { .. } => DispatcherItemKind::Updater { ctx_name },
            DispatcherItemKind::Getter { data_ty } => DispatcherItemKind::Getter { data_ty },
        };

        Ok(MaybeDispatcherItem::Is(Self {
            kind,
            args,
            attrs,
            vis: &value.vis,
            name: &value.sig.ident,
            generics: &value.sig.generics,
            fields,
            block: &value.block,
        }))
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

struct DispatcherField<'a> {
    args: FieldArgs,
    attrs: Vec<&'a Attribute>,
    name: &'a Ident,
    ty: &'a Type,
}

impl<'a> DispatcherField<'a> {
    fn new(
        fn_arg: &'a FnArg,
        is_updater: bool,
        ctx_name: &mut Option<Ident>,
    ) -> syn::Result<Option<Self>> {
        match fn_arg {
            // self type, skip
            FnArg::Receiver(_) => Ok(None),
            FnArg::Typed(pat_type) => {
                // `&mut UpdateContext<App>`, skip
                if is_updater && let Some(ident) = is_update_context(pat_type) {
                    *ctx_name = Some(ident.clone());
                    return Ok(None);
                }

                // todo: more sophisticated error handling for this case
                let Pat::Ident(PatIdent { ident: name, .. }) = &*pat_type.pat else {
                    return Ok(None);
                };

                let (attrs, field_args) = parse_then_filter(&pat_type.attrs)?;
                Ok(Some(Self {
                    args: field_args,
                    attrs,
                    name,
                    ty: &pat_type.ty,
                }))
            }
        }
    }

    fn to_field(&self) -> Field {
        Field {
            attrs: self.attrs.iter().copied().cloned().collect(),
            vis: self.args.vis.clone().unwrap_or(Visibility::Inherited),
            mutability: FieldMutability::None,
            colon_token: Some(Token![:](self.name.span())),
            ident: Some(self.name.clone()),
            ty: self.ty.clone(),
        }
    }
}

impl<'a> ToTokens for DispatcherField<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.to_field().to_tokens(tokens)
    }
}

impl<'a> DispatcherItem<'a> {
    fn name(&self) -> Ident {
        self.args.name.clone().unwrap_or_else(|| {
            Ident::new(&ccase!(pascal, self.name.to_string()), Span::call_site())
        })
    }
}

impl<'a> DispatcherItem<'a> {
    fn generate(&self, crate_: &TokenStream, model_ty: &Type) -> syn::Result<TokenStream> {
        let fields = &self.fields;
        let field_names = fields.iter().map(|f| &f.name).collect::<Vec<_>>();
        let name = self.name();
        let attrs = &self.attrs;
        let vis = &self.vis;
        let block = &self.block;
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let struct_decl = quote! {
            #(#attrs)*
            #vis struct #name #impl_generics #where_clause {
                #(#fields),*
            }
        };
        match &self.kind {
            DispatcherItemKind::Updater { ctx_name } => {
                let ctx_name = ctx_name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| Token![_](Span::call_site()).into());
                Ok(quote! {
                    #struct_decl
                    impl #impl_generics #crate_::ModelMessage for #name #ty_generics #where_clause {}
                    impl #impl_generics #crate_::ModelHandler<#name> for #model_ty #ty_generics #where_clause {
                        fn update(
                            &mut self,
                            #name { #(#field_names),* }: #name #ty_generics,
                            #ctx_name: &mut #crate_::UpdateContext<<#model_ty as #crate_::Model>::ForApp>,
                        ) {
                            #block
                        }
                    }
                })
            }
            DispatcherItemKind::Getter { data_ty } => Ok(quote! {
                #struct_decl
                impl #impl_generics #crate_::ModelGetterMessage for #name #ty_generics #where_clause {
                    type Data = #data_ty;
                }
                impl #impl_generics #crate_::ModelGetterHandler<#name> for #model_ty #where_clause {
                    fn getter(&self, #name { #(#field_names),* }: #name #ty_generics) -> #data_ty {
                        #block
                    }
                }
            }),
        }
    }
}

impl<'a> DispatcherItem<'a> {
    fn generate_dispatcher_fns(&self) -> GeneratedDispatcherFns {
        let vis = self
            .args
            .dispatcher
            .clone()
            .unwrap_or(Visibility::Inherited);
        let fields = &self.fields;
        let field_names = self.fields.iter().map(|f| f.name).collect::<Vec<_>>();
        let fn_name = self.name;
        let name = self.name();
        // todo: add the ability to customize the closure body, maybe with a `with = "function`?
        let closure_body = quote! {
            #name { #(#field_names),* }
        };
        let (return_ty, method_call) = match &self.kind {
            DispatcherItemKind::Updater { .. } => (TokenStream::new(), quote! { send }),
            DispatcherItemKind::Getter { data_ty, .. } => (quote! { -> #data_ty }, quote! { get }),
        };
        let dispatcher_fn = quote! {
            // todo: add the ability to add attributes to dispatcher functions
            #vis async fn #fn_name(&mut self, #(#fields),*) #return_ty {
                let f: fn(#(#fields),*) -> #name = |#(#field_names),*| #closure_body;
                self.0.#method_call(f(#(#field_names),*)).await
            }
        };
        let wrapper_fn = quote! {
            #vis async fn #fn_name(&mut self, #(#fields),*) #return_ty {
                self.0.#fn_name(#(#field_names),*).await
            }
        };
        let wrapper_fn = match &self.kind {
            DispatcherItemKind::Updater { .. } => GeneratedDispatcherWrapperFn::Updater(wrapper_fn),
            DispatcherItemKind::Getter { .. } => GeneratedDispatcherWrapperFn::Getter(wrapper_fn),
        };
        GeneratedDispatcherFns {
            dispatcher_fn,
            wrapper_fn,
        }
    }
}

struct GeneratedDispatcherFns {
    dispatcher_fn: TokenStream,
    wrapper_fn: GeneratedDispatcherWrapperFn,
}

enum GeneratedDispatcherWrapperFn {
    Updater(TokenStream),
    Getter(TokenStream),
}

pub fn build(value: ItemImpl, args: DispatcherArgs) -> syn::Result<TokenStream> {
    DispatcherContext::new(&value, args)?.generate()
}
