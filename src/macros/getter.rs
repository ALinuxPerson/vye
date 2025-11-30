#[macro_export]
macro_rules! getter {
    (
        $(#[$meta:meta])*
        $vis:vis struct $MsgName:ident {
            $($fields:tt)*
        } for $ModelName:ty where Data = $Data:ty;
        $body:expr
    ) => {
        $(#[$meta])* $vis struct $MsgName { $($fields)* }
        $crate::__getter_impl!($MsgName $ModelName; $Data; $body);
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $MsgName:ident (
            $($fields:tt)*
        ) for $ModelName:ty where Data = $Data:ty;
        $body:expr
    ) => {
        $(#[$meta])* $vis struct $MsgName ( $($fields)* );
        $crate::__getter_impl!($MsgName $ModelName; $Data; $body);
    };

    (
        $(#[$meta:meta])*
        $vis:vis struct $MsgName:ident for $ModelName:ty where Data = $Data:ty;
        $body:expr
    ) => {
        $(#[$meta])* $vis struct $MsgName;
        $crate::__getter_impl!($MsgName $ModelName; $Data; $body);
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __getter_impl {
    (
        $MsgName:ident $ModelName:ty;
        $Data:ty;
        $body:expr
    ) => {
        impl $crate::ModelGetterMessage for $MsgName {
            type Data = $Data;
        }

        impl $crate::ModelGetterHandler<$MsgName> for $ModelName {
            fn getter(&self, message: $MsgName) -> <$MsgName as $crate::ModelGetterMessage>::Data {
                let f: fn(&$ModelName, $MsgName) -> <$MsgName as $crate::ModelGetterMessage>::Data = $body;
                f(self, message);
            }
        }
    };
}
