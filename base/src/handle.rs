mod spawner {
    pub trait GlobalSpawner {
        fn spawn_detached(fut: impl Future<Output = ()> + Send + 'static);
    }

    #[cfg(feature = "frb-compat")]
    pub enum FrbSpawner {}

    #[cfg(feature = "frb-compat")]
    impl GlobalSpawner for FrbSpawner {
        fn spawn_detached(fut: impl Future<Output = ()> + Send + 'static) {
            flutter_rust_bridge::spawn(fut);
        }
    }

    #[cfg(feature = "tokio")]
    pub enum TokioSpawner {}

    #[cfg(feature = "tokio")]
    impl GlobalSpawner for TokioSpawner {
        fn spawn_detached(fut: impl Future<Output = ()> + Send + 'static) {
            tokio::spawn(fut);
        }
    }
}

pub use spawner::GlobalSpawner;
use std::marker::PhantomData;

#[cfg(feature = "frb-compat")]
pub use spawner::FrbSpawner;

#[cfg(feature = "tokio")]
pub use spawner::TokioSpawner;

use crate::{Application, Host, HostBuilder};
use crate::{WrappedGetter, WrappedUpdater};

pub struct AppHandle<A: Application, WU, WG> {
    updater: WU,
    getter: WG,
    _app: PhantomData<A>,
}

impl<A, WU, WG> AppHandle<A, WU, WG>
where
    A: Application,
    WU: WrappedUpdater<Model = A::RootModel>,
    WG: WrappedGetter<Model = A::RootModel>,
{
    pub fn new<S: GlobalSpawner>(
        builder_fn: impl FnOnce(HostBuilder<A>) -> Host<A>,
    ) -> Self {
        let host = builder_fn(HostBuilder::new());
        let updater = host.updater();
        let getter = host.getter();
        S::spawn_detached(host.run());
        Self {
            updater: WU::__new(updater, crate::__token()),
            getter: WG::__new(getter, crate::__token()),
            _app: PhantomData,
        }
    }

    #[cfg(feature = "frb-compat")]
    pub fn new_frb(builder_fn: impl FnOnce(HostBuilder<A>) -> Host<A>) -> Self {
        Self::new::<FrbSpawner>(builder_fn)
    }

    #[cfg(feature = "tokio")]
    pub fn new_tokio(
        builder_fn: impl FnOnce(HostBuilder<A>) -> (Host<A>, ShouldRefreshSubscriber<A>),
    ) -> Self {
        Self::new::<TokioSpawner>(builder_fn)
    }
}

impl<A, WU, WG> AppHandle<A, WU, WG>
where
    A: Application,
    WU: WrappedUpdater<Model = A::RootModel>,
    WG: WrappedGetter<Model = A::RootModel>,
{
    pub fn updater(&self) -> WU {
        self.updater.clone()
    }

    pub fn getter(&self) -> WG {
        self.getter.clone()
    }
}
