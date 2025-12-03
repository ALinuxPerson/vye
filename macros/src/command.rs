use crate::{crate_, utils};
use convert_case::ccase;
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use syn::{
    AngleBracketedGenericArguments, Attribute, Block, FnArg, GenericArgument, Generics, ItemFn,
    Pat, PatIdent, PatType, PathArguments, Signature, Type, TypeReference, Visibility,
};

/// All attrs shall be interpreted as the attrs for the generated Command struct. The visibility
/// of the function shall determine the visibility of the generated Command struct. Must not
/// have a return type. Can be async or not async. `ctx` argument can be elided
#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct CommandArgs {
    /// Inferred by the presence of `CommandContext<App>`, _if present_, in the function
    /// arguments.
    for_app: Option<Type>,

    /// Name of the generated Command struct, otherwise, the function name in PascalCase.
    name: Option<Ident>,

    /// Automatically derive debug for the generated Command struct.
    debug: bool,
}

enum FieldKind {
    Field,
    State,
}

#[derive(FromAttributes)]
#[darling(attributes(vye))]
struct FieldArgs {
    /// Visibility of the generated field. Defaults to inherited.
    #[darling(default)]
    vis: Option<Visibility>,

    /// The field shall be generated as a part of the generated Command struct. Accepts attrs.
    /// Must be a (mutable) reference.
    #[darling(default)]
    field: bool,

    /// The field shall be retrieved as state from the CommandContext. This is the default if
    /// neither `field` nor `state` is specified. Must be a (mutable) reference.
    #[darling(default)]
    #[allow(dead_code)]
    state: bool,
}

impl FieldArgs {
    fn kind(&self) -> FieldKind {
        if self.field {
            FieldKind::Field
        } else {
            FieldKind::State
        }
    }
}

fn validate_signature(sig: &Signature) -> syn::Result<()> {
    if sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Command functions cannot be const",
        ));
    }
    if sig.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Command functions cannot be unsafe",
        ));
    }
    if sig.abi.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Command functions cannot have a custom ABI",
        ));
    }
    if sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "Command functions cannot be variadic",
        ));
    }
    Ok(())
}

pub struct CommandContext<'a> {
    crate_: TokenStream,
    args: CommandArgs,
    ctx_name: Ident,
    for_app_ty: Type,
    struct_attrs: &'a [Attribute],
    fn_name: &'a Ident,
    generics: &'a Generics,
    vis: &'a Visibility,
    fields: Vec<ParsedField<'a>>,
    block: &'a Block,
}

impl<'a> CommandContext<'a> {
    fn new(item: &'a ItemFn, args: CommandArgs) -> syn::Result<Self> {
        validate_signature(&item.sig)?;
        let (fields, ctx) = item.sig.inputs.iter().map(ParsedField::new).try_fold(
            (Vec::with_capacity(item.sig.inputs.len()), None),
            |(mut fields, mut ctx), field| {
                let (field, field_ctx) = field?;
                fields.push(field);
                ctx = field_ctx;
                Ok::<_, syn::Error>((fields, ctx))
            },
        )?;
        let (ctx_name, for_app_ty) = match ctx {
            Some((name, for_app_ty)) => (name.clone(), for_app_ty.clone()),
            None => match &args.for_app {
                Some(ty) => (format_ident!("_ctx"), ty.clone()),
                None => {
                    return Err(syn::Error::new_spanned(
                        &item.sig,
                        "Command functions must have an `&mut CommandContext<App>` argument or specify `for_app`",
                    ));
                }
            },
        };

        Ok(Self {
            crate_: crate_(),
            args,
            ctx_name,
            for_app_ty,
            struct_attrs: &item.attrs,
            fn_name: &item.sig.ident,
            generics: &item.sig.generics,
            vis: &item.vis,
            fields,
            block: &item.block,
        })
    }

    fn expand(&self) -> TokenStream {
        let crate_ = &self.crate_;
        let derive_debug = if self.args.debug {
            quote! { #[derive(Debug)] }
        } else {
            TokenStream::new()
        };
        let ctx_name = &self.ctx_name;
        let for_app_ty = &self.for_app_ty;
        let struct_attrs = self.struct_attrs;
        let struct_name = self.args.name.clone().unwrap_or_else(|| {
            Ident::new(
                &ccase!(pascal, self.fn_name.to_string()),
                self.fn_name.span(),
            )
        });
        let (impl_generics, ty_generics, where_clause) = self.generics.split_for_impl();
        let vis = self.vis;
        let fields = self.fields.iter().filter_map(|f| f.generate_field());
        let field_names = self.fields.iter().filter_map(|f| {
            if matches!(f.args.kind(), FieldKind::Field) {
                Some(f.name)
            } else {
                None
            }
        });
        let var_statements = self
            .fields
            .iter()
            .map(|f| f.generate_var_statement(ctx_name));
        let block = &self.block;

        quote! {
            #derive_debug
            #(#struct_attrs)*
            #vis struct #struct_name #impl_generics #where_clause {
                #(#fields)*
            }

            #[#crate_::__macros::async_trait]
            impl #impl_generics #crate_::Command for #struct_name #ty_generics #where_clause {
                type ForApp = #for_app_ty;

                async fn apply(&mut self, #ctx_name: &mut #crate_::CommandContext<'_, #for_app_ty>) {
                    let Self { #(#field_names),* } = self;
                    #(#var_statements)*
                    #block
                }
            }
        }
    }
}

struct ParsedField<'a> {
    args: FieldArgs,
    attrs: Vec<&'a Attribute>,
    name: &'a Ident,
    ty: &'a TypeReference,
}

fn is_apply_context(ty: &TypeReference) -> Option<&Type> {
    // from `&mut CommandContext<MyApp>`, extract `MyApp`
    if ty.mutability.is_some()
        && let Type::Path(ty_path) = &*ty.elem
        && let Some(segment) = ty_path.path.segments.last()
        && segment.ident == "CommandContext"
        && let PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) =
            &segment.arguments
        && args.len() == 1
        && let GenericArgument::Type(for_app_ty) = &args[0]
    {
        Some(for_app_ty)
    } else {
        None
    }
}

impl<'a> ParsedField<'a> {
    fn new(fn_arg: &'a FnArg) -> syn::Result<(Self, Option<(&'a Ident, &'a Type)>)> {
        let FnArg::Typed(PatType { attrs, pat, ty, .. }) = fn_arg else {
            return Err(syn::Error::new_spanned(
                fn_arg,
                "Command functions cannot have a `self` parameter",
            ));
        };
        let Type::Reference(ty) = &**ty else {
            return Err(syn::Error::new_spanned(
                ty,
                "Command function parameters must be references",
            ));
        };
        let Pat::Ident(PatIdent { ident: name, .. }) = &**pat else {
            return Err(syn::Error::new_spanned(
                pat,
                "Command function parameters must be simple identifiers",
            ));
        };
        let ctx = is_apply_context(ty).map(|for_app_ty| (name, for_app_ty));
        let (attrs, args) = utils::extract_vye_attrs(attrs)?;
        Ok((
            Self {
                args,
                attrs,
                name,
                ty,
            },
            ctx,
        ))
    }
}

impl<'a> ParsedField<'a> {
    fn generate_field(&self) -> Option<TokenStream> {
        if !matches!(self.args.kind(), FieldKind::Field) {
            return None;
        }
        let vis = self.args.vis.clone().unwrap_or(Visibility::Inherited);
        let name = self.name;
        let ty = &self.ty.elem;
        let attrs = &self.attrs;
        Some(quote! {
            #(#attrs)*
            #vis #name: #ty,
        })
    }

    fn generate_var_statement(&self, ctx_name: &Ident) -> TokenStream {
        let name = self.name;
        let ty = &self.ty.elem;
        match (self.args.kind(), self.ty.mutability.is_some()) {
            (FieldKind::Field, false) => quote! { let #name = &*#name; },
            (FieldKind::Field, true) => quote! { let mut #name = &mut *#name; },
            (FieldKind::State, false) => quote! { let #name = #ctx_name.state::<#ty>(); },
            (FieldKind::State, true) => {
                quote! { let mut #name = #ctx_name.state_mut::<#ty>(); }
            }
        }
    }
}

pub fn build(input: ItemFn, args: CommandArgs) -> syn::Result<TokenStream> {
    Ok(CommandContext::new(&input, args)?.expand())
}
