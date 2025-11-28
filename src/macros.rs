#[macro_export]
macro_rules! impl_child_parent_model_getter_relationship {
    ($parent:ident: $child:ident) => {
        impl $crate::ChildModelGetter for $child {
            type Parent = $parent;

            fn parent(&self) -> &Self::Parent {
                &self.0
            }
        }

        impl $crate::ParentModelGetter<$child> for $parent {}
    };
}

#[macro_export]
macro_rules! impl_child_parent_model_relationship {
    ($parent:ident: $child:ident) => {
        impl $crate::ChildModel for $child {
            type Parent = $parent;
        }

        impl $crate::ParentModel<$child> for $parent {}
    };
}

#[macro_export]
macro_rules! make_child_model_getter {
    (
        type Model = $model:ty;
        variant ParentMessage = $parent_message_variant:ident;

        $(#[$($message_meta:meta)*])*
        $message_vis:vis enum $message:ident {
            $($(#[$($variant_meta:meta)*])*
            $variant:ident($variant_ty:ty) =
            $(#[$($variant_fn_meta:meta)*])*
            $variant_fn:ident
            $(,)?)*
        }

        $(#[$($model_getter_meta:meta)*])*
        $model_getter_vis:vis struct $model_getter:ident($parent_model_getter_vis:vis $parent_model_getter:ty);
    ) => {
        $(#[$($message_meta)*])*
        $message_vis enum $message {
            $(
            $(#[$($variant_meta)*])*
            $variant(::futures::channel::oneshot::Sender<$variant_ty>),
            )*
        }

        impl ::core::convert::From<$message> for <$parent_model_getter as $crate::ModelGetter>::Message {
            fn from(value: $message) -> Self {
                Self::$parent_message_variant(value)
            }
        }

        $(#[$($model_getter_meta)*])*
        $model_getter_vis struct $model_getter($parent_model_getter_vis $parent_model_getter);

        impl $model_getter {
            $(
            $(#[$($variant_fn_meta)*])*
            pub async fn $variant_fn(&self) -> ::core::result::Result<$variant_ty, $crate::Error> {
                $crate::SendableModelGetter::send(self, $message::$variant).await
            }
            )*
        }

        impl $crate::ModelGetter for $model_getter {
            type Model = $model;
            type Message = $message;
        }

        impl $crate::ChildModelGetter for $model_getter {
            type Parent = $parent_model_getter;

            fn parent(&self) -> &Self::Parent {
                &self.0
            }
        }

        impl $crate::ParentModelGetter<$model_getter> for $parent_model_getter {}
    };
}

#[macro_export]
macro_rules! define_command {
    (
        $(#[$meta:meta])*
        $name:ident($($type:ty),* $(,)?) for $app:ty;
        fn($self:ident, $ctx:ident) $body:block
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $name($($type),*);

        #[::async_trait::async_trait]
        impl $crate::Command for $name {
            type ForApp = $app;

            async fn apply(&mut self, $ctx: &mut ExecContext<'_, Self::ForApp>) {
                let $self  = self;
                $body
            }
        }
    };
}

#[cfg(feature = "frb-compat")]
#[doc(hidden)]
pub mod __app_wrapper_imports {
    pub extern crate anyhow;
    pub extern crate flutter_rust_bridge;
}

#[cfg(feature = "frb-compat")]
#[macro_export]
macro_rules! make_app_wrapper_for_frb {
    (
        $app_wrapper:ident for $app_ty:ty;
        type Dispatcher = $dispatcher:ident;
        type ModelGetter = $model_getter:ty;
        type StreamSink = $stream_sink:ident;
        type RegionId = $region_id:ty;
    ) => {
        pub struct $app_wrapper {
            runtime: $crate::MvuRuntime<$app_ty>,
            should_refresh_subscriber:
                ::core::option::Option<$crate::ShouldRefreshSubscriber<$app_ty>>,
        }

        impl $app_wrapper {
            pub async fn run(self) {
                $crate::__app_wrapper_imports::flutter_rust_bridge::spawn(self.runtime.run());
            }

            #[$crate::__app_wrapper_imports::flutter_rust_bridge::frb(sync, getter)]
            pub fn message_dispatcher(&self) -> $dispatcher {
                $dispatcher(self.runtime.message_dispatcher())
            }

            #[$crate::__app_wrapper_imports::flutter_rust_bridge::frb(sync, getter)]
            pub fn model_getter(&self) -> $model_getter {
                const _: ::core::option::Option<
                    <<$app_ty as $crate::Application>::RootModel as $crate::Model>::Getter,
                > = ::core::option::Option::<$model_getter>::None;
                self.runtime.model_getter()
            }

            pub async fn should_refresh(
                &mut self,
                sink: $stream_sink<$region_id>,
            ) -> $crate::__app_wrapper_imports::anyhow::Result<()> {
                const _: ::core::option::Option<<$app_ty as $crate::Application>::RegionId> =
                    ::core::option::Option::<$region_id>::None;
                let mut subscriber = self
                    .should_refresh_subscriber
                    .take()
                    .expect("`should_refresh` called more than once");
                $crate::__app_wrapper_imports::flutter_rust_bridge::spawn(async move {
                    while let Some(region_id) = subscriber.recv().await {
                        sink.add(region_id).ok();
                    }
                });
                Ok(())
            }
        }

        #[derive(Clone)]
        pub struct $dispatcher($crate::MessageDispatcher<$app_ty>);
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __make_delegator_model_common {
    (
        $(#[$($model_meta:meta)*])* $model:ident; $for_app:ty; $(#[$($message_meta:meta)*])*
        $message:ident; $(#[$($dispatcher_meta:meta)*])* $dispatcher:ident; $dispatcher_inner:ty;
        $(#[$($model_getter_meta:meta)*])* $model_getter:ident; $model_getter_inner:ty;
        $(#[$($model_getter_message_meta:meta)*])*
        $model_getter_message:ident;
        $(
            $d_model:ty; $d_base:ident; $d_base_snake:ident;
        )*
    ) => {
        #[derive(Debug)]
        $(#[$($message_meta)*])*
        pub enum $message {
            $($d_base(<$d_model as $crate::Model>::Message),)*
        }

        #[derive(Clone)]
        $(#[$($dispatcher_meta)*])*
        pub struct $dispatcher($dispatcher_inner);

        $(#[$($model_getter_message_meta)*])*
        pub enum $model_getter_message {
            $($d_base(<<$d_model as $crate::Model>::Getter as $crate::ModelGetter>::Message),)*
        }

        #[derive(Clone)]
        $(#[$($model_getter_meta)*])*
        pub struct $model_getter($model_getter_inner);

        impl $crate::ModelGetter for $model_getter {
            type Model = $model_getter;
            type Message = $model_getter_message;
        }

        $(#[$($model_meta)*])*
        pub struct $model {
            $($d_base_snake: $d_model,)*
        }

        impl $crate::Model for $model {
            type ForApp = $for_app;
            type Getter = $model_getter;
            type Message = $message;
            type UpdateRet = ();

            fn update(&mut self, message: Self::Message, ctx: &mut $crate::UpdateContext<Self::ForApp>) -> Self::UpdateRet {
                match message {
                    $($message::$d_base(message) => self.$d_base_snake.update(message, ctx),)*
                }
            }

            fn getter(&self, message: <Self::Getter as $crate::ModelGetter>::Message) {
                match message {
                    $($model_getter_message::$d_base(message) => self.$d_base_snake.getter(message),)*
                }
            }
        }
    };
}

#[cfg(not(feature = "frb-compat"))]
#[macro_export]
macro_rules! make_delegator_model {
    (
        $(#[$($model_meta:meta)*])* type Model = $model:ident;
        type ForApp = $for_app:ty;
        $(#[$($message_meta:meta)*])* type Message = $message:ident;
        $(#[$($model_getter_meta:meta)*])* type ModelGetter = $model_getter:ident: $model_getter_inner:ty;
        $(#[$($model_getter_message_meta:meta)*])* type ModelGetterMessage = $model_getter_message:ident;

        $(
        $d_model:ty => $d_base:ident, $d_base_snake:ident, $d_message:ty, $d_model_getter_message:ty $(,)?
        )*
    ) => {
        $crate::__make_delegator_model_common! {
            $(#[$($model_meta)*])* $model; $for_app; $(#[$($message_meta)*])* $message;
            $(#[$($model_getter_meta)*])* $model_getter; $(#[$($model_getter_message_meta)*])*
            $model_getter_message;
            $($d_model; $d_base; $d_base_snake; $d_message; $d_model_getter_message;)*
        }
    };
}

#[cfg(feature = "frb-compat")]
#[macro_export]
macro_rules! make_delegator_model {
    (
        $(#[$($model_meta:meta)*])* type Model = $model:ident;
        type ForApp = $for_app:ty;
        $(#[$($message_meta:meta)*])* type Message = $message:ident;
        $(#[$($dispatcher_meta:meta)*])* type Dispatcher = $dispatcher:ident: $dispatcher_inner:ty;
        $(#[$($model_getter_meta:meta)*])* type ModelGetter = $model_getter:ident: $model_getter_inner:ty;
        $(#[$($model_getter_message_meta:meta)*])* type ModelGetterMessage = $model_getter_message:ident;

        $(
        $d_model:ty => {
            ident Base = $d_base:ident;
            ident BaseSnake = $d_base_snake:ident;
            type Dispatcher = $d_dispatcher:ty;
            Dispatcher fn($d_dispatcher_self:ident) $d_dispatcher_body:expr;
            type ModelGetter = $d_model_getter:ty;
            ModelGetter fn($d_mg_self:ident) $d_mg_body:expr $(;)?
        } $(,)?
        )*
    ) => {
        $crate::__make_delegator_model_common! {
            $(#[$($model_meta)*])* $model; $for_app; $(#[$($message_meta)*])* $message;
            $(#[$($dispatcher_meta)*])* $dispatcher; $dispatcher_inner;
            $(#[$($model_getter_meta)*])* $model_getter; $model_getter_inner;
            $(#[$($model_getter_message_meta)*])* $model_getter_message;
            $($d_model; $d_base; $d_base_snake;)*
        }

        impl $dispatcher {
            $(
            pub fn $d_base_snake(&self) -> $d_dispatcher {
                let $d_dispatcher_self = self;
                $d_dispatcher_body
            }
            )*
        }

        impl $model_getter {
            $(
            pub fn $d_base_snake(&self) -> $d_model_getter {
                let $d_mg_self = self;
                $d_mg_body
            }
            )*
        }
    };
}
