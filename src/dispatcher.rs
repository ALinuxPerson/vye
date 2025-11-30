use crate::runtime::{Action, GetterAction, UpdateAction};
use crate::{
    Application, Error, Model, ModelGetterHandler, ModelGetterMessage, ModelHandler, ModelMessage,
    UpdateContext,
};
use futures::channel::{mpsc, oneshot};
use futures::{FutureExt, SinkExt};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("the channel to the mvu runtime is closed")]
#[non_exhaustive]
pub struct MvuRuntimeChannelClosedError;

type RootModelOf<M> = <<M as Model>::ForApp as Application>::RootModel;
type UpdateMapper<M> = dyn Fn(UpdateAction<M>) -> UpdateAction<RootModelOf<M>>;
type GetterMapper<M> = dyn Fn(GetterAction<M>) -> GetterAction<RootModelOf<M>>;

pub struct Dispatcher<M: Model> {
    tx: mpsc::Sender<Action<RootModelOf<M>>>,
    update_mapper: Arc<UpdateMapper<M>>,
    getter_mapper: Arc<GetterMapper<M>>,
}

impl<M: Model> Dispatcher<M> {
    pub async fn try_send<Msg>(&mut self, message: Msg) -> Result<(), MvuRuntimeChannelClosedError>
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        let action = Box::new(move |model: &mut M, ctx: &mut UpdateContext<M::ForApp>| {
            model.update(message, ctx)
        });
        let root_action = (self.update_mapper)(action);
        self.tx
            .send(Action::Update(root_action))
            .await
            .map_err(|_| MvuRuntimeChannelClosedError)
    }

    pub async fn send<Msg>(&mut self, message: Msg)
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        self.try_send(message)
            .await
            .expect("the channel to the mvu runtime is closed")
    }

    pub async fn try_get<Msg>(&mut self, message: Msg) -> Result<Msg::Data, Error>
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        let (tx, rx) = oneshot::channel();
        let action: GetterAction<M> = Box::new(move |model: &M| {
            async move {
                let data = model.getter(message);
                tx.send(data).ok();
            }
            .boxed()
        });
        let root_action = (self.getter_mapper)(action);

        self.tx
            .send(Action::Getter(root_action))
            .await
            .map_err(|_| Error::MvuRuntimeChannelClosed)?;
        rx.await.map_err(|_| Error::MvuRuntimeChannelClosed)
    }

    pub async fn get<Msg>(&mut self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.try_get(message).await.expect("the channel is closed")
    }

    pub fn split(self) -> (Updater<M>, Getter<M>) {
        (Updater(self.clone()), Getter(self))
    }
}

impl<M: Model> Dispatcher<M> {
    pub fn zoom<Child>(self, lens: Lens<M, Child>) -> Dispatcher<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        let update_mapper = Arc::new(move |child_action: UpdateAction<Child>| {
            let parent_action: UpdateAction<M> = Box::new(move |parent, ctx| {
                let child = (lens.get_mut)(parent);
                child_action(child, ctx)
            });
            (self.update_mapper)(parent_action)
        });
        let getter_mapper = Arc::new(move |child_getter: GetterAction<Child>| {
            let parent_getter: GetterAction<M> = Box::new(move |parent| {
                let child = (lens.get)(parent);
                child_getter(child)
            });

            (self.getter_mapper)(parent_getter)
        });

        Dispatcher {
            tx: self.tx,
            update_mapper,
            getter_mapper,
        }
    }
}

impl<M: Model> Clone for Dispatcher<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            update_mapper: Arc::clone(&self.update_mapper),
            getter_mapper: Arc::clone(&self.getter_mapper),
        }
    }
}

impl<R> Dispatcher<R>
where
    R: Model,
    <R as Model>::ForApp: Application<RootModel = R>,
{
    pub(crate) fn new_root(tx: mpsc::Sender<Action<R>>) -> Self {
        Self {
            tx,
            update_mapper: Arc::new(|action| action),
            getter_mapper: Arc::new(|action| action),
        }
    }
}

pub struct Lens<Parent, Child> {
    pub get: fn(&Parent) -> &Child,
    pub get_mut: fn(&mut Parent) -> &mut Child,
}

impl<Parent, Child> Copy for Lens<Parent, Child> {}

impl<Parent, Child> Clone for Lens<Parent, Child> {
    fn clone(&self) -> Self {
        *self
    }
}

#[macro_export]
macro_rules! lens {
    ($parent:ty => $child:ident) => {
        $crate::dispatcher:FnLens::<$parent, _> {
            get: |parent| &parent.$child,
            get_mut: |parent| &mut parent.$child,
        }
    };
}

pub struct Updater<M: Model>(Dispatcher<M>);

impl<M: Model> Updater<M> {
    pub async fn try_send<Msg>(&mut self, message: Msg) -> Result<(), MvuRuntimeChannelClosedError>
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        self.0.try_send(message).await
    }

    pub async fn send<Msg>(&mut self, message: Msg)
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        self.0.send(message).await
    }

    pub fn zoom<Child>(self, lens: fn(&mut M) -> &mut Child) -> Updater<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        Updater(self.0.zoom(Lens {
            get_mut: lens,
            get: |_| {
                unreachable!(
                    "did not expect `get` to be called for an Updater only dispatcher instance"
                )
            },
        }))
    }
}

impl<M: Model> Clone for Updater<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct Getter<M: Model>(Dispatcher<M>);

impl<M: Model> Getter<M> {
    pub async fn try_get<Msg>(&mut self, message: Msg) -> Result<Msg::Data, Error>
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.0.try_get(message).await
    }

    pub async fn get<Msg>(&mut self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.0.get(message).await
    }

    pub fn zoom<Child>(self, lens: fn(&M) -> &Child) -> Getter<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        Getter(self.0.zoom(Lens {
            get_mut: |_| {
                unreachable!(
                    "did not expect `get_mut` to be called for a Getter only dispatcher instance"
                )
            },
            get: lens,
        }))
    }
}

impl<M: Model> Clone for Getter<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
