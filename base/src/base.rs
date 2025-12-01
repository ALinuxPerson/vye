use thiserror::Error;
use crate::dispatcher::MvuRuntimeChannelClosedError;
use crate::runtime::{ApplyContext, UpdateContext};
use async_trait::async_trait;
use std::fmt::Debug;
use std::hash::Hash;

pub trait Application: 'static {
    type RootModel: Model<ForApp = Self>;
    type RegionId: Debug + Eq + Hash + Send + Sync;
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
