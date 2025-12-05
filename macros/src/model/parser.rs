use crate::crate_;
use crate::model::attr::{ModelArgs, NewMethodArgs, UpdaterGetterMethodArgs, raw};
use crate::model::{
    FnKind, ModelContext, ParsedFnArg, ParsedGetterFn, ParsedNewFn, ParsedNewSplitFn,
    ParsedSplitFn, ParsedUpdaterFn, ParsedUpdaterGetterFn, RawModelArgs,
};
use crate::utils::{InterfaceImpl, MaybeStubFn};
use darling::FromAttributes;
use proc_macro2::Ident;
use syn::spanned::Spanned;
use syn::{
    AngleBracketedGenericArguments, FnArg, GenericArgument, GenericParam, Pat, PatIdent, PatType,
    Path, PathArguments, PathSegment, ReturnType, Signature, Type, TypePath, TypeReference,
    Visibility,
};

fn is_ty_update_context(ty: &Type) -> bool {
    // &...
    if let Type::Reference(TypeReference {
        mutability, elem, ..
    }) = ty
        // &mut ...
        && mutability.is_some()
        // &mut is::a::path ...
        && let Type::Path(TypePath {
            path: Path { segments, .. },
            ..
        }) = &**elem
        // &mut [...]path<...?>
        && let Some(PathSegment {
            ident: ty_ident,
            arguments:
                PathArguments::AngleBracketed(AngleBracketedGenericArguments {
                    args: generic_args, ..
                }),
        }) = segments.last()
        // &mut UpdateContext<...?>
        && ty_ident == "UpdateContext"
        // &mut UpdateContext<...>
        && !generic_args.is_empty()
        // &mut UpdateContext<_, _> or &mut UpdateContext<_>
        && generic_args.len() <= 2
    {
        if generic_args.len() == 2 {
            matches!(
                (generic_args.first(), generic_args.last()),
                // UpdateContext<'_, MyApp>
                (
                    Some(GenericArgument::Lifetime(_)),
                    Some(GenericArgument::Type(_))
                ),
            )
        } else if generic_args.len() == 1 {
            matches!(
                generic_args.first(),
                // UpdateContext<MyApp>
                Some(GenericArgument::Type(_))
            )
        } else {
            false
        }
    } else {
        false
    }
}

impl<'a> ModelContext<'a> {
    pub(super) fn parse(item: &'a InterfaceImpl, attrs: RawModelArgs) -> syn::Result<Self> {
        let Type::Path(ty_path) = &item.self_ty else {
            return Err(syn::Error::new_spanned(
                &item.self_ty,
                "`#[vye::model]` can only be applied to impl blocks for named types",
            ));
        };
        let model_name = &ty_path
            .path
            .segments
            .last()
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &item.self_ty,
                    "`#[vye::model]` can only be applied to impl blocks for named types",
                )
            })?
            .ident;
        let items = item
            .items
            .iter()
            .map(ParsedFnFirstPass::parse)
            .collect::<syn::Result<Vec<_>>>()?;
        let ParsedFnsSecondPass {
            new_fn,
            split_fn,
            updaters,
            getters,
        } = ParsedFnsSecondPass::parse(items);
        Ok(Self {
            crate_: crate_(),
            args: ModelArgs::parse(attrs, model_name, ty_path.span())?,
            struct_vis: &item.vis,
            model_ty: ty_path,
            new_fn,
            split_fn,
            updaters,
            getters,
        })
    }
}

struct ParsedFnFirstPass<'a> {
    vis: &'a Visibility,
    fn_args: Vec<ParsedFnArg<'a>>,
    kind: FnKind<'a>,
}

impl<'a> ParsedFnFirstPass<'a> {
    fn parse(item: &'a MaybeStubFn) -> syn::Result<Self> {
        let args = raw::MethodArgs::from_attributes(&item.attrs)?;
        let mut kind = FnKind::analyze(item, args)?;
        Ok(Self {
            vis: &item.vis,
            fn_args: item
                .sig
                .inputs
                .iter()
                .flat_map(|i| {
                    ParsedFnArg::parse(i)
                        .map(|ret| ret.and_then(|r| r.fill_fn_kind(&mut kind)))
                        .transpose()
                })
                .collect::<syn::Result<Vec<_>>>()?,
            kind,
        })
    }
}

impl<'a> FnKind<'a> {
    fn validate(sig: &Signature) -> syn::Result<()> {
        if sig.constness.is_some() {
            return Err(syn::Error::new_spanned(
                sig,
                "const functions are not supported in `#[vye::model]`",
            ));
        }

        if sig.asyncness.is_some() {
            return Err(syn::Error::new_spanned(
                sig,
                "async functions are not supported in `#[vye::model]`",
            ));
        }

        if sig.unsafety.is_some() {
            return Err(syn::Error::new_spanned(
                sig,
                "unsafe functions are not supported in `#[vye::model]`",
            ));
        }

        if sig.abi.is_some() {
            return Err(syn::Error::new_spanned(
                sig,
                "extern functions are not supported in `#[vye::model]`",
            ));
        }

        for param in &sig.generics.params {
            if let GenericParam::Lifetime(_) = param {
                return Err(syn::Error::new_spanned(
                    param,
                    "lifetime parameters are not supported in `#[vye::model]`",
                ));
            }
        }

        if sig.variadic.is_some() {
            return Err(syn::Error::new_spanned(
                sig,
                "variadic functions are not supported in `#[vye::model]`",
            ));
        }

        Ok(())
    }

    fn analyze(item: &'a MaybeStubFn, args: raw::MethodArgs) -> syn::Result<Self> {
        enum SelfTy {
            Shared,
            Mutable,
        }

        impl SelfTy {
            fn analyze<'a>(args: impl Iterator<Item = &'a FnArg>) -> Option<Self> {
                for arg in args {
                    if let FnArg::Receiver(receiver) = arg
                        && receiver.reference.is_some()
                    {
                        return if receiver.mutability.is_some() {
                            Some(Self::Mutable)
                        } else {
                            Some(Self::Shared)
                        };
                    }
                }

                None
            }
        }

        Self::validate(&item.sig)?;
        let fn_name = item.sig.ident.to_string();
        let (self_ty, ret_ty, block) = (
            SelfTy::analyze(item.sig.inputs.iter()),
            &item.sig.output,
            item.block.as_ref(),
        );

        match (fn_name.as_str(), self_ty, ret_ty, block) {
            ("new", None, ReturnType::Default, None) => {
                Ok(Self::New(NewMethodArgs::parse(args, item.sig.span())?))
            }
            ("split", None, ReturnType::Default, None) => Ok(Self::Split(&item.attrs)),
            (_, Some(SelfTy::Mutable), ReturnType::Default, Some(block)) => Ok(Self::Updater {
                args: UpdaterGetterMethodArgs::parse_updater(
                    args,
                    &item.sig.ident,
                    item.sig.span(),
                )?,
                ctx: None, // ctx ident will be found in FnArgs parsing
                block,
            }),
            (_, Some(SelfTy::Shared), ReturnType::Type(_, ty), block) => Ok(Self::Getter {
                args: UpdaterGetterMethodArgs::parse_getter(
                    args,
                    &item.sig.ident,
                    item.sig.span(),
                )?,
                ty,
                block,
            }),
            _ => Err(syn::Error::new_spanned(
                &item.sig,
                "could not determine function shape",
            )),
        }
    }
}

impl<'a> FnKind<'a> {
    fn fill_update_context(&mut self, ctx: &'a Ident) {
        if let FnKind::Updater { ctx: ctx_field, .. } = self {
            *ctx_field = Some(ctx);
        }
    }
}

enum MaybeParsedFnArg<'a> {
    Is(ParsedFnArg<'a>),
    UpdateContext(&'a Ident),
}

impl<'a> MaybeParsedFnArg<'a> {
    fn fill_fn_kind(self, kind: &mut FnKind<'a>) -> Option<ParsedFnArg<'a>> {
        match self {
            MaybeParsedFnArg::Is(fn_arg) => Some(fn_arg),
            MaybeParsedFnArg::UpdateContext(ident) => {
                kind.fill_update_context(ident);
                None
            }
        }
    }
}

impl<'a> ParsedFnArg<'a> {
    fn parse(item: &'a FnArg) -> syn::Result<Option<MaybeParsedFnArg<'a>>> {
        match item {
            FnArg::Receiver(_) => Ok(None),
            FnArg::Typed(PatType { attrs, pat, ty, .. }) => {
                if let Pat::Ident(PatIdent { ident: name, .. }) = &**pat {
                    if is_ty_update_context(ty) {
                        Ok(Some(MaybeParsedFnArg::UpdateContext(name)))
                    } else {
                        Ok(Some(MaybeParsedFnArg::Is(Self { attrs, name, ty })))
                    }
                } else {
                    Err(syn::Error::new_spanned(
                        item,
                        "unsupported function argument in `#[vye::model]`",
                    ))
                }
            }
        }
    }
}

struct ParsedFnsSecondPass<'a> {
    new_fn: ParsedNewFn,
    split_fn: ParsedSplitFn<'a>,
    updaters: Vec<ParsedUpdaterFn<'a>>,
    getters: Vec<ParsedGetterFn<'a>>,
}

impl<'a> ParsedFnsSecondPass<'a> {
    fn parse(items: Vec<ParsedFnFirstPass<'a>>) -> Self {
        let mut new_fn = ParsedNewFn::default();
        let mut split_fn = ParsedSplitFn::default();
        let mut updaters = Vec::with_capacity(items.len());
        let mut getters = Vec::with_capacity(items.len());

        for item in items {
            match item.kind {
                FnKind::New(method_args) => {
                    new_fn = ParsedNewFn(ParsedNewSplitFn {
                        vis: item.vis.clone(),
                        method_args,
                    })
                }
                FnKind::Split(attrs) => {
                    split_fn = ParsedSplitFn {
                        vis: item.vis.clone(),
                        attrs,
                    }
                }
                FnKind::Updater {
                    args: method_args,
                    ctx,
                    block,
                } => updaters.push(ParsedUpdaterFn {
                    common: ParsedUpdaterGetterFn {
                        vis: item.vis,
                        method_args,
                        fn_args: item.fn_args,
                    },
                    ctx,
                    block,
                }),
                FnKind::Getter {
                    args: method_args,
                    ty,
                    block,
                } => getters.push(ParsedGetterFn {
                    common: ParsedUpdaterGetterFn {
                        vis: item.vis,
                        method_args,
                        fn_args: item.fn_args,
                    },
                    block,
                    ret_ty: ty,
                }),
            }
        }

        updaters.shrink_to_fit();
        getters.shrink_to_fit();

        Self {
            new_fn,
            split_fn,
            updaters,
            getters,
        }
    }
}
