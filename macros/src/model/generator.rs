use crate::model::attr::{ModelArgs, NewMethodArgs, Properties};
use crate::model::{ModelContext, ParsedFnArg, ParsedGetterFn, ParsedNewFn, ParsedSplitFn, ParsedUpdaterFn, ParsedUpdaterGetterFn};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, quote};
use syn::{TypePath, Visibility};
use crate::model::attr::raw::ProcessedMeta;

impl<'a> ModelContext<'a> {
    pub(super) fn generate(&self) -> TokenStream {
        let dispatcher = self.generate_dispatcher();
        let updater = self.generate_updater();
        let getter = self.generate_getter();
        quote! {
            #dispatcher
            #updater
            #getter
        }
    }
}

impl<'a> ModelContext<'a> {
    fn generate_any(&self, struct_decl: TokenStream, impls: TokenStream) -> TokenStream {
        quote! {
            #struct_decl
            #impls
        }
    }

    fn generate_any_struct(
        &self,
        props_accessor: impl FnOnce(&ModelArgs) -> &Properties,
        f: impl FnOnce(&TokenStream, &Visibility, &TypePath, &Ident, &[ProcessedMeta], &[ProcessedMeta]) -> TokenStream,
    ) -> TokenStream {
        let props = props_accessor(&self.args);
        let crate_ = &self.crate_;
        let vis = self.struct_vis;
        let model_ty = self.model_ty;
        let name = &props.name;
        let outer_meta = &props.outer_meta;
        let inner_meta = &props.inner_meta;
        f(crate_, vis, model_ty, name, outer_meta, inner_meta)
    }
}

impl<'a> ModelContext<'a> {
    fn generate_dispatcher(&self) -> TokenStream {
        self.generate_any(
            self.generate_dispatcher_struct(),
            self.generate_dispatcher_impls(),
        )
    }

    fn generate_dispatcher_struct(&self) -> TokenStream {
        self.generate_any_struct(
            |a| &a.dispatcher,
            |crate_, vis, model_ty, name, outer_meta, inner_meta| {
                quote! {
                    #(#[#outer_meta])*
                    #vis struct #name(#(#[#inner_meta])* #crate_::Dispatcher<#model_ty>);
                }
            },
        )
    }

    fn generate_dispatcher_impls(&self) -> TokenStream {
        let crate_ = &self.crate_;
        let model_ty = self.model_ty;
        let dispatcher_name = &self.args.dispatcher.name;
        let updater_name = &self.args.updater.name;
        let getter_name = &self.args.getter.name;
        let new_fn = self.new_fn.generate_for_dispatcher(crate_, model_ty);
        let split_fn = self.split_fn.generate(crate_, updater_name, getter_name);
        let dispatcher_fns = self
            .updaters
            .iter()
            .map(ParsedUpdaterFn::generate_dispatcher_fn)
            .chain(
                self.getters
                    .iter()
                    .map(ParsedGetterFn::generate_dispatcher_fn),
            );

        quote! {
            impl #crate_::WrappedDispatcher for #dispatcher_name {
                type Model = #model_ty;
                type Updater = #updater_name;
                type Getter = #getter_name;
                fn __new(dispatcher: #crate_::Dispatcher<#model_ty>, _token: #crate_::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::__private::Sealed for #dispatcher_name {}
            impl #dispatcher_name {
                #new_fn
                #split_fn
                #(#dispatcher_fns)*
            }
        }
    }
}

impl<'a> ModelContext<'a> {
    fn generate_updater_getter_impls<AnyFn>(
        &self,
        trait_name: &'static str,
        props_accessor: impl FnOnce(&ModelArgs) -> &Properties,
        new_fn: impl FnOnce(&ParsedNewFn, &TokenStream, &Ident) -> TokenStream,
        updater_getter_accessor: impl FnOnce(&Self) -> &Vec<AnyFn>,
        generate_fn: impl Fn(&AnyFn) -> TokenStream,
        generate_message_struct_and_trait_impls_fn: impl Fn(
            &AnyFn,
            &TypePath,
            &TokenStream,
        ) -> TokenStream,
    ) -> TokenStream {
        let trait_name = Ident::new(trait_name, Span::call_site());
        let props = props_accessor(&self.args);
        let updater_getter = updater_getter_accessor(self);
        let crate_ = &self.crate_;
        let dispatcher_name = &self.args.dispatcher.name;
        let updater_getter_name = &props.name;
        let new_fn = new_fn(&self.new_fn, crate_, dispatcher_name);
        let updater_fns = updater_getter.iter().map(generate_fn);
        let message_structs_and_trait_impls = updater_getter
            .iter()
            .map(|u| generate_message_struct_and_trait_impls_fn(u, self.model_ty, crate_));

        quote! {
            impl #crate_::#trait_name for #updater_getter_name {
                type WrappedDispatcher = #dispatcher_name;
                fn __new(
                    dispatcher: <Self as #crate_::#trait_name>::WrappedDispatcher,
                    _token: #crate_::__private::Token,
                ) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::__private::Sealed for #updater_getter_name {}
            impl #updater_getter_name {
                #new_fn
                #(#updater_fns)*
            }
            #(#message_structs_and_trait_impls)*
        }
    }
}

impl<'a> ModelContext<'a> {
    fn generate_updater(&self) -> TokenStream {
        self.generate_any(
            self.generate_updater_struct(),
            self.generate_updater_impls(),
        )
    }

    fn generate_updater_struct(&self) -> TokenStream {
        self.generate_any_struct(
            |a| &a.updater,
            |_, vis, _, updater_name, outer_meta, inner_meta| {
                let dispatcher_name = &self.args.dispatcher.name;
                quote! {
                    #(#[#outer_meta])*
                    #vis struct #updater_name(#(#[#inner_meta])* #dispatcher_name);
                }
            },
        )
    }

    fn generate_updater_impls(&self) -> TokenStream {
        self.generate_updater_getter_impls(
            "WrappedUpdater",
            |a| &a.updater,
            |new_fn, crate_, dispatcher_name| new_fn.generate_for_updater(crate_, dispatcher_name),
            |m| &m.updaters,
            |u| u.generate_updater_fn(),
            |updater_fn, model_ty, crate_| {
                updater_fn.generate_message_struct_and_trait_impls(model_ty, crate_)
            },
        )
    }
}

impl<'a> ModelContext<'a> {
    fn generate_getter(&self) -> TokenStream {
        let struct_decl = self.generate_getter_struct();
        let impls = self.generate_getter_impls();

        quote! {
            #struct_decl
            #impls
        }
    }

    fn generate_getter_struct(&self) -> TokenStream {
        self.generate_any_struct(
            |a| &a.getter,
            |_, vis, _, getter_name, outer_meta, inner_meta| {
                let dispatcher_name = &self.args.dispatcher.name;
                quote! {
                    #(#[#outer_meta])*
                    #vis struct #getter_name(#(#[#inner_meta])* #dispatcher_name);
                }
            },
        )
    }

    fn generate_getter_impls(&self) -> TokenStream {
        self.generate_updater_getter_impls(
            "WrappedGetter",
            |a| &a.getter,
            |new_fn, crate_, dispatcher_name| new_fn.generate_for_getter(crate_, dispatcher_name),
            |m| &m.getters,
            |g| g.generate_getter_fn(),
            |getter_fn, model_ty, crate_| {
                getter_fn.generate_message_struct_and_trait_impls(model_ty, crate_)
            },
        )
    }
}

impl<'a> ParsedFnArg<'a> {
    fn generate_fn_arg(&self) -> TokenStream {
        let Self { name, ty, .. } = *self;
        quote! { #name: #ty }
    }

    fn generate_field(&self) -> TokenStream {
        let Self { attrs, name, ty } = *self;
        quote! { #(#[#attrs])* #name: #ty }
    }
}

impl ParsedNewFn {
    fn generate(
        &self,
        crate_: &TokenStream,
        wrapped_ty: &'static str,
        dispatcher_ty: TokenStream,
        meta_fn: impl FnOnce(&NewMethodArgs) -> &Vec<ProcessedMeta>,
    ) -> TokenStream {
        let wrapped_ty = Ident::new(wrapped_ty, Span::call_site());
        let vis = &self.0.vis;
        let meta = meta_fn(&self.0.method_args);

        quote! {
            #(#[#meta])*
            #vis fn new(dispatcher: #dispatcher_ty) -> Self {
                #crate_::#wrapped_ty::__new(dispatcher, #crate_::__private::Token::new())
            }
        }
    }

    fn generate_for_dispatcher(&self, crate_: &TokenStream, model_ty: &TypePath) -> TokenStream {
        self.generate(
            crate_,
            "WrappedDispatcher",
            quote! { #crate_::Dispatcher<#model_ty> },
            |args| &args.dispatcher_meta,
        )
    }

    fn generate_for_updater(&self, crate_: &TokenStream, dispatcher_name: &Ident) -> TokenStream {
        self.generate(
            crate_,
            "WrappedUpdater",
            quote! { #dispatcher_name },
            |args| &args.updater_meta,
        )
    }

    fn generate_for_getter(&self, crate_: &TokenStream, dispatcher_name: &Ident) -> TokenStream {
        self.generate(
            crate_,
            "WrappedGetter",
            quote! { #dispatcher_name },
            |args| &args.getter_meta,
        )
    }
}

impl<'a> ParsedSplitFn<'a> {
    fn generate(&self, crate_: &TokenStream, updater_name: &Ident, getter_name: &Ident) -> TokenStream {
        let vis = &self.vis;
        let attrs = &self.attrs;

        quote! {
            #(#[#attrs])*
            #vis fn split(self) -> (#updater_name, #getter_name) {
                #crate_::WrappedDispatcher::__split(self, #crate_::__private::Token::new())
            }
        }
    }
}

impl<'a> ParsedUpdaterGetterFn<'a> {
    fn generate_message_struct(&self) -> TokenStream {
        let vis = self.vis;
        let outer_meta = &self.method_args.message.outer_meta;
        let name = &self.method_args.message.name;
        let fields = self.fn_args.iter().map(|fa| fa.generate_field());

        quote! {
            #(#[#outer_meta])*
            #vis struct #name {
                #(#fields),*
            }
        }
    }

    fn generate_message_struct_and_trait_impls(
        &self,
        trait_impls_fn: impl FnOnce(&Ident, Vec<&Ident>) -> TokenStream,
    ) -> TokenStream {
        let struct_decl = self.generate_message_struct();
        let message_name = &self.method_args.message.name;
        let field_names = self.fn_args.iter().map(|fa| fa.name).collect::<Vec<_>>();
        let trait_impls = trait_impls_fn(message_name, field_names);

        quote! {
            #struct_decl
            #trait_impls
        }
    }

    fn generate_dispatcher_fn(
        &self,
        f: impl FnOnce(
            &Visibility,
            &[ProcessedMeta],
            &Ident,
            &Ident,
            Vec<&Ident>,
            Vec<TokenStream>,
        ) -> TokenStream,
    ) -> TokenStream {
        let vis = self.vis;
        let meta = &self.method_args.dispatcher_fn_meta;
        let fn_name = &self.method_args.fn_name;
        let message_name = &self.method_args.message.name;
        let field_names = self.fn_args.iter().map(|fa| fa.name).collect::<Vec<_>>();
        let fn_args = self
            .fn_args
            .iter()
            .map(|fa| fa.generate_fn_arg())
            .collect::<Vec<_>>();

        f(vis, meta, fn_name, message_name, field_names, fn_args)
    }

    fn generate_updater_getter_fn(
        &self,
        f: impl FnOnce(&Visibility, &[ProcessedMeta], &Ident, Vec<TokenStream>, Vec<&Ident>) -> TokenStream,
    ) -> TokenStream {
        let vis = self.vis;
        let meta = &self.method_args.fn_meta;
        let fn_name = &self.method_args.fn_name;
        let fn_args = self
            .fn_args
            .iter()
            .map(|fa| fa.generate_fn_arg())
            .collect::<Vec<_>>();
        let fn_arg_names = self.fn_args.iter().map(|fa| fa.name).collect::<Vec<_>>();

        f(vis, meta, fn_name, fn_args, fn_arg_names)
    }
}

impl<'a> ParsedUpdaterFn<'a> {
    fn generate_message_struct_and_trait_impls(
        &self,
        model_ty: &TypePath,
        crate_: &TokenStream,
    ) -> TokenStream {
        self.common
            .generate_message_struct_and_trait_impls(|message_name, field_names| {
                let ctx = self
                    .ctx
                    .cloned()
                    .unwrap_or_else(|| Ident::new("_", Span::call_site()));
                let block = &self.block;

                quote! {
                    impl #crate_::ModelMessage for #message_name {}
                    impl #crate_::ModelHandler<#message_name> for #model_ty {
                        fn update(
                            &mut self,
                            #message_name { #(#field_names),* }: #message_name,
                            #ctx: &mut #crate_::UpdateContext<<Self as #crate_::Model>::ForApp>,
                        ) { #block }
                    }
                }
            })
    }

    fn generate_dispatcher_fn(&self) -> TokenStream {
        self.common.generate_dispatcher_fn(
            |vis, meta, fn_name, message_name, field_names, fn_args| {
                quote! {
                    #(#[#meta])*
                    #vis async fn #fn_name(&mut self, #(#fn_args),*) {
                        self.0.send(#message_name { #(#field_names),* }).await
                    }
                }
            },
        )
    }

    fn generate_updater_fn(&self) -> TokenStream {
        self.common
            .generate_updater_getter_fn(|vis, meta, fn_name, fn_args, fn_arg_names| {
                quote! {
                    #(#[#meta])*
                    #vis async fn #fn_name(&mut self, #(#fn_args),*) {
                        self.0.#fn_name(#(#fn_arg_names),*).await
                    }
                }
            })
    }
}

impl<'a> ParsedGetterFn<'a> {
    fn generate_message_struct_and_trait_impls(
        &self,
        model_ty: &TypePath,
        crate_: &TokenStream,
    ) -> TokenStream {
        self.common
            .generate_message_struct_and_trait_impls(|message_name, field_names| {
                let ret_ty = self.ret_ty;
                let block = self
                    .block
                    .map(ToTokens::to_token_stream)
                    .unwrap_or_else(|| {
                        let field_name = &self.common.method_args.fn_name;
                        quote! { ::core::clone::Clone::clone(&self.#field_name) }
                    });
                quote! {
                    impl #crate_::ModelGetterMessage for #message_name {
                        type Data = #ret_ty;
                    }
                    impl #crate_::ModelGetterHandler<#message_name> for #model_ty {
                        fn getter(
                            &self,
                            #message_name { #(#field_names),* }: #message_name,
                        ) -> #ret_ty { #block }
                    }
                }
            })
    }

    fn generate_dispatcher_fn(&self) -> TokenStream {
        self.common.generate_dispatcher_fn(
            |vis, meta, fn_name, message_name, field_names, fn_args| {
                let ret_ty = self.ret_ty;
                quote! {
                    #(#[#meta])*
                    #vis fn #fn_name(&mut self, #(#fn_args),*) -> #ret_ty {
                        self.0.get(#message_name { #(#field_names),* })
                    }
                }
            },
        )
    }

    fn generate_getter_fn(&self) -> TokenStream {
        self.common
            .generate_updater_getter_fn(|vis, meta, fn_name, fn_args, fn_arg_names| {
                let ret_ty = self.ret_ty;
                quote! {
                    #(#[#meta])*
                    #vis fn #fn_name(&mut self, #(#fn_args),*) -> #ret_ty {
                        self.0.#fn_name(#(#fn_arg_names),*)
                    }
                }
            })
    }
}
