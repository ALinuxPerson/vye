//! `#[vye::model]` macro implementation.
mod attr;

use crate::utils::{InterfaceImpl, MaybeStubFn};
use attr::ModelArgs;
use attr::raw::MethodArgs as RawMethodArgs;
pub use attr::raw::ModelArgs as RawModelArgs;
use darling::FromAttributes;
use proc_macro2::{Ident, TokenStream};
use syn::spanned::Spanned;
use syn::{
    Attribute, Block, FnArg, GenericParam, Pat, PatIdent, PatType, ReturnType, Signature, Type,
};

struct ModelContext<'a> {
    attrs: ModelArgs,
    items: Vec<ParsedFn<'a>>,
}

impl<'a> ModelContext<'a> {
    fn new(item: &'a InterfaceImpl, attrs: RawModelArgs) -> syn::Result<Self> {
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
        Ok(Self {
            attrs: ModelArgs::parse(attrs, model_name, ty_path.span())?,
            items: item
                .items
                .iter()
                .map(ParsedFn::new)
                .collect::<syn::Result<Vec<_>>>()?,
        })
    }

    fn expand(self) -> syn::Result<TokenStream> {
        todo!()
    }
}

struct ParsedFn<'a> {
    original: &'a MaybeStubFn,
    method_args: RawMethodArgs,
    fn_args: Vec<ParsedFnArg<'a>>,
    kind: FnKind,
}

impl<'a> ParsedFn<'a> {
    fn new(item: &'a MaybeStubFn) -> syn::Result<Self> {
        Ok(Self {
            original: item,
            method_args: RawMethodArgs::from_attributes(&item.attrs)?,
            kind: FnKind::analyze(&item.sig.ident, &item.sig, item.block.as_ref())?,
            fn_args: item
                .sig
                .inputs
                .iter()
                .flat_map(|i| ParsedFnArg::new(i).transpose())
                .collect::<syn::Result<Vec<_>>>()?,
        })
    }
}

enum FnKind {
    // fn new();
    New,

    // fn split() -> (_, _);
    Split,

    // fn updater(&mut self) {}
    Updater,

    // fn getter(&self) -> Ret [{} | ;]
    Getter,
}

impl FnKind {
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

    fn analyze(fn_name: &Ident, sig: &Signature, block: Option<&Block>) -> syn::Result<Self> {
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

        enum RetTy {
            InferredTwoTuple,
            Other,
        }

        impl RetTy {
            fn analyze(ty: &ReturnType) -> Option<Self> {
                match ty {
                    ReturnType::Default => None,
                    ReturnType::Type(_, ty) => {
                        if let Type::Tuple(tuple) = &**ty
                            && tuple.elems.len() == 2
                            && let (Some(first), Some(last)) =
                                (tuple.elems.first(), tuple.elems.last())
                            && matches!(first, Type::Infer(_))
                            && matches!(last, Type::Infer(_))
                        {
                            return Some(Self::InferredTwoTuple);
                        }

                        Some(Self::Other)
                    }
                }
            }
        }

        Self::validate(sig)?;
        let fn_name = fn_name.to_string();
        let (self_ty, ret_ty, block) = (
            SelfTy::analyze(sig.inputs.iter()),
            RetTy::analyze(&sig.output),
            block,
        );

        match (fn_name.as_str(), self_ty, ret_ty, block) {
            ("new", None, None, None) => Ok(Self::New),
            ("split", None, Some(RetTy::InferredTwoTuple), None) => Ok(Self::Split),
            (_, Some(SelfTy::Mutable), None, Some(_)) => Ok(Self::Updater),
            (_, Some(SelfTy::Shared), Some(_), _) => Ok(Self::Getter),
            _ => Err(syn::Error::new_spanned(
                sig,
                "could not determine function shape",
            )),
        }
    }
}

struct ParsedFnArg<'a> {
    attrs: &'a [Attribute],
    name: &'a Ident,
    ty: &'a Type,
}

impl<'a> ParsedFnArg<'a> {
    fn new(item: &'a FnArg) -> syn::Result<Option<Self>> {
        match item {
            FnArg::Receiver(_) => Ok(None),
            FnArg::Typed(PatType { attrs, pat, ty, .. }) => {
                if let Pat::Ident(PatIdent { ident: name, .. }) = &**pat {
                    Ok(Some(Self { attrs, name, ty }))
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

pub fn build(item: InterfaceImpl, attrs: RawModelArgs) -> syn::Result<TokenStream> {
    ModelContext::new(&item, attrs)?.expand()
}
