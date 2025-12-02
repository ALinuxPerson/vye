use crate::utils::{InterfaceImpl, MaybeStubFn};
use crate::{crate_, utils};
use convert_case::{Case, Casing};
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Block, FnArg, MetaList, Pat, PatIdent, PatType, PathArguments, ReturnType,
    Signature, Token, Type, TypePath, Visibility,
};
// ==================================================================================
// Utilities & Helpers
// ==================================================================================

fn meta_to_token_stream(meta: &syn::Meta) -> syn::Result<TokenStream> {
    match meta {
        syn::Meta::List(MetaList { tokens, .. }) => Ok(quote! { #[#tokens] }),
        _ => Err(syn::Error::new_spanned(
            meta,
            "Expected attribute meta to be a list",
        )),
    }
}

fn get_model_name(ty: &Type) -> syn::Result<Ident> {
    match ty {
        Type::Path(TypePath { path, .. }) => path
            .segments
            .last()
            .ok_or_else(|| syn::Error::new_spanned(ty, "Provided type path has no segments"))
            .map(|seg| seg.ident.clone()),
        _ => Err(syn::Error::new_spanned(
            ty,
            "Expected a type path for the model type",
        )),
    }
}

// ==================================================================================
// Argument Structs
// ==================================================================================

#[derive(FromMeta)]
struct GenerateDispatcherArgs {
    #[darling(default)]
    dispatcher: Option<Ident>,

    #[darling(default)]
    vis: Option<Visibility>,

    /// Automatically generates `#[derive(Clone)]` for the dispatcher, updater, and getter
    #[darling(default)]
    clone: bool,

    /// Shorthand for `pub fn new();`
    #[darling(default)]
    new: bool,

    /// Shorthand for `pub fn split() -> (_, _);`
    #[darling(default)]
    split: bool,

    /// Outer attributes of the generated dispatcher struct
    #[darling(multiple, rename = "attr")]
    attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "inner_attr")]
    inner_attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "updater_attr")]
    updater_attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "updater_inner_attr")]
    updater_inner_attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "getter_attr")]
    getter_attrs: Vec<syn::Meta>,

    #[darling(multiple, rename = "getter_inner_attr")]
    getter_inner_attrs: Vec<syn::Meta>,
}

impl GenerateDispatcherArgs {
    fn dispatcher_ident(&self, model_name: &Ident) -> Ident {
        self.dispatcher
            .clone()
            .unwrap_or_else(|| format_ident!("{model_name}Dispatcher"))
    }

    fn vis(&self) -> Option<Visibility> {
        if self.new {
            Some(Visibility::Public(Token![pub](Span::call_site())))
        } else {
            self.vis.clone()
        }
    }

    fn derive_clone(&self) -> TokenStream {
        if self.clone {
            quote! { #[derive(Clone)] }
        } else {
            TokenStream::new()
        }
    }
}

impl GenerateDispatcherArgs {
    fn attrs(&self) -> syn::Result<(Vec<TokenStream>, Vec<TokenStream>)> {
        let outer = self
            .attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        let inner = self
            .inner_attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        Ok((outer, inner))
    }

    fn updater_attrs(&self) -> syn::Result<(Vec<TokenStream>, Vec<TokenStream>)> {
        let outer = self
            .updater_attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        let inner = self
            .updater_inner_attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        Ok((outer, inner))
    }

    fn getter_attrs(&self) -> syn::Result<(Vec<TokenStream>, Vec<TokenStream>)> {
        let outer = self
            .getter_attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        let inner = self
            .getter_inner_attrs
            .iter()
            .map(meta_to_token_stream)
            .collect::<syn::Result<_>>()?;
        Ok((outer, inner))
    }
}

#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct DispatcherArgs {
    generate: Option<GenerateDispatcherArgs>,
}

#[derive(FromAttributes, Default)]
#[darling(attributes(vye))]
struct MethodArgs {
    /// Name of the generated Message struct
    #[darling(default)]
    name: Option<Ident>,

    /// Applicable to getters only; clones the field as determined by the function name and returns
    /// it
    #[darling(default)]
    // `clone` is implied anyway if it is not passed but there is no block, just add this here to
    // keep the illusion
    #[allow(dead_code)]
    clone: bool,

    /// Applicable to getters only; copies the field as determined by the function name and returns
    /// it
    #[darling(default)]
    copy: bool,

    /// Common attributes for dispatcher, updater, getter methods
    #[darling(multiple, rename = "method_attr")]
    method_attrs: Vec<syn::Meta>,

    /// Attributes for the generated dispatcher method
    #[darling(multiple, rename = "dispatcher_attr")]
    dispatcher_attrs: Vec<syn::Meta>,

    /// Attributes for the generated updater method
    #[darling(multiple, rename = "updater_attr")]
    updater_attrs: Vec<syn::Meta>,

    /// Attributes for the generated getter method
    #[darling(multiple, rename = "getter_attr")]
    getter_attrs: Vec<syn::Meta>,
}

impl MethodArgs {
    pub fn dispatcher_attrs(&self) -> syn::Result<Vec<TokenStream>> {
        self.method_attrs
            .iter()
            .chain(&self.dispatcher_attrs)
            .map(meta_to_token_stream)
            .collect()
    }

    pub fn updater_attrs(&self) -> syn::Result<Vec<TokenStream>> {
        self.method_attrs
            .iter()
            .chain(&self.updater_attrs)
            .map(meta_to_token_stream)
            .collect()
    }

    pub fn getter_attrs(&self) -> syn::Result<Vec<TokenStream>> {
        self.method_attrs
            .iter()
            .chain(&self.getter_attrs)
            .map(meta_to_token_stream)
            .collect()
    }
}

#[derive(FromAttributes, Default)]
#[darling(attributes(vye))]
struct FieldArgs {
    #[darling(default)]
    vis: Option<Visibility>,
}

// ==================================================================================
// Core Analysis Structures
// ==================================================================================

enum MethodKind {
    /// fn new() -> Self
    Constructor,
    /// fn split(self) -> (Updater, Getter)
    Splitter {
        updater: Option<Ident>,
        getter: Option<Ident>,
    },
    /// &mut self
    Updater { context_arg: Option<Ident> },
    /// &self
    Getter { return_ty: Box<Type> },
}

struct ParsedMethod<'a> {
    args: MethodArgs,
    kind: MethodKind,
    attrs: Vec<&'a Attribute>,
    vis: &'a Visibility,
    sig: &'a Signature,
    block: Option<&'a Block>,
    fields: Vec<ParsedField<'a>>,
}

struct ParsedField<'a> {
    args: FieldArgs,
    attrs: Vec<&'a Attribute>,
    name: &'a Ident,
    ty: &'a Type,
}

struct DispatcherContext<'a> {
    args: DispatcherArgs,
    crate_root: TokenStream,

    // Model Info
    model_ty: &'a Type,
    model_name: Ident,

    // Parsed Items
    handlers: Vec<ParsedMethod<'a>>,

    constructor: Option<(Visibility, Vec<&'a Attribute>)>,
    splitter: Option<(Visibility, Vec<&'a Attribute>, Option<Ident>, Option<Ident>)>,
}

// ==================================================================================
// Analysis Logic (Parsing)
// ==================================================================================

impl<'a> DispatcherContext<'a> {
    fn new(interface_impl: &'a InterfaceImpl, args: DispatcherArgs) -> syn::Result<Self> {
        let model_name = get_model_name(&interface_impl.self_ty)?;
        let mut handlers = Vec::new();
        let mut constructor = None;
        let mut splitter = None;

        if args.generate.as_ref().map(|g| g.new).unwrap_or_default() {
            constructor = Some((
                Visibility::Public(Token![pub](Span::call_site())),
                Vec::new(),
            ));
        }

        if args.generate.as_ref().map(|g| g.split).unwrap_or_default() {
            splitter = Some((
                Visibility::Public(Token![pub](Span::call_site())),
                Vec::new(),
                None,
                None,
            ));
        }

        for item in &interface_impl.items {
            let parsed = ParsedMethod::new(item)?;

            match parsed.kind {
                MethodKind::Constructor => {
                    constructor = Some((parsed.vis.clone(), parsed.attrs));
                }
                MethodKind::Splitter { updater, getter } => {
                    splitter = Some((parsed.vis.clone(), parsed.attrs, updater, getter));
                }
                MethodKind::Updater { .. } | MethodKind::Getter { .. } => {
                    handlers.push(parsed);
                }
            }
        }

        Ok(Self {
            args,
            crate_root: crate_(),
            model_ty: &interface_impl.self_ty,
            model_name,
            handlers,
            constructor,
            splitter,
        })
    }
}

impl<'a> ParsedMethod<'a> {
    fn new(func: &'a MaybeStubFn) -> syn::Result<Self> {
        validate_signature(&func.sig)?;

        let block = func.block.as_ref();
        let (attrs, args) = utils::extract_vye_attrs(&func.attrs)?;
        let kind = MethodKind::determine(&func.sig, block)?;

        // Parse arguments into fields (skipping self and context)
        let mut fields = Vec::new();
        let mut context_arg = None;

        for input in &func.sig.inputs {
            match input {
                FnArg::Receiver(_) => continue,
                FnArg::Typed(pat_ty) => {
                    // Check if this is the generic UpdateContext arg
                    if matches!(kind, MethodKind::Updater { .. })
                        && let Some(ctx_ident) = get_update_context_ident(pat_ty)
                    {
                        context_arg = Some(ctx_ident.clone());
                        continue;
                    }

                    if let Some(field) = ParsedField::new(pat_ty)? {
                        fields.push(field);
                    }
                }
            }
        }

        // Update kind with found context arg if it's an updater
        let kind = match kind {
            MethodKind::Updater { .. } => MethodKind::Updater { context_arg },
            k => k,
        };

        Ok(Self {
            args,
            kind,
            attrs,
            vis: &func.vis,
            sig: &func.sig,
            block,
            fields,
        })
    }

    fn struct_name(&self) -> Ident {
        self.args.name.clone().unwrap_or_else(|| {
            let name = self.sig.ident.to_string().to_case(Case::Pascal);
            Ident::new(&name, Span::call_site())
        })
    }
}

impl<'a> ParsedField<'a> {
    fn new(pat_type: &'a PatType) -> syn::Result<Option<Self>> {
        let name = match &*pat_type.pat {
            Pat::Ident(PatIdent { ident, .. }) => ident,
            _ => return Ok(None),
        };

        let (attrs, args) = utils::extract_vye_attrs(&pat_type.attrs)?;

        Ok(Some(Self {
            args,
            attrs,
            name,
            ty: &pat_type.ty,
        }))
    }
}

// --- Analysis Helpers ---

fn validate_signature(sig: &Signature) -> syn::Result<()> {
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

impl MethodKind {
    fn determine(sig: &Signature, block: Option<&Block>) -> syn::Result<Self> {
        // 1. Check for `new` (Constructor)
        if sig.ident == "new" && sig.inputs.is_empty() && block.is_none() {
            return Ok(Self::Constructor);
        }

        // 2. Check for `split`
        if sig.ident == "split"
            && sig.inputs.is_empty()
            && block.is_none()
            && let ReturnType::Type(_, ty) = &sig.output
            && let Type::Tuple(tuple) = &**ty
            && tuple.elems.len() == 2
        {
            let get_ident = |t: &Type| -> Option<Ident> {
                if let Type::Path(p) = t {
                    p.path.segments.last().map(|s| s.ident.clone())
                } else {
                    None
                }
            };
            let (updater, getter) = (get_ident(&tuple.elems[0]), get_ident(&tuple.elems[1]));
            return Ok(Self::Splitter { updater, getter });
        }

        // 3. Check Receiver (&self vs &mut self)
        for input in &sig.inputs {
            if let FnArg::Receiver(recv) = input {
                return if recv.mutability.is_some() {
                    Ok(Self::Updater { context_arg: None })
                } else {
                    match &sig.output {
                        ReturnType::Type(_, ty) => Ok(Self::Getter {
                            return_ty: ty.clone(),
                        }),
                        ReturnType::Default => Err(syn::Error::new_spanned(
                            &sig.output,
                            "Getter functions must have a return type",
                        )),
                    }
                };
            }
        }

        Err(syn::Error::new_spanned(
            &sig.inputs,
            "Dispatcher functions must have a self parameter (or be new/split)",
        ))
    }
}

/// Helper to detect `&mut UpdateContext<App>`
fn get_update_context_ident(pat_ty: &PatType) -> Option<&Ident> {
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

// ==================================================================================
// Code Generation
// ==================================================================================

impl<'a> DispatcherContext<'a> {
    fn generate(&self) -> syn::Result<TokenStream> {
        // 1. Generate Message Structs and their Trait Implementations
        let messages = self
            .handlers
            .iter()
            .map(|h| h.generate_message_struct(&self.crate_root, self.model_ty))
            .collect::<syn::Result<Vec<_>>>()?;

        let output_items = quote! { #(#messages)* };

        // 2. If 'generate' args are present, generate the Dispatcher wrapper
        if let Some(gen_args) = &self.args.generate {
            let dispatcher = self.generate_wrapper(gen_args)?;
            Ok(quote! {
                #output_items
                #dispatcher
            })
        } else {
            Ok(output_items)
        }
    }

    fn generate_wrapper(&self, args: &GenerateDispatcherArgs) -> syn::Result<TokenStream> {
        let crate_ = &self.crate_root;
        let model_ty = self.model_ty;

        let dispatcher_name = args.dispatcher_ident(&self.model_name);
        let vis = args.vis().unwrap_or(Visibility::Inherited);
        let derive_clone = args.derive_clone();
        let (dispatcher_attrs, dispatcher_inner_attrs) = args.attrs()?;

        // Constructor info
        let (new_vis, new_attrs) = self
            .constructor
            .clone()
            .unwrap_or((Visibility::Inherited, Vec::new()));

        // Generate methods for the main dispatcher
        let dispatcher_methods = self
            .handlers
            .iter()
            .map(|h| h.generate_dispatcher_method())
            .collect::<syn::Result<Vec<_>>>()?;

        // Core Dispatcher definition
        let mut tokens = quote! {
            #(#dispatcher_attrs)*
            #derive_clone
            #vis struct #dispatcher_name(#(#dispatcher_inner_attrs)* #crate_::Dispatcher<#model_ty>);

            impl #dispatcher_name {
                #(#new_attrs)*
                #new_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedDispatcher::__new(dispatcher, #crate_::dispatcher::__private::Token::new())
                }
                #(#dispatcher_methods)*
            }

            impl #crate_::WrappedDispatcher for #dispatcher_name {
                type Model = #model_ty;
                fn __new(dispatcher: #crate_::Dispatcher<#model_ty>, _: #crate_::dispatcher::__private::Token) -> Self {
                    Self(dispatcher)
                }
            }
            impl #crate_::dispatcher::__private::Sealed for #dispatcher_name {}
        };

        // If 'split' is defined, generate Updater and Getter wrappers
        if let Some((split_vis, split_attrs, updater_name, getter_name)) = &self.splitter {
            let model_name = &self.model_name;
            let split_impl = self.generate_split_impl(
                args,
                &dispatcher_name,
                updater_name
                    .clone()
                    .unwrap_or_else(|| format_ident!("{model_name}Updater")),
                getter_name
                    .clone()
                    .unwrap_or_else(|| format_ident!("{model_name}Getter")),
                split_vis,
                split_attrs,
            )?;
            tokens.extend(split_impl);
        }

        Ok(tokens)
    }

    fn generate_split_impl(
        &self,
        args: &GenerateDispatcherArgs,
        dispatcher_name: &Ident,
        updater_name: Ident,
        getter_name: Ident,
        split_vis: &Visibility,
        split_attrs: &[&Attribute],
    ) -> syn::Result<TokenStream> {
        let crate_ = &self.crate_root;
        let model_ty = self.model_ty;
        let (new_vis, _) = self
            .constructor
            .clone()
            .unwrap_or((Visibility::Inherited, Vec::new()));

        // Separate handler methods into updater methods and getter methods
        let mut updater_fns = Vec::new();
        let mut getter_fns = Vec::new();

        for handler in &self.handlers {
            let wrapper_fn = handler.generate_wrapper_method()?;
            match handler.kind {
                MethodKind::Updater { .. } => updater_fns.push(wrapper_fn),
                MethodKind::Getter { .. } => getter_fns.push(wrapper_fn),
                _ => {}
            }
        }

        let derive_clone = args.derive_clone();
        let (updater_attrs, updater_inner_attrs) = args.updater_attrs()?;
        let (getter_attrs, getter_inner_attrs) = args.getter_attrs()?;

        Ok(quote! {
            // Split implementation on Dispatcher
            impl #dispatcher_name {
                #(#split_attrs)*
                #split_vis fn split(self) -> (#updater_name, #getter_name) {
                    #crate_::SplittableWrappedDispatcher::__split(self, #crate_::dispatcher::__private::Token::new())
                }
            }
            impl #crate_::SplittableWrappedDispatcher for #dispatcher_name {
                type Updater = #updater_name;
                type Getter = #getter_name;
            }

            // Updater Struct
            #(#updater_attrs)*
            #derive_clone
            #split_vis struct #updater_name(#(#updater_inner_attrs)* #dispatcher_name);

            impl #crate_::WrappedUpdater for #updater_name {
                type WrappedDispatcher = #dispatcher_name;
                fn __new(dispatcher: #dispatcher_name, _: #crate_::dispatcher::__private::Token) -> Self { Self(dispatcher) }
            }
            impl #crate_::dispatcher::__private::Sealed for #updater_name {}

            impl #updater_name {
                // todo: add ability to add attrs here
                #new_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedUpdater::__new(#dispatcher_name::new(dispatcher), #crate_::dispatcher::__private::Token::new())
                }
                #(#updater_fns)*
            }

            // Getter Struct
            #(#getter_attrs)*
            #derive_clone
            #split_vis struct #getter_name(#(#getter_inner_attrs)* #dispatcher_name);

            impl #crate_::WrappedGetter for #getter_name {
                type WrappedDispatcher = #dispatcher_name;
                fn __new(dispatcher: #dispatcher_name, _: #crate_::dispatcher::__private::Token) -> Self { Self(dispatcher) }
            }
            impl #crate_::dispatcher::__private::Sealed for #getter_name {}

            impl #getter_name {
                // todo: add ability to add attrs here
                #new_vis fn new(dispatcher: #crate_::Dispatcher<#model_ty>) -> Self {
                    #crate_::WrappedGetter::__new(#dispatcher_name::new(dispatcher), #crate_::dispatcher::__private::Token::new())
                }
                #(#getter_fns)*
            }
        })
    }
}

impl<'a> ParsedMethod<'a> {
    /// Generates the Message struct and the ModelHandler/ModelGetterHandler impl
    fn generate_message_struct(
        &self,
        crate_: &TokenStream,
        model_ty: &Type,
    ) -> syn::Result<TokenStream> {
        let struct_name = self.struct_name();
        let field_defs = self.fields.iter().map(|f| f.to_token_stream());
        let field_names = self.fields.iter().map(|f| f.name);

        let attrs = &self.attrs;
        let vis = self.vis;
        let block = self.block;
        let (impl_gen, ty_gen, where_clause) = self.sig.generics.split_for_impl();

        let struct_def = quote! {
            #(#attrs)*
            #vis struct #struct_name #impl_gen #where_clause {
                #(#field_defs),*
            }
        };

        match &self.kind {
            MethodKind::Updater { context_arg } => {
                let ctx_pat = context_arg
                    .as_ref()
                    .map(|i| quote! { #i })
                    .unwrap_or_else(|| quote! { _ });

                Ok(quote! {
                    #struct_def
                    impl #impl_gen #crate_::ModelMessage for #struct_name #ty_gen #where_clause {}
                    impl #impl_gen #crate_::ModelHandler<#struct_name> for #model_ty #ty_gen #where_clause {
                        fn update(
                            &mut self,
                            #struct_name { #(#field_names),* }: #struct_name #ty_gen,
                            #ctx_pat: &mut #crate_::UpdateContext<<#model_ty as #crate_::Model>::ForApp>,
                        ) {
                            #block
                        }
                    }
                })
            }
            MethodKind::Getter { return_ty } => {
                let field_name = &self.sig.ident;
                let block = if block.is_none() {
                    if self.args.copy {
                        quote! { self.#field_name }
                    } else {
                        quote! { self.#field_name.clone() }
                    }
                } else {
                    quote! { #block }
                };

                Ok(quote! {
                    #struct_def
                    impl #impl_gen #crate_::ModelGetterMessage for #struct_name #ty_gen #where_clause {
                        type Data = #return_ty;
                    }
                    impl #impl_gen #crate_::ModelGetterHandler<#struct_name> for #model_ty #where_clause {
                        fn getter(&self, #struct_name { #(#field_names),* }: #struct_name #ty_gen) -> #return_ty {
                            #block
                        }
                    }
                })
            }
            _ => Ok(TokenStream::new()), // Constructors/Splitters don't generate message structs
        }
    }

    fn generate_args(&self) -> Vec<TokenStream> {
        self.fields
            .iter()
            .map(|f| {
                let name = f.name;
                let ty = f.ty;
                quote! { #name: #ty }
            })
            .collect()
    }

    /// Generates the `async fn name(...)` for the main Dispatcher struct
    fn generate_dispatcher_method(&self) -> syn::Result<TokenStream> {
        let vis = self.vis;
        let fn_name = &self.sig.ident;
        let struct_name = self.struct_name();

        let args = self.generate_args();
        let field_names = self.fields.iter().map(|f| f.name).collect::<Vec<_>>();
        let field_tys = self.fields.iter().map(|f| f.ty);
        let closure_construction = quote! { #struct_name { #(#field_names),* } };
        let dispatcher_attrs = self.args.dispatcher_attrs()?;

        match &self.kind {
            MethodKind::Updater { .. } => Ok(quote! {
                #(#dispatcher_attrs)*
                #vis async fn #fn_name(&mut self, #(#args),*) {
                    let f: fn(#(#field_tys),*) -> #struct_name = |#(#field_names),*| #closure_construction;
                    self.0.send(f(#(#field_names),*)).await
                }
            }),
            MethodKind::Getter { return_ty } => Ok(quote! {
                #(#dispatcher_attrs)*
                #vis fn #fn_name(&mut self, #(#args),*) -> #return_ty {
                    let f: fn(#(#field_tys),*) -> #struct_name = |#(#field_names),*| #closure_construction;
                    self.0.get(f(#(#field_names),*))
                }
            }),
            _ => Ok(TokenStream::new()),
        }
    }

    /// Generates the method for the Updater/Getter wrapper (delegates to inner dispatcher)
    fn generate_wrapper_method(&self) -> syn::Result<TokenStream> {
        let vis = self.vis;
        let fn_name = &self.sig.ident;

        let args = self.generate_args();
        let field_names = self.fields.iter().map(|f| f.name);
        let (return_type, attrs, async_, dot_await) = match &self.kind {
            MethodKind::Getter { return_ty } => (
                quote! { -> #return_ty },
                self.args.getter_attrs()?,
                TokenStream::new(),
                TokenStream::new(),
            ),
            MethodKind::Updater { .. } => (
                TokenStream::new(),
                self.args.updater_attrs()?,
                quote! { async },
                quote! { .await },
            ),
            _ => (
                TokenStream::new(),
                Vec::new(),
                TokenStream::new(),
                TokenStream::new(),
            ),
        };

        Ok(quote! {
            #(#attrs)*
            #vis #async_ fn #fn_name(&mut self, #(#args),*) #return_type {
                self.0.#fn_name(#(#field_names),*) #dot_await
            }
        })
    }
}

impl<'a> ToTokens for ParsedField<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let attrs = &self.attrs;
        let vis = self.args.vis.clone().unwrap_or(Visibility::Inherited);
        let name = self.name;
        let ty = self.ty;

        tokens.extend(quote! {
            #(#attrs)*
            #vis #name: #ty
        });
    }
}

pub fn build(interface_impl: InterfaceImpl, args: DispatcherArgs) -> syn::Result<TokenStream> {
    DispatcherContext::new(&interface_impl, args)?.generate()
}
