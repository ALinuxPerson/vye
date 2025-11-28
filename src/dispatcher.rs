use crate::base::{Application, ChildModel, Model, ParentModel};
use async_trait::async_trait;
use futures::SinkExt;
use futures::channel::mpsc;
use std::marker::PhantomData;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("the channel to the mvu runtime is closed")]
#[non_exhaustive]
pub struct MvuRuntimeChannelClosedError;

#[async_trait]
pub trait Dispatcher<Msg>: Clone {
    async fn try_dispatch(&self, message: Msg) -> Result<(), MvuRuntimeChannelClosedError>;
}

#[async_trait]
pub trait DispatcherExt<Msg: Send>: Dispatcher<Msg> {
    async fn dispatch(&self, message: Msg)
    where
        Msg: 'async_trait,
    {
        self.try_dispatch(message)
            .await
            .expect("mvu runtime channel closed")
    }
}

impl<D, Msg> DispatcherExt<Msg> for D
where
    D: Dispatcher<Msg>,
    Msg: Send,
{
}

pub struct MessageDispatcher<A: Application>(
    pub(crate) mpsc::Sender<<A::RootModel as Model>::Message>,
);

impl<A: Application> MessageDispatcher<A> {
    pub fn map<CM>(self) -> MappedDispatcher<Self, CM>
    where
        CM: ChildModel<Parent = A::RootModel>,
        <A::RootModel as Model>::Message: From<CM::Message>,
    {
        MappedDispatcher(self, PhantomData)
    }
}

#[async_trait]
impl<A: Application> Dispatcher<<A::RootModel as Model>::Message> for MessageDispatcher<A> {
    async fn try_dispatch(
        &self,
        message: <A::RootModel as Model>::Message,
    ) -> Result<(), MvuRuntimeChannelClosedError> {
        self.0
            .clone()
            .send(message)
            .await
            .map_err(|_| MvuRuntimeChannelClosedError)
    }
}

impl<A: Application> Clone for MessageDispatcher<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct MappedDispatcher<PD, CM>(PD, PhantomData<CM>);

impl<PD, CM> MappedDispatcher<PD, CM> {
    pub fn map<SCM>(self) -> MappedDispatcher<Self, SCM>
    where
        SCM: ChildModel<Parent = CM>,
        CM: ParentModel<SCM>,
        CM::Message: From<SCM::Message>,
    {
        MappedDispatcher(self, PhantomData)
    }
}

impl<PD, CM> Clone for MappedDispatcher<PD, CM>
where
    CM: ChildModel,
    PD: Dispatcher<<<CM as ChildModel>::Parent as Model>::Message>,
{
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

#[async_trait]
impl<PD, CM> Dispatcher<CM::Message> for MappedDispatcher<PD, CM>
where
    CM: ChildModel + Sync,
    CM::Message: Send,
    PD: Dispatcher<<<CM as ChildModel>::Parent as Model>::Message> + Sync,
    <CM::Parent as Model>::Message: From<CM::Message>,
{
    async fn try_dispatch(&self, message: CM::Message) -> Result<(), MvuRuntimeChannelClosedError> {
        self.0.try_dispatch(message.into()).await
    }
}
