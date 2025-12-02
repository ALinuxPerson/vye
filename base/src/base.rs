use crate::dispatcher::MvuRuntimeChannelClosedError;
use crate::runtime::{ApplyContext, UpdateContext};
use async_trait::async_trait;
use core::fmt::Debug;
use core::hash::Hash;
use core::marker::PhantomData;
use alloc::sync::Arc;
use alloc::boxed::Box;
use thiserror::Error;
use crate::{VRWLockReadGuard, VRWLockWriteGuard, VRwLock};

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

    async fn apply(&mut self, ctx: &mut ApplyContext<'_, Self::ForApp>);
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
        #[cfg(feature = "std")]
        let ret = self.0.read().unwrap();
        
        #[cfg(not(feature = "std"))]
        let ret = self.0.read();
        
        ret
    }

    pub fn write(&self) -> VRWLockWriteGuard<'_, M> {
        #[cfg(feature = "std")]
        let ret = self.0.write().unwrap();
        
        #[cfg(not(feature = "std"))]
        let ret = self.0.write();
        
        ret
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
}

impl<M> Clone for ModelBase<M> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}
