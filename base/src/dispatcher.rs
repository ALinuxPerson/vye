use crate::{Application, Model, ModelBase, ModelGetterHandler, ModelGetterMessage, MvuRuntimeChannelClosedError, Signal, __private};
use futures::SinkExt;
use futures::channel::mpsc;
use std::convert::identity;
use crate::maybe::Shared;

type RootModelOf<M> = <<M as Model>::ForApp as Application>::RootModel;
type RootMessageOf<M> = <RootModelOf<M> as Model>::Message;

// NOTE: can't use `+ MaybeSendSync` because it is not an auto trait
#[cfg(feature = "thread-safe")]
type Mapper<M> = dyn Fn(<M as Model>::Message) -> RootMessageOf<M> + Send + Sync;

#[cfg(not(feature = "thread-safe"))]
type Mapper<M> = dyn Fn(<M as Model>::Message) -> RootMessageOf<M>;

pub struct Updater<M: Model> {
    tx: mpsc::Sender<RootMessageOf<M>>,
    mapper: Shared<Mapper<M>>,
}

impl<R> Updater<R>
where
    R: Model,
    <R as Model>::ForApp: Application<RootModel = R>,
{
    pub(crate) fn new(tx: mpsc::Sender<RootMessageOf<R>>) -> Self {
        Self {
            tx,
            mapper: Shared::new(identity),
        }
    }
}

impl<M: Model> Updater<M> {
    pub async fn try_send(
        &mut self,
        message: M::Message,
    ) -> Result<(), MvuRuntimeChannelClosedError> {
        self.tx
            .send((self.mapper)(message))
            .await
            .map_err(|_| MvuRuntimeChannelClosedError)
    }

    pub async fn send(&mut self, message: M::Message) {
        self.try_send(message)
            .await
            .expect("the channel to the mvu runtime is closed")
    }

    pub fn zoom<Child>(self, lens: fn(<Child as Model>::Message) -> M::Message) -> Updater<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        let parent_mapper = Shared::clone(&self.mapper);
        let child_mapper = Shared::new(move |child_message| {
            let parent_message = lens(child_message);
            parent_mapper(parent_message)
        });
        Updater {
            tx: self.tx.clone(),
            mapper: child_mapper,
        }
    }
}

impl<M: Model> Clone for Updater<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            mapper: Shared::clone(&self.mapper),
        }
    }
}

pub trait WrappedUpdater: Clone + __private::Sealed {
    type Model: Model;

    #[doc(hidden)]
    fn __new(updater: Updater<Self::Model>, _token: __private::Token) -> Self;
}

pub struct Getter<M> {
    model: ModelBase<M>,
}

impl<M> Getter<M> {
    pub(crate) fn new(model: ModelBase<M>) -> Self {
        Self { model }
    }

    pub fn get<Msg>(&self) -> Signal<Msg::Data>
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.model.get()
    }

    pub fn zoom<Child>(self, lens: fn(&M) -> &ModelBase<Child>) -> Getter<Child>
    where
        M: Model,
        Child: Model<ForApp = M::ForApp>,
    {
        Getter {
            model: self.model.zoom(lens),
        }
    }
}

impl<M> Clone for Getter<M> {
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
        }
    }
}

pub trait WrappedGetter: Clone + __private::Sealed {
    type Model: Model;

    #[doc(hidden)]
    fn __new(getter: Getter<Self::Model>, _token: __private::Token) -> Self;
}
