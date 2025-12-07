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

    pub extern crate futures;

    pub use async_trait::async_trait;

    #[cfg(feature = "frb-compat")]
    pub use flutter_rust_bridge::frb;
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
pub mod runtime;
pub mod handle;

pub use base::*;
pub use dispatcher::*;
pub use runtime::*;
pub use handle::*;

#[cfg(feature = "std")]
mod sync {
    pub type VRwLock<T> = std::sync::RwLock<T>;
    pub type VRWLockReadGuard<'a, T> = std::sync::RwLockReadGuard<'a, T>;
    pub type VRWLockWriteGuard<'a, T> = std::sync::RwLockWriteGuard<'a, T>;
    pub type VMutex<T> = std::sync::Mutex<T>;
    pub type VMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
}

#[cfg(not(feature = "std"))]
mod sync {
    pub type VRwLock<T> = spin::RwLock<T>;
    pub type VRWLockReadGuard<'a, T> = spin::RwLockReadGuard<'a, T>;
    pub type VRWLockWriteGuard<'a, T> = spin::RwLockWriteGuard<'a, T>;
    pub type VMutex<T> = spin::Mutex<T>;
    pub type VMutexGuard<'a, T> = spin::MutexGuard<'a, T>;
}

use sync::*;

macro_rules! cfg_unwrap {
    ($expr:expr) => {{
        #[cfg(feature = "std")]
        { $expr.unwrap() }
        #[cfg(not(feature = "std"))]
        { $expr }
    }};
}

fn read_vrwlock<T>(lock: &VRwLock<T>) -> VRWLockReadGuard<'_, T> {
    cfg_unwrap!(lock.read())
}

fn write_vrwlock<T>(lock: &VRwLock<T>) -> VRWLockWriteGuard<'_, T> {
    cfg_unwrap!(lock.write())
}

fn lock_mutex<T>(lock: &VMutex<T>) -> VMutexGuard<'_, T> {
    cfg_unwrap!(lock.lock())
}

#[doc(hidden)]
pub fn __token() -> __private::Token {
    __private::Token::new()
}
