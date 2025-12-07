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

use crate::{Application, MvuRuntime, MvuRuntimeBuilder, WrappedDispatcher};

pub struct AppHandle<A: Application, WD> {
    dispatcher: WD,
    _app: PhantomData<A>,
}

impl<A, WD> AppHandle<A, WD>
where
    A: Application,
    WD: WrappedDispatcher<Model = A::RootModel>,
{
    pub fn new<S: GlobalSpawner>(
        builder_fn: impl FnOnce(MvuRuntimeBuilder<A>) -> MvuRuntime<A>,
    ) -> Self {
        let runtime = builder_fn(MvuRuntimeBuilder::new());
        let dispatcher = runtime.dispatcher();
        S::spawn_detached(runtime.run());
        Self {
            dispatcher: WD::__new(dispatcher, crate::__private::Token::new()),
            _app: PhantomData,
        }
    }

    #[cfg(feature = "frb-compat")]
    pub fn new_frb(builder_fn: impl FnOnce(MvuRuntimeBuilder<A>) -> MvuRuntime<A>) -> Self {
        Self::new::<FrbSpawner>(builder_fn)
    }

    #[cfg(feature = "tokio")]
    pub fn new_tokio(
        builder_fn: impl FnOnce(MvuRuntimeBuilder<A>) -> (MvuRuntime<A>, ShouldRefreshSubscriber<A>),
    ) -> Self {
        Self::new::<TokioSpawner>(builder_fn)
    }
}

impl<A, WD> AppHandle<A, WD>
where
    A: Application,
    WD: WrappedDispatcher<Model = A::RootModel>,
{
    pub fn dispatcher(&self) -> WD
    where
        WD: Clone,
    {
        self.dispatcher.clone()
    }
}

impl<A, WD> AppHandle<A, WD>
where
    A: Application,
    WD: WrappedDispatcher<Model = A::RootModel>,
{
    pub fn updater(&self) -> WD::Updater {
        let (updater, _) = self.dispatcher.clone().__split(crate::__token());
        updater
    }

    pub fn getter(&self) -> WD::Getter {
        let (_, getter) = self.dispatcher.clone().__split(crate::__token());
        getter
    }
}
