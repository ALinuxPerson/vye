use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, FnArg, Pat, ReturnType, Signature, Token, Type, Visibility, braced, token,
};

fn extract_args(inputs: &Punctuated<FnArg, Token![,]>) -> Punctuated<Ident, Token![,]> {
    let mut args = Punctuated::new();
    for input in inputs {
        if let FnArg::Typed(pat_type) = input
            && let Pat::Ident(pat_ident) = &*pat_type.pat
        {
            args.push(pat_ident.ident.clone());
        }
    }
    args
}

pub struct DispatcherDef {
    ty_attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    model_ty: Type,
    updater_getter_defs: Option<(WrapperDef, WrapperDef)>,
    methods: Vec<MethodDef>,
    crate_: TokenStream,
}

impl DispatcherDef {
    pub fn expand(&self) -> syn::Result<TokenStream> {
        let name = &self.name;
        let model_ty = &self.model_ty;
        let vis = &self.vis;
        let ty_attrs = &self.ty_attrs;
        let crate_ = &self.crate_;
        let dispatcher_struct = quote! {
            #(#ty_attrs)*
            #vis struct #name(#crate_::Dispatcher<#model_ty>);
        };
        let (dispatcher_methods, updater_methods, getter_methods) = self.generate_methods()?;
        let impl_dispatcher = quote! {
            impl #name {
                #(#dispatcher_methods)*
            }
        };
        let impl_wrapped_dispatcher_for_ty = quote! {
            impl #crate_::WrappedDispatcher for #name {
                type Model = #model_ty;
                
                fn __new(dispatcher: #crate_::Dispatcher<Self::Model>, _: #crate_::dispatcher::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::dispatcher::__private::Sealed for #name {}
        };
        let wrappers = self
            .updater_getter_defs
            .as_ref()
            .map(|(updater_def, getter_def)| {
                let updater_name = &updater_def.name;
                let updater_attrs = &updater_def.ty_attrs;
                let getter_name = &getter_def.name;
                let getter_attrs = &getter_def.ty_attrs;

                quote! {
                    impl #crate_::SplittableWrappedDispatcher for #name {
                        type Updater = #updater_name;
                        type Getter = #getter_name;
                    }
                    
                    #(#updater_attrs)*
                    #vis struct #updater_name(#name);

                    impl #updater_name {
                        #(#updater_methods)*
                    }
                    
                    impl #crate_::WrappedUpdater for #updater_name {
                        type WrappedDispatcher = #name;
                        
                        fn __new(dispatcher: Self::WrappedDispatcher, _: #crate_::dispatcher::__private::Token) -> Self {
                            Self(dispatcher)
                        }
                    }
                    
                    impl #crate_::dispatcher::__private::Sealed for #updater_name {}

                    #(#getter_attrs)*
                    #vis struct #getter_name(#name);

                    impl #getter_name {
                        #(#getter_methods)*
                    }
                    
                    impl #crate_::WrappedGetter for #getter_name {
                        type WrappedDispatcher = #name;

                        fn __new(dispatcher: Self::WrappedDispatcher, _: #crate_::dispatcher::__private::Token) -> Self {
                            Self(dispatcher)
                        }
                    }
                    
                    impl #crate_::dispatcher::__private::Sealed for #getter_name {}
                }
            })
            .unwrap_or_default();

        Ok(quote! {
            #dispatcher_struct
            #impl_dispatcher
            #impl_wrapped_dispatcher_for_ty
            #wrappers
        })
    }

    fn generate_methods(
        &self,
    ) -> syn::Result<(Vec<TokenStream>, Vec<TokenStream>, Vec<TokenStream>)> {
        let model = &self.model_ty;
        let crate_ = &self.crate_;
        let mut dispatcher_methods = Vec::new();
        let mut updater_methods = Vec::new();
        let mut getter_methods = Vec::new();

        for method in &self.methods {
            let attrs = &method.attrs;
            let vis = &method.vis;

            match &method.kind {
                MethodKind::New => dispatcher_methods.push(quote! {
                    #(#attrs)*
                    #vis fn new(dispatcher: #crate_::Dispatcher<#model>) -> Self {
                        #crate_::WrappedDispatcher::__new(dispatcher, #crate_::dispatcher::__private::Token::new())
                    }
                }),
                MethodKind::Split => {
                    if let Some((update_def, getter_def)) = &self.updater_getter_defs {
                        let updater_name = &update_def.name;
                        let getter_name = &getter_def.name;
                        dispatcher_methods.push(quote! {
                            #(#attrs)*
                            #vis fn split(self) -> (#updater_name, #getter_name) {
                                #crate_::SplittableWrappedDispatcher::__split(self, #crate_::dispatcher::__private::Token::new())
                            }
                        })
                    } else {
                        return Err(syn::Error::new(
                            method.vis.span(),
                            "`split` method requires Updater and Getter definitions",
                        ));
                    }
                }
                MethodKind::Updater(action) => {
                    let (impl_dispatcher, impl_updater) =
                        self.generate_updater_fn(attrs, method.vis.clone(), action);
                    dispatcher_methods.push(impl_dispatcher);
                    updater_methods.push(impl_updater);
                }
                MethodKind::Getter(action) => {
                    let (impl_dispatcher, impl_getter) =
                        self.generate_getter_fn(attrs, method.vis.clone(), action);
                    dispatcher_methods.push(impl_dispatcher);
                    getter_methods.push(impl_getter);
                }
            }
        }

        Ok((dispatcher_methods, updater_methods, getter_methods))
    }
}

impl DispatcherDef {
    fn generate_action_fn(
        &self,
        attrs: &[Attribute],
        vis: Visibility,
        action: &MethodAction,
        method_call: TokenStream,
        return_ty: Option<TokenStream>,
    ) -> (TokenStream, TokenStream) {
        let fn_name = &action.signature.ident;
        let inputs = &action.signature.inputs;
        let message_ty = &action.message_ty;
        let args = extract_args(inputs);
        let closure_body = action
            .body
            .as_ref()
            .map(|b| quote! { #b })
            .unwrap_or_else(|| quote! { ::core::default::Default::default() });

        let return_signature = return_ty.map(|rty| quote! { -> #rty }).unwrap_or_default();

        let main_impl = quote! {
            #(#attrs)*
            #vis async fn #fn_name(&mut self, #inputs) #return_signature {
                let f: fn(#inputs) -> #message_ty = |#inputs| #closure_body;
                self.0.#method_call(f(#args)).await
            }
        };

        let wrapper_impl = quote! {
            #(#attrs)*
            #vis async fn #fn_name(&mut self, #inputs) #return_signature {
                self.0.#fn_name(#args).await
            }
        };

        (main_impl, wrapper_impl)
    }

    fn generate_updater_fn(
        &self,
        attrs: &[Attribute],
        vis: Visibility,
        action: &MethodAction,
    ) -> (TokenStream, TokenStream) {
        self.generate_action_fn(attrs, vis, action, quote! { send }, None)
    }

    fn generate_getter_fn(
        &self,
        attrs: &[Attribute],
        vis: Visibility,
        action: &MethodAction,
    ) -> (TokenStream, TokenStream) {
        let crate_ = &self.crate_;
        let message_ty = &action.message_ty;
        let return_ty = action
            .return_ty
            .as_ref()
            .map(|rty| quote! { #rty })
            .unwrap_or_else(|| quote! { <#message_ty as #crate_::ModelGetterMessage>::Data });

        self.generate_action_fn(attrs, vis, action, quote! { get }, Some(return_ty))
    }
}

impl Parse for DispatcherDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // $(#[$($meta:meta)*)* $vis struct $name for $model_ty
        let ty_attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse::<Visibility>()?;
        let _struct = input.parse::<Token![struct]>()?;
        let name = input.parse::<Ident>()?;
        let _for = input.parse::<Token![for]>()?;
        let model_ty = input.parse::<Type>()?;

        let mut updater_def = None;
        let mut getter_def = None;
        let updater_getter_defs = if input.peek(Token![where]) {
            let _where = input.parse::<Token![where]>()?;

            loop {
                let key = input.parse::<Ident>()?;
                let _eq = input.parse::<Token![=]>()?;
                let wrapper_def = WrapperDef {
                    ty_attrs: input.call(Attribute::parse_outer)?,
                    name: input.parse()?,
                };

                if key == "Updater" {
                    updater_def = Some(wrapper_def)
                } else if key == "Getter" {
                    getter_def = Some(wrapper_def)
                } else {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("Expected 'Updater' or 'Getter', found '{key}"),
                    ));
                }

                if input.peek(Token![,]) {
                    let _comma = input.parse::<Token![,]>();
                    if input.peek(token::Brace) {
                        break;
                    }
                } else {
                    break;
                }
            }

            if let (Some(updater_def), Some(getter_def)) = (updater_def, getter_def) {
                Some((updater_def, getter_def))
            } else {
                return Err(syn::Error::new(
                    input.span(),
                    "Both Updater and Getter needs to be passed",
                ));
            }
        } else {
            None
        };

        let content;
        braced!(content in input);

        let mut methods = Vec::new();
        while !content.is_empty() {
            methods.push(content.parse()?)
        }

        let crate_ = match crate_name("vye").expect("`vye` crate should be present in `Cargo.toml`")
        {
            FoundCrate::Itself => quote! { crate },
            FoundCrate::Name(name) => {
                let ident = Ident::new(&name, Span::call_site());
                quote! { #ident }
            }
        };

        Ok(Self {
            ty_attrs,
            vis,
            name,
            model_ty,
            updater_getter_defs,
            methods,
            crate_,
        })
    }
}

struct WrapperDef {
    ty_attrs: Vec<Attribute>,
    name: Ident,
}

struct MethodDef {
    attrs: Vec<Attribute>,
    vis: Visibility,
    kind: MethodKind,
}

impl Parse for MethodDef {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse::<Visibility>()?;

        let lookahead = input.lookahead1();
        if lookahead.peek(Token![fn]) {
            let _fn = input.parse::<Token![fn]>()?;
            let name = input.parse::<Ident>()?;
            let _semi = input.parse::<Token![;]>()?;
            let kind = if name == "new" {
                MethodKind::New
            } else if name == "split" {
                MethodKind::Split
            } else {
                return Err(syn::Error::new(
                    name.span(),
                    "Only 'new' or 'split' allowed here, or use 'updater/getter' fn",
                ));
            };

            Ok(MethodDef { attrs, vis, kind })
        } else {
            let kind = input.parse::<Ident>()?;
            let signature = input.parse::<Signature>()?;

            // (MessageType) or (MessageType, ReturnType)
            let (message_ty, return_ty) = match &signature.output {
                ReturnType::Type(_, ty) => {
                    if let Type::Tuple(tuple) = &**ty {
                        if tuple.elems.len() == 2 {
                            (tuple.elems[0].clone(), Some(tuple.elems[1].clone()))
                        } else {
                            return Err(syn::Error::new(
                                signature.output.span(),
                                "Tuple return must have exactly 2 elements: (MessageType, ReturnType)",
                            ));
                        }
                    } else {
                        (*ty.clone(), None)
                    }
                }
                ReturnType::Default => {
                    return Err(syn::Error::new(
                        signature.output.span(),
                        "Expected return type in the form of (MessageType) or (MessageType, ReturnType)",
                    ));
                }
            };

            let body = if input.peek(Token![;]) {
                let _semi = input.parse::<Token![;]>()?;
                None
            } else {
                Some(input.parse()?)
            };

            let action = MethodAction {
                signature,
                message_ty,
                return_ty,
                body,
            };

            let kind = if kind == "updater" {
                MethodKind::Updater(action)
            } else if kind == "getter" {
                MethodKind::Getter(action)
            } else {
                return Err(syn::Error::new(
                    kind.span(),
                    "Expected 'updater' or 'getter'",
                ));
            };

            Ok(MethodDef { attrs, vis, kind })
        }
    }
}

enum MethodKind {
    New,
    Split,
    Updater(MethodAction),
    Getter(MethodAction),
}

struct MethodAction {
    signature: Signature,
    message_ty: Type,
    return_ty: Option<Type>,
    body: Option<Expr>,
}
