use crate::__private;
use crate::maybe::{
    MaybeMutex, MaybeRwLock, MaybeRwLockReadGuard, MaybeRwLockWriteGuard, MaybeSend,
    MaybeSendStatic, MaybeSendSync, Shared,
};
use crate::host::{CommandContext, UpdateContext};
use core::fmt::Debug;
use core::marker::PhantomData;
use futures::StreamExt;
use futures::channel::mpsc;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

// must be `'static` for interceptors
pub trait Application: 'static {
    type RootModel: Model<ForApp = Self>;
}

pub struct AdHocApp<RootModel>(PhantomData<RootModel>);

impl<RootModel> Application for AdHocApp<RootModel>
where
    RootModel: Model<ForApp = AdHocApp<RootModel>>,
{
    type RootModel = RootModel;
}

pub trait ModelGetterMessage: MaybeSendStatic {
    type Data: MaybeSendStatic;
}

pub trait Model: MaybeSendSync + 'static {
    type ForApp: Application;
    type Message: MaybeSend;

    fn update(&mut self, message: Self::Message, ctx: &mut UpdateContext<Self::ForApp>);

    #[doc(hidden)]
    fn __accumulate_signals(
        &self,
        signals: &mut VecDeque<Shared<dyn FlushSignals>>,
        _token: __private::Token,
    );
}

pub trait ModelGetterHandler<M: ModelGetterMessage>: Model {
    fn getter(&self) -> Signal<M::Data>;
}

maybe_async_trait! {
    pub trait Command: Debug + MaybeSendSync {
        type ForApp: Application;

        async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>);
    }

    impl<C> Command for Option<C>
    where
        C: Command,
        Option<C>: MaybeSendSync,
    {
        type ForApp = C::ForApp;

        async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>) {
            if let Some(this) = self {
                this.apply(ctx).await
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("the channel to the host is closed")]
    HostChannelClosed,

    #[error("the channel to the model getter is closed")]
    ModelGetterChannelClosed,
}

impl From<HostChannelClosed> for Error {
    fn from(_: HostChannelClosed) -> Self {
        Self::HostChannelClosed
    }
}

impl From<ModelGetterChannelClosedError> for Error {
    fn from(_: ModelGetterChannelClosedError) -> Self {
        Self::ModelGetterChannelClosed
    }
}

#[derive(Error, Debug)]
#[error("the channel to the host is closed")]
#[non_exhaustive]
pub struct HostChannelClosed;

#[derive(Error, Debug)]
#[non_exhaustive]
#[error("the channel to the model getter is closed")]
pub struct ModelGetterChannelClosedError;

pub struct ModelBase<M>(Shared<MaybeRwLock<M>>);

impl<M> ModelBase<M> {
    pub fn new(model: M) -> Self {
        Self(Shared::new(MaybeRwLock::new(model)))
    }

    pub fn read(&self) -> MaybeRwLockReadGuard<'_, M> {
        self.0.read()
    }

    pub fn reader(&self) -> ModelBaseReader<M> {
        ModelBaseReader(self.clone())
    }

    pub fn write(&self) -> MaybeRwLockWriteGuard<'_, M> {
        self.0.write()
    }
}

pub type Lens<MParent, MChild> = fn(&<MChild as Model>::Message) -> <MParent as Model>::Message;

#[macro_export]
macro_rules! lens {
    ($parent:ty => $child:ident) => {
        |parent: $parent| &parent.$child
    };
}

impl<M: Model> ModelBase<M> {
    pub fn update(&self, message: M::Message, ctx: &mut UpdateContext<M::ForApp>) {
        self.write().update(message, ctx)
    }

    pub fn get<Msg>(&self) -> Signal<Msg::Data>
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.read().getter()
    }

    pub fn zoom<Child>(&self, lens: fn(&M) -> &ModelBase<Child>) -> ModelBase<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        lens(&*self.read()).clone()
    }

    #[doc(hidden)]
    pub fn __accumulate_signals(
        &self,
        signals: &mut VecDeque<Shared<dyn FlushSignals>>,
        token: __private::Token,
    ) {
        self.read().__accumulate_signals(signals, token);
    }
}

impl<M> Clone for ModelBase<M> {
    fn clone(&self) -> Self {
        Self(Shared::clone(&self.0))
    }
}

pub struct ModelBaseReader<M>(ModelBase<M>);

impl<M> ModelBaseReader<M> {
    pub fn read(&self) -> MaybeRwLockReadGuard<'_, M> {
        self.0.read()
    }
}

impl<M: Model> ModelBaseReader<M> {
    pub fn get<Msg>(&self) -> Signal<Msg::Data>
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.0.get()
    }

    pub fn zoom<Child>(&self, lens: fn(&M) -> &ModelBase<Child>) -> ModelBaseReader<Child>
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

pub trait Interceptor<A: Application>: MaybeSendSync + 'static {
    fn intercept(
        &mut self,
        model: ModelBaseReader<A::RootModel>,
        message: &<A::RootModel as Model>::Message,
    );
}

impl<A, F> Interceptor<A> for F
where
    A: Application,
    F: FnMut(ModelBaseReader<A::RootModel>, &<A::RootModel as Model>::Message)
        + MaybeSendSync
        + 'static,
{
    fn intercept(
        &mut self,
        model: ModelBaseReader<A::RootModel>,
        message: &<A::RootModel as Model>::Message,
    ) {
        self(model, message)
    }
}

pub enum SignalStatus {
    Changed,
    Destroyed,
}

pub struct Signal<T>(Shared<SignalRepr<T>>);

impl<T> Signal<T> {
    pub fn new(value: T) -> Self {
        Self(Shared::new(SignalRepr {
            data: Shared::new(MaybeRwLock::new(value)),
            subscribers: MaybeMutex::new(Vec::new()),
            dirty: AtomicBool::new(false),
        }))
    }

    pub fn subscribe(&self) -> SignalSubscriber<T> {
        let (status_tx, status_rx) = mpsc::channel(1);
        let mut subscribers = self.0.subscribers.lock();
        subscribers.push(status_tx);
        SignalSubscriber {
            reader: self.reader(),
            status_rx,
        }
    }

    pub fn reader(&self) -> SignalReader<T> {
        SignalReader(self.clone())
    }

    pub fn writer(&self) -> SignalWriter<T> {
        SignalWriter(self.clone())
    }

    #[doc(hidden)]
    pub fn __to_dyn_flush_signals(&self, _: __private::Token) -> Shared<dyn FlushSignals>
    where
        T: MaybeSendSync + 'static,
    {
        Shared::clone(&self.0) as _
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(Shared::clone(&self.0))
    }
}

impl<T: Default> Default for Signal<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Drop for Signal<T> {
    fn drop(&mut self) {
        for subscriber in &mut *self.0.subscribers.lock() {
            subscriber.try_send(SignalStatus::Destroyed).ok();
        }
    }
}

struct SignalRepr<T> {
    data: Shared<MaybeRwLock<T>>,
    subscribers: MaybeMutex<Vec<mpsc::Sender<SignalStatus>>>,
    dirty: AtomicBool,
}

#[doc(hidden)]
pub trait FlushSignals: MaybeSendSync {
    fn __flush(&self, _token: __private::Token);
}

impl<T: MaybeSendSync> FlushSignals for SignalRepr<T> {
    fn __flush(&self, _: __private::Token) {
        if self.dirty.swap(false, Ordering::AcqRel) {
            let mut subscribers = self.subscribers.lock();
            for subscriber in &mut *subscribers {
                subscriber.try_send(SignalStatus::Changed).ok();
            }
            subscribers.retain(|s| !s.is_closed());
        }
    }
}

impl<T: MaybeSendSync> FlushSignals for Vec<Signal<T>> {
    fn __flush(&self, _: __private::Token) {
        for signal in self {
            signal.0.__flush(crate::__token());
        }
    }
}

pub struct SignalSubscriber<T> {
    reader: SignalReader<T>,
    status_rx: mpsc::Receiver<SignalStatus>,
}

impl<T> SignalSubscriber<T> {
    pub fn read(&self) -> MaybeRwLockReadGuard<'_, T> {
        self.reader.read()
    }

    pub async fn recv_status(&mut self) -> Option<SignalStatus> {
        self.status_rx.next().await
    }
}

pub struct SignalReader<T>(Signal<T>);

impl<T> SignalReader<T> {
    pub fn read(&self) -> MaybeRwLockReadGuard<'_, T> {
        self.0.0.data.read()
    }
}

impl<T> Clone for SignalReader<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct SignalWriter<T>(Signal<T>);

impl<T> SignalWriter<T> {
    pub fn write(&self) -> MaybeRwLockWriteGuard<'_, T> {
        self.0.0.data.write()
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let ret = f(&mut self.write());
        self.0.0.dirty.store(true, Ordering::Release);
        ret
    }

    pub fn set(&self, value: T) {
        self.update(|data| *data = value);
    }
}

impl<T> Clone for SignalWriter<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
