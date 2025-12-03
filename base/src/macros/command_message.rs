#[macro_export]
macro_rules! command {
    ($($tt:tt)*) => {
        $crate::__define_struct_and_delegate!(
            @impl $crate::__command_impl;
            $($tt)*
        );
    };
}

#[macro_export]
macro_rules! message {
    ($($tt:tt)*) => {
        $crate::__define_struct_and_delegate!(
            @impl $crate::__message_impl;
            $($tt)*
        );
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __define_struct_and_delegate {
    // 1. Named Fields: struct Name { ... }
    (
        @impl $callback:path;
        $(#[$meta:meta])*
        $vis:vis struct $Name:ident {
            $($fields:tt)*
        } for $Model:ty;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        $vis struct $Name {
            $($fields)*
        }
        $callback!($Name $Model; $($rest)*);
    };

    // 2. Tuple Struct: struct Name( ... );
    (
        @impl $callback:path;
        $(#[$meta:meta])*
        $vis:vis struct $Name:ident (
            $($fields:tt)*
        ) for $Model:ty;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        $vis struct $Name ( $($fields)* );
        $callback!($Name $Model; $($rest)*);
    };

    // 3. Unit Struct: struct Name;
    (
        @impl $callback:path;
        $(#[$meta:meta])*
        $vis:vis struct $Name:ident for $Model:ty;
        $($rest:tt)*
    ) => {
        $(#[$meta])*
        $vis struct $Name;
        $callback!($Name $Model; $($rest)*);
    };
}

// -----------------------------------------------------------------------------
// Specific Implementation Logic
// -----------------------------------------------------------------------------

#[macro_export]
#[doc(hidden)]
macro_rules! __command_impl {
    (
        $CommandName:ident $ModelName:ty;
        | $this:ident, $ctx:ident | $body:expr
    ) => {
        #[$crate::__macros::async_trait]
        impl $crate::Command for $CommandName {
            type ForApp = <$ModelName as $crate::Model>::ForApp;

            async fn apply(&mut self, ctx: &mut $crate::CommandContext<'_, Self::ForApp>) {
                async fn f(
                    $this: &mut $CommandName,
                    $ctx: &mut $crate::CommandContext<'_, <$CommandName as $crate::Command>::ForApp>,
                ) {
                    $body
                }
                f(self, ctx).await
            }
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __message_impl {
    (
        $MsgName:ident $ModelName:ty; $body:expr
    ) => {
        impl $crate::ModelMessage for $MsgName {}

        impl $crate::ModelHandler<$MsgName> for $ModelName {
            fn update(&mut self, message: $MsgName, ctx: &mut $crate::UpdateContext<<Self as $crate::Model>::ForApp>) {
                let f: fn(&mut $ModelName, $MsgName, &mut $crate::UpdateContext<<Self as $crate::Model>::ForApp>) = $body;
                f(self, message, ctx);
            }
        }
    };
}
