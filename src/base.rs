use thiserror::Error;
use crate::dispatcher::MvuRuntimeChannelClosedError;
use crate::runtime::{ApplyContext, UpdateContext};
use async_trait::async_trait;
use futures::channel::{mpsc, oneshot};
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;

pub trait Application {
    type RootModel: RootModel<ForApp = Self>;
    type RegionId: Debug + Eq + Hash;
    type State;
}

pub trait Model {
    type ForApp: Application;
    type Getter: ModelGetter<Model = Self>;
    type Message: Send + Debug;
    type UpdateRet;

    fn update(
        &mut self,
        message: Self::Message,
        ctx: &mut UpdateContext<Self::ForApp>,
    ) -> Self::UpdateRet;
    fn getter(&self, message: <Self::Getter as ModelGetter>::Message);
}

pub trait RootModel: Model<Getter: RootModelGetter, UpdateRet = ()> {}

pub trait ParentModel<CM: ChildModel<Parent = Self>>: Model {}

pub trait ChildModel: Model + Sized {
    type Parent: ParentModel<Self>;
}

pub trait ModelGetter {
    type Model: Model<Getter = Self>;
    type Message;
}

#[async_trait]
pub trait SendableModelGetter: ModelGetter {
    async fn send<T: Send>(
        &self,
        f: impl FnOnce(oneshot::Sender<T>) -> Self::Message + Send,
    ) -> Result<T, Error>;
}

pub trait ParentModelGetter<MG: ChildModelGetter<Parent = Self>>: ModelGetter {}

pub trait ChildModelGetter: ModelGetter + Sized {
    type Parent: ParentModelGetter<Self>;

    fn parent(&self) -> &Self::Parent;
}

#[async_trait]
impl<CMG> SendableModelGetter for CMG
where
    CMG: ChildModelGetter + Sync,
    <Self as ChildModelGetter>::Parent: SendableModelGetter,
    <<Self as ChildModelGetter>::Parent as ModelGetter>::Message: From<Self::Message>,
{
    async fn send<T: Send>(
        &self,
        f: impl FnOnce(oneshot::Sender<T>) -> Self::Message + Send,
    ) -> Result<T, Error> {
        self.parent().send(|tx| f(tx).into()).await
    }
}

pub trait RootModelGetter: SendableModelGetter {
    fn new(tx: mpsc::Sender<Self::Message>) -> Self;
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
    fn from(_value: MvuRuntimeChannelClosedError) -> Self {
        Self::MvuRuntimeChannelClosed
    }
}

pub struct AdHocApp<RootModel, ViewId, State>(PhantomData<(RootModel, ViewId, State)>);

impl<RootModel, ViewId, State> Application for AdHocApp<RootModel, ViewId, State>
where
    RootModel: self::RootModel<ForApp = Self>,
    ViewId: Debug + Eq + Hash,
{
    type RootModel = RootModel;
    type RegionId = ViewId;
    type State = State;
}

