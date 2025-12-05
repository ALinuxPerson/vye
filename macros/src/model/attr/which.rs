use super::raw;
use crate::model::attr::raw::{ProcessedMeta, ProcessedMetaRef};
use either::Either;
use proc_macro2::Ident;
use std::iter;

fn meta_or_empty<'a, I: Iterator<Item = ProcessedMetaRef<'a>>>(
    meta: &'a Option<raw::MetaConfig>,
    f: impl FnOnce(&'a raw::MetaConfig) -> I,
) -> impl Iterator<Item = ProcessedMetaRef<'a>> {
    match meta.as_ref() {
        Some(config) => Either::Left(f(config)),
        None => Either::Right(iter::empty()),
    }
}

pub trait With {
    const SUFFIX: &'static str;

    fn name(name: &raw::NameConfig) -> &Option<Ident>;
    fn outer_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>>;
    fn inner_meta_impl(meta: &raw::InnerMetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>>;
    fn fn_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>>;

    fn outer_meta(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta_or_empty(meta, Self::outer_meta_impl)
    }

    fn outer_meta_owned(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMeta> {
        Self::outer_meta(meta).map(ProcessedMetaRef::to_owned)
    }

    fn inner_meta(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        match meta.as_ref() {
            Some(raw::MetaConfig {
                inner: Some(config),
                ..
            }) => Either::Left(Self::inner_meta_impl(config)),
            _ => Either::Right(iter::empty()),
        }
    }

    fn inner_meta_owned(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMeta> {
        Self::inner_meta(meta).map(ProcessedMetaRef::to_owned)
    }

    fn fn_meta(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta_or_empty(meta, Self::fn_meta_impl)
    }

    fn fn_meta_owned(meta: &Option<raw::MetaConfig>) -> impl Iterator<Item = ProcessedMeta> {
        Self::fn_meta(meta).map(ProcessedMetaRef::to_owned)
    }
}

pub enum Dispatcher {}

impl With for Dispatcher {
    const SUFFIX: &'static str = "Dispatcher";

    fn name(name: &raw::NameConfig) -> &Option<Ident> {
        &name.dispatcher
    }

    fn outer_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.dispatcher()
    }

    fn inner_meta_impl(meta: &raw::InnerMetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.dispatcher()
    }

    fn fn_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.dispatcher_fn()
    }
}

pub enum Updater {}

impl With for Updater {
    const SUFFIX: &'static str = "Updater";

    fn name(name: &raw::NameConfig) -> &Option<Ident> {
        &name.updater
    }

    fn outer_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.updater()
    }

    fn inner_meta_impl(meta: &raw::InnerMetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.updater()
    }

    fn fn_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.updater_fn()
    }
}

pub enum Getter {}

impl With for Getter {
    const SUFFIX: &'static str = "Getter";

    fn name(name: &raw::NameConfig) -> &Option<Ident> {
        &name.getter
    }

    fn outer_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.getter()
    }

    fn inner_meta_impl(meta: &raw::InnerMetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.getter()
    }

    fn fn_meta_impl(meta: &raw::MetaConfig) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        meta.getter_fn()
    }
}
