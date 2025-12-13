#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

#[macro_use]
pub mod macros;

#[doc(hidden)]
pub mod __macros {
    #[cfg(feature = "frb-compat")]
    pub extern crate flutter_rust_bridge;

    #[cfg(feature = "frb-compat")]
    pub extern crate anyhow;

    pub extern crate alloc;
    pub extern crate futures;

    pub use async_trait::async_trait;

    #[cfg(feature = "frb-compat")]
    pub use flutter_rust_bridge::frb;

    pub use crate::FlushSignals;
    pub use crate::maybe::Shared;
}

#[doc(hidden)]
pub mod __private {
    pub trait Sealed {}

    pub struct Token(());

    impl Token {
        pub const fn new() -> Self {
            Self(())
        }
    }

    impl Default for Token {
        fn default() -> Self {
            Self::new()
        }
    }
}

pub mod base;
pub mod dispatcher;
mod maybe;
pub mod host;

#[cfg(feature = "thread-safe")]
pub mod handle;

pub use base::*;
pub use dispatcher::*;
pub use host::*;

#[cfg(feature = "thread-safe")]
pub use handle::*;

#[doc(hidden)]
pub fn __token() -> __private::Token {
    __private::Token::new()
}
