use darling::ast::NestedMeta;
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Ident, TokenStream};
use quote::ToTokens;
use syn::Meta;

/// ```rust
/// #[vye::model(
///     /*
///     If passed, generates a wrapped dispatcher, updater, and getter struct. A corresponding
///     `fn split() -> (Updater, Getter)` will also be generated.
///
///     `#[vye::model]` must be applied to an `impl` block: `pub impl Model { /* ... */ }`
///
///     The visibility of the `impl` block can be declared. This affects the visibility of the
///     generated dispatcher, updater, and getter structs.
///     */
///
///     // Generates a dispatcher with default settings. Generated structs will be named according
///     // to the name of their model. E.g. if the model is `FooModel`, the structs will be named
///     // `FooDispatcher`, `FooUpdater`, and `FooGetter`.
///     dispatcher,
///
///     // More customizability options
///     dispatcher(
///         // Name config
///         name(
///             dispatcher = "FooDispatcher", // explicitly specifies the name of the dispatcher
///             updater = "FooUpdater",       // explicitly specifies the name of the updater
///             getter = "FooGetter",         // explicitly specifies the name of the getter
///         ),
///
///         // Attributes config
///         meta(
///             // These attributes operate on the outer struct:
///             // `#[derive(Debug)] pub struct FooDispatcher(vye::Dispatcher<FooModel>);`
///
///             // Common attributes for all generated structs
///             base(derive(Debug)),
///             // `base`, `dispatcher`, `updater`, and `getter` can be specified multiple times
///             base(derive(PartialOrd)),
///             dispatcher(derive(Clone)),     // attributes for the generated `dispatcher` struct
///             updater(derive(PartialEq)),    // attributes for the generated `updater` struct
///             getter(derive(Serialize)),     // attributes for the generated `getter` struct
///
///             // These attributes operate on the inner value:
///             // `pub struct FooDispatcher(#[foo] vye::Dispatcher<FooModel>);`
///             inner(
///                 dispatcher(foo), // inner attributes for the generated `dispatcher` struct
///                 // `dispatcher`, `updater`, and `getter` can be specified multiple times
///                 dispatcher(bar),
///                 updater(baz),    // inner attributes for the generated `updater` struct
///                 getter(qux),     // inner attributes for the generated `getter` struct
///             ),
///         ),
///     ),
/// )]
/// pub(crate) impl FooModel {
///     // The visibility of this function determines the visibility of the generated `new`
///     // functions for the dispatcher, getter, and updater structs.
///     #[vye(
///         // Attributes config:
///         // `#[some_meta] fn new(dispatcher: vye::Dispatcher<FooModel>) -> { /* ... */ }`
///         meta(
///             // Common attributes of the `new` function for all generated structs
///             base(foo),
///             // `base`, `dispatcher`, `updater`, and `getter` can be specified multiple times
///             base(bar),
///             dispatcher(baz), // attributes for the generated `dispatcher` struct
///             updater(qux),    // attributes for the generated `updater` struct
///             getter(quux),    // attributes for the generated `getter` struct
///         ),
///     )]
///     pub fn new();
///
///     /// Any attributes on this function will be applied to the `split` function on the
///     /// dispatcher struct.
///     pub fn split();
///
///     // An updater function.
///     // The function must follow this shape:
///     // `$vis fn $fn_name[<T>](
///     //    &mut self,
///     //    field: i32,
///     //    [, generic: T,]
///     //    [, ctx: &mut UpdateContext<App>,]
///     //  )
///     // { /* ... */ }
///     // Meaning:
///     // - The header can only be the visibility followed by `fn`. No `async`, `const`, etc.
///     // - While type generics _are_ allowed, lifetimes are NOT allowed.
///     // - The `ctx` argument can be omitted for brevity.
///     // - The function must not return anything (void).
///     //
///     // The visibility of the function determines the visibility of the generated message struct
///     // and its visibility on the dispatcher and updater structs.
///     //
///     // Function names for the dispatcher and updater structs will inherit the name of this
///     // function.
///     #[vye(
///         // Name config. If not passed, the message name will be the function name converted
///         // to PascalCase with "Message" appended to it. For example, `set_name` becomes
///         // `SetNameMessage`.
///         message = "SetNameMessage",
///
///         // Attributes config, these can be specified multiple times:
///         // `#[some_meta] fn set_name(&mut self, message: SetNameMessage) -> { /* ... */ }`
///         meta(
///             message(derive(Clone)), // outer attributes for the message struct
///             fns(foo),               // common attributes for the dispatcher and updater functions
///             dispatcher(bar),        // attributes for the dispatcher function
///             updater(baz),           // attributes for the updater function
///         ),
///     )]
///     pub(crate) fn set_name(
///         &mut self,
///
///         // Any attributes declared on these arguments are pasted on the fields of the message
///         // struct
///         #[serde(rename = "pangalan")]
///         name: String,
///         ctx: &mut UpdateContext<MyCoolApp>, // this can be omitted if not used
///     ) {
///         self.name = name;
///         ctx.mark_dirty(Region::Root);
///     }
///
///     // A getter function.
///     // The function must follow this shape:
///     // `$vis fn $fn_name[<T>](
///     //    &self,
///     //    field: i32,
///     //    [, generic: T,]
///     //  ) -> ReturnType
///     // [{ /* ... */ } | ;]
///     // Meaning:
///     // - The header can only be the visibility followed by `fn`. No `async`, `const`, etc.
///     // - While type generics _are_ allowed, lifetimes are NOT allowed.
///     // - The function must return a value.
///     // - The function body CAN be omitted if there are no other arguments, the function name
///     //   corresponds to a field in the model, and the type of the field implements Clone.
///     //
///     // The visibility of the function determines the visibility of the generated message struct
///     // and its visibility on the dispatcher and getter structs.
///     //
///     // Function names for the dispatcher and getter structs will inherit the name of this
///     // function.
///     #[vye(
///         // Name config. If not passed, the message name will be the function name converted
///         // to PascalCase with "Get" and "Message" as its prefix and suffix respectively.
///         // For example, `location` becomes `GetLocationMessage`.
///         message = "GetLocationMessage",
///
///         // Attributes config, these can be specified multiple times:
///         // `#[some_meta] fn location(&self, message: GetLocationMessage) -> String { /* ... */ }`
///         meta(
///             message(derive(Clone)), // outer attributes for the message struct
///             fns(foo),               // common attributes for the dispatcher and getter functions
///             dispatcher(bar),        // attributes for the dispatcher function
///             getter(baz),            // attributes for the getter function
///         ),
///     )]
///     pub(super) fn location(
///         &self,
///         // Any attributes declared on these arguments are pasted on the fields of the message
///         // struct
///         #[serde(rename = "makeLowercase")]
///         make_lowercase: bool,
///     ) -> String {
///         if make_lowercase {
///             self.location.to_lowercase()
///         } else {
///             self.location.clone()
///         }
///     }
///
///     // This is equivalent to `Clone::clone(&self.name)` because the body is omitted.
///     fn name(&self) -> String;
/// }
/// ```
#[derive(FromMeta)]
#[darling(derive_syn_parse)]
pub struct ModelArgs {
    #[darling(default)]
    pub dispatcher: Option<DispatcherDef>,
}

#[derive(FromAttributes)]
#[darling(attributes(vye))]
pub struct MethodArgs {
    #[darling(default)]
    pub name: Option<NameConfig>,

    #[darling(default)]
    pub message: Option<Ident>,

    #[darling(default)]
    pub meta: Option<MetaConfig>,
}

pub enum DispatcherDef {
    Default,
    Config(Box<DispatcherConfig>),
}

impl DispatcherDef {
    pub fn into_config(self) -> DispatcherConfig {
        match self {
            Self::Default => DispatcherConfig::default(),
            Self::Config(config) => *config,
        }
    }
}

impl FromMeta for DispatcherDef {
    // #[vye::model(dispatcher)]
    fn from_word() -> darling::Result<Self> {
        Ok(Self::Default)
    }

    // #[vye::model(dispatcher(...))]
    fn from_list(items: &[NestedMeta]) -> darling::Result<Self> {
        Ok(Self::Config(Box::new(DispatcherConfig::from_list(items)?)))
    }
}

#[derive(FromMeta, Default)]
pub struct DispatcherConfig {
    #[darling(default)]
    pub name: Option<NameConfig>,

    #[darling(default)]
    pub meta: Option<MetaConfig>,
}

#[derive(FromMeta, Default)]
pub struct NameConfig {
    #[darling(default)]
    pub dispatcher: Option<Ident>,

    #[darling(default)]
    pub updater: Option<Ident>,

    #[darling(default)]
    pub getter: Option<Ident>,
}

pub struct ProcessedMetaRef<'a>(&'a TokenStream);

impl<'a> ProcessedMetaRef<'a> {
    fn process(meta: &'a Meta) -> Self {
        // strip out the outer `meta(...)`
        Self(
            &meta
                .require_list()
                .expect("should always be a` MetaList`")
                .tokens,
        )
    }
    
    pub(crate) fn to_owned(self) -> ProcessedMeta {
        ProcessedMeta(self.0.clone())
    }
}

impl<'a> ToTokens for ProcessedMetaRef<'a> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.to_tokens(tokens);
    }
}

pub struct ProcessedMeta(TokenStream);

impl ToTokens for ProcessedMeta {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.to_tokens(tokens);
    }
}

#[derive(FromMeta)]
pub struct MetaConfig {
    #[darling(multiple)]
    pub base: Vec<Meta>,

    #[darling(multiple)]
    pub dispatcher: Vec<Meta>,

    #[darling(multiple)]
    pub updater: Vec<Meta>,

    #[darling(multiple)]
    pub getter: Vec<Meta>,

    #[darling(multiple)]
    pub message: Vec<Meta>,

    #[darling(multiple)]
    pub fns: Vec<Meta>,

    #[darling(default)]
    pub inner: Option<InnerMetaConfig>,
}

impl MetaConfig {
    fn field_with(
        &self,
        f: impl FnOnce(&Self) -> &Vec<Meta>,
    ) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.base
            .iter()
            .chain(f(self))
            .map(ProcessedMetaRef::process)
    }

    pub fn dispatcher(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.field_with(|m| &m.dispatcher)
    }

    pub fn updater(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.field_with(|m| &m.updater)
    }

    pub fn getter(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.field_with(|m| &m.getter)
    }

    pub fn message(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.field_with(|m| &m.message)
    }
}

impl MetaConfig {
    pub fn fns(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.field_with(|m| &m.fns)
    }

    pub fn dispatcher_fn(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.fns()
            .chain(self.dispatcher.iter().map(ProcessedMetaRef::process))
    }

    pub fn updater_fn(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.fns()
            .chain(self.updater.iter().map(ProcessedMetaRef::process))
    }

    pub fn getter_fn(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.fns()
            .chain(self.getter.iter().map(ProcessedMetaRef::process))
    }
}

#[derive(FromMeta)]
pub struct InnerMetaConfig {
    #[darling(multiple)]
    pub dispatcher: Vec<Meta>,

    #[darling(multiple)]
    pub updater: Vec<Meta>,

    #[darling(multiple)]
    pub getter: Vec<Meta>,
}

impl InnerMetaConfig {
    pub fn dispatcher(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.dispatcher
            .iter()
            .map(ProcessedMetaRef::process)
    }
    
    pub fn updater(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.updater.iter().map(ProcessedMetaRef::process)
    }
    
    pub fn getter(&self) -> impl Iterator<Item = ProcessedMetaRef<'_>> {
        self.getter.iter().map(ProcessedMetaRef::process)
    }
}
