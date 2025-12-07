use crate::dispatcher::MvuRuntimeChannelClosedError;
use crate::runtime::{CommandContext, UpdateContext};
use crate::sync::VMutex;
use crate::{
    __private, VRWLockReadGuard, VRWLockWriteGuard, VRwLock, lock_mutex, read_vrwlock,
    write_vrwlock,
};
use alloc::sync::Arc;
use async_trait::async_trait;
use core::any::Any;
use core::fmt::Debug;
use core::hash::Hash;
use core::marker::PhantomData;
use futures::channel::mpsc;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

pub trait Application: 'static {
    type RootModel: Model<ForApp = Self>;
    type RegionId: Debug + Eq + Hash + Send + Sync;
}

pub struct AdHocApp<RootModel, RegionId = ()>(PhantomData<(RootModel, RegionId)>);

impl<RootModel, RegionId> Application for AdHocApp<RootModel, RegionId>
where
    RootModel: Model<ForApp = AdHocApp<RootModel, RegionId>>,
    RegionId: Debug + Eq + Hash + Send + Sync + 'static,
{
    type RootModel = RootModel;
    type RegionId = RegionId;
}

pub trait ModelMessage: Send + 'static {}

pub trait ModelGetterMessage: Send + 'static {
    type Data: Send + 'static;
}

pub trait Model: Send + Sync + 'static {
    type ForApp: Application;

    #[doc(hidden)]
    fn __accumulate_signals(
        &self,
        signals: &mut VecDeque<Arc<dyn FlushSignals>>,
        _token: __private::Token,
    );
}

pub trait ModelWithRegion: Model {
    const REGION: <Self::ForApp as Application>::RegionId;
}

pub trait ModelHandler<M: ModelMessage>: Model {
    fn update(&mut self, message: M, ctx: &mut UpdateContext<Self::ForApp>);
}

pub trait ModelGetterHandler<M: ModelGetterMessage>: Model {
    fn getter(&self, message: M) -> M::Data;
}

#[async_trait]
pub trait Command: Debug + Send + Sync {
    type ForApp: Application;

    async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>);
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("the channel to the mvu runtime is closed")]
    MvuRuntimeChannelClosed,

    #[error("the channel to the model getter is closed")]
    ModelGetterChannelClosed,
}

impl From<MvuRuntimeChannelClosedError> for Error {
    fn from(_: MvuRuntimeChannelClosedError) -> Self {
        Self::MvuRuntimeChannelClosed
    }
}

impl From<ModelGetterChannelClosedError> for Error {
    fn from(_: ModelGetterChannelClosedError) -> Self {
        Self::ModelGetterChannelClosed
    }
}

#[derive(Error, Debug)]
#[non_exhaustive]
#[error("the channel to the model getter is closed")]
pub struct ModelGetterChannelClosedError;

pub struct ModelBase<M>(Arc<VRwLock<M>>);

impl<M> ModelBase<M> {
    pub fn new(model: M) -> Self {
        Self(Arc::new(VRwLock::new(model)))
    }

    pub fn read(&self) -> VRWLockReadGuard<'_, M> {
        read_vrwlock(&self.0)
    }

    pub fn reader(&self) -> ModelBaseReader<M> {
        ModelBaseReader(self.clone())
    }

    pub fn write(&self) -> VRWLockWriteGuard<'_, M> {
        write_vrwlock(&self.0)
    }
}

pub type Lens<Parent, Child> = fn(&Parent) -> &ModelBase<Child>;

#[macro_export]
macro_rules! lens {
    ($parent:ty => $child:ident) => {
        |parent: $parent| &parent.$child
    };
}

impl<M: Model> ModelBase<M> {
    pub fn update<Msg>(&self, message: Msg, ctx: &mut UpdateContext<M::ForApp>)
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        self.write().update(message, ctx)
    }

    pub fn getter<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.read().getter(message)
    }

    pub fn zoom<Child>(&self, lens: Lens<M, Child>) -> ModelBase<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        lens(&*self.read()).clone()
    }

    #[doc(hidden)]
    pub fn __accumulate_signals(
        &self,
        signals: &mut VecDeque<Arc<dyn FlushSignals>>,
        token: __private::Token,
    ) {
        self.read().__accumulate_signals(signals, token);
    }
}

impl<M> Clone for ModelBase<M> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

pub struct ModelBaseReader<M>(ModelBase<M>);

impl<M> ModelBaseReader<M> {
    pub fn read(&self) -> VRWLockReadGuard<'_, M> {
        self.0.read()
    }
}

impl<M: Model> ModelBaseReader<M> {
    pub fn getter<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.0.getter(message)
    }

    pub fn zoom<Child>(&self, lens: Lens<M, Child>) -> ModelBaseReader<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        ModelBaseReader(self.0.zoom(lens))
    }
}

impl<M> Clone for ModelBaseReader<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub trait DynInterceptor: Send + Sync + 'static {
    fn intercept_dyn(&self, model: &dyn Any, message: &dyn Any);
}

impl<F> DynInterceptor for F
where
    F: Fn(&dyn Any, &dyn Any) + Send + Sync + 'static,
{
    fn intercept_dyn(&self, model: &dyn Any, message: &dyn Any) {
        self(model, message)
    }
}

pub trait Interceptor<M, Msg>: Send + Sync + 'static
where
    M: Model,
    Msg: ModelMessage,
{
    fn intercept(&self, model: ModelBaseReader<M>, message: &Msg);
}

impl<M, Msg, F> Interceptor<M, Msg> for F
where
    M: Model,
    Msg: ModelMessage,
    F: Fn(ModelBaseReader<M>, &Msg) + Send + Sync + 'static,
{
    fn intercept(&self, model: ModelBaseReader<M>, message: &Msg) {
        self(model, message)
    }
}

pub struct InterceptorWrapper<M, Msg, I> {
    interceptor: I,
    _marker: PhantomData<(M, Msg)>,
}

impl<M, Msg, I> InterceptorWrapper<M, Msg, I> {
    pub fn new(interceptor: I) -> Self {
        Self {
            interceptor,
            _marker: PhantomData,
        }
    }
}

impl<M, Msg, I> DynInterceptor for InterceptorWrapper<M, Msg, I>
where
    M: Model,
    Msg: ModelMessage + Sync,
    I: Interceptor<M, Msg>,
{
    fn intercept_dyn(&self, model: &dyn Any, message: &dyn Any) {
        if let (Some(model), Some(message)) = (
            model.downcast_ref::<ModelBase<M>>(),
            message.downcast_ref::<Msg>(),
        ) {
            let model_reader = ModelBaseReader(model.clone());
            self.interceptor.intercept(model_reader, message);
        }
    }
}

pub struct Signal<T>(Arc<SignalRepr<T>>);

impl<T> Signal<T> {
    pub fn subscribe(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(1);
        let mut subscribers = lock_mutex(&self.0.subscribers);
        subscribers.push(tx);
        rx
    }

    pub fn read(&self) -> VRWLockReadGuard<'_, T> {
        read_vrwlock(&self.0.state)
    }

    fn write(&mut self) -> VRWLockWriteGuard<'_, T> {
        write_vrwlock(&self.0.state)
    }

    pub fn update<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        let ret = { f(&mut *self.write()) };
        self.0.dirty.store(true, Ordering::Release);
        ret
    }

    pub fn set(&mut self, value: T) {
        self.update(|v| *v = value);
    }

    #[doc(hidden)]
    pub fn __to_dyn_flush_signals(&self, _: __private::Token) -> Arc<dyn FlushSignals>
    where
        T: Send + Sync + 'static,
    {
        Arc::clone(&self.0) as _
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

struct SignalRepr<T> {
    state: Arc<VRwLock<T>>,
    subscribers: VMutex<Vec<mpsc::Sender<()>>>,
    dirty: AtomicBool,
}

#[doc(hidden)]
pub trait FlushSignals: Send {
    fn __flush(&mut self, _token: __private::Token);
}

impl<T: Send + Sync> FlushSignals for SignalRepr<T> {
    fn __flush(&mut self, _: __private::Token) {
        if self.dirty.swap(false, Ordering::AcqRel) {
            let mut subscribers = lock_mutex(&self.subscribers);
            for subscriber in &mut *subscribers {
                subscriber.try_send(()).ok();
            }
            subscribers.retain(|s| !s.is_closed());
        }
    }
}
