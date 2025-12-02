use alloc::boxed::Box;
use crate::runtime::UpdateAction;
use crate::{
    Application, Lens, Model, ModelBase, ModelGetterHandler, ModelGetterMessage, ModelHandler,
    ModelMessage, UpdateContext,
};
use futures::SinkExt;
use futures::channel::mpsc;
use alloc::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("the channel to the mvu runtime is closed")]
#[non_exhaustive]
pub struct MvuRuntimeChannelClosedError;

type RootModelOf<M> = <<M as Model>::ForApp as Application>::RootModel;
type UpdateMapper<M> = dyn Fn(UpdateAction<M>) -> UpdateAction<RootModelOf<M>> + Send + Sync;

pub struct Dispatcher<M: Model> {
    tx: mpsc::Sender<UpdateAction<RootModelOf<M>>>,
    model: ModelBase<M>,
    mapper: Arc<UpdateMapper<M>>,
}

impl<M: Model> Dispatcher<M> {
    pub async fn try_send<Msg>(&mut self, message: Msg) -> Result<(), MvuRuntimeChannelClosedError>
    where
        Msg: ModelMessage,
        M: ModelHandler<Msg>,
    {
        let action = Box::new(
            move |model: ModelBase<M>, ctx: &mut UpdateContext<M::ForApp>| {
                model.update(message, ctx)
            },
        );
        let root_action = (self.mapper)(action);
        self.tx
            .send(root_action)
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

    pub fn get<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.model.getter(message)
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
            let parent_action: UpdateAction<M> =
                Box::new(move |parent, ctx| child_action(parent.zoom(lens), ctx));
            (self.mapper)(parent_action)
        });

        Dispatcher {
            tx: self.tx,
            model: self.model.zoom(lens),
            mapper: update_mapper,
        }
    }
}

impl<M: Model> Clone for Dispatcher<M> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            model: self.model.clone(),
            mapper: Arc::clone(&self.mapper),
        }
    }
}

impl<R> Dispatcher<R>
where
    R: Model,
    <R as Model>::ForApp: Application<RootModel = R>,
{
    pub(crate) fn new_root(tx: mpsc::Sender<UpdateAction<R>>, model: ModelBase<R>) -> Self {
        Self {
            tx,
            model,
            mapper: Arc::new(|action| action),
        }
    }
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

    pub fn zoom<Child>(self, lens: Lens<M, Child>) -> Updater<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        Updater(self.0.zoom(lens))
    }
}

impl<M: Model> Clone for Updater<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct Getter<M: Model>(Dispatcher<M>);

impl<M: Model> Getter<M> {
    pub fn get<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        M: ModelGetterHandler<Msg>,
    {
        self.0.get(message)
    }

    pub fn zoom<Child>(self, lens: Lens<M, Child>) -> Getter<Child>
    where
        Child: Model<ForApp = M::ForApp>,
    {
        Getter(self.0.zoom(lens))
    }
}

impl<M: Model> Clone for Getter<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[doc(hidden)]
pub mod __private {
    pub trait Sealed {}

    pub struct Token(());

    impl Token {
        pub const fn new() -> Self {
            Self(())
        }
    }

    impl Default for Token {
        fn default() -> Self {
            Self::new()
        }
    }
}

pub trait WrappedDispatcher: __private::Sealed {
    type Model: Model;

    #[doc(hidden)]
    fn __new(dispatcher: Dispatcher<Self::Model>, _token: __private::Token) -> Self;
}

pub trait SplittableWrappedDispatcher: WrappedDispatcher + Clone {
    type Updater: WrappedUpdater<WrappedDispatcher = Self>;
    type Getter: WrappedGetter<WrappedDispatcher = Self>;

    #[doc(hidden)]
    fn __split(self, _token: __private::Token) -> (Self::Updater, Self::Getter) {
        (
            Self::Updater::__new(self.clone(), __private::Token::new()),
            Self::Getter::__new(self, __private::Token::new()),
        )
    }
}

pub trait WrappedUpdater: __private::Sealed {
    type WrappedDispatcher: SplittableWrappedDispatcher<Updater = Self>;

    #[doc(hidden)]
    fn __new(dispatcher: Self::WrappedDispatcher, _token: __private::Token) -> Self;
}

pub trait WrappedGetter: __private::Sealed {
    type WrappedDispatcher: SplittableWrappedDispatcher<Getter = Self>;

    #[doc(hidden)]
    fn __new(dispatcher: Self::WrappedDispatcher, _token: __private::Token) -> Self;
}
