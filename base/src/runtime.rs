use crate::base::{Application, Command, Model, ModelGetterHandler, ModelGetterMessage};
use crate::maybe::MaybeSendSync;
use crate::{
    Dispatcher, DynInterceptor, Interceptor, InterceptorWrapper, ModelBase, ModelBaseReader,
    Updater, VRWLockReadGuard,
};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use core::any::type_name;
use core::ops::ControlFlow;
use futures::channel::mpsc;
use futures::StreamExt;
use std::sync::Arc;

const DEFAULT_CHANNEL_BUFFER_SIZE: usize = 64;

pub struct CommandQueue<A>(VecDeque<Box<dyn Command<ForApp = A>>>);

impl<A: Application> CommandQueue<A> {
    pub fn emit<C: Command<ForApp = A> + 'static>(&mut self, command: C) {
        self.0.push_back(Box::new(command));
    }
}

impl<A: Application> CommandQueue<A> {
    fn pop(&mut self) -> Option<Box<dyn Command<ForApp = A>>> {
        self.0.pop_front()
    }
}

impl<A> Default for CommandQueue<A> {
    fn default() -> Self {
        Self(VecDeque::new())
    }
}

pub struct UpdateContext<'rt, A: Application> {
    pub queue: &'rt mut CommandQueue<A>,
}

impl<'rt, A: Application> UpdateContext<'rt, A> {
    pub fn emit_command<C: Command<ForApp = A> + 'static>(&mut self, command: C) {
        self.queue.emit(command);
    }
}

pub struct CommandContext<'rt, A: Application> {
    pub model: ModelBaseReader<A::RootModel>,
    pub world: &'rt mut World,
    pub updater: Updater<A::RootModel>,
}

impl<'rt, A: Application> CommandContext<'rt, A> {
    pub fn read(&self) -> VRWLockReadGuard<'_, A::RootModel> {
        self.model.read()
    }

    pub fn getter<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        A::RootModel: ModelGetterHandler<Msg>,
    {
        self.model.getter(message)
    }

    pub fn state<S: MaybeSendSync + 'static>(&self) -> &S {
        self.world.get()
    }

    pub fn state_mut<S: MaybeSendSync + 'static>(&mut self) -> &mut S {
        self.world.get_mut()
    }

    pub async fn send_message(&mut self, message: <A::RootModel as Model>::Message) {
        self.updater.send(message).await
    }
}

// NOTE: can't use `+ MaybeSend` because it is not an auto trait
#[cfg(feature = "thread-safe")]
pub(crate) type UpdateAction<M> =
    Box<dyn FnOnce(ModelBase<M>, &mut UpdateContext<<M as Model>::ForApp>) + Send>;

#[cfg(not(feature = "thread-safe"))]
pub(crate) type UpdateAction<M> =
    Box<dyn FnOnce(ModelBase<M>, &mut UpdateContext<<M as Model>::ForApp>)>;

pub struct MvuRuntime<A: Application> {
    model: ModelBase<A::RootModel>,
    world: World,

    queue: CommandQueue<A>,

    dispatcher: Dispatcher<A::RootModel>,
    update_actions_rx: mpsc::Receiver<UpdateAction<A::RootModel>>,
}

impl<A: Application> MvuRuntime<A> {
    pub fn builder() -> MvuRuntimeBuilder<A> {
        MvuRuntimeBuilder::new()
    }

    pub fn new(model: A::RootModel) -> Self {
        Self::builder().model(model).build()
    }

    pub fn defaults() -> Self
    where
        A::RootModel: Default,
    {
        MvuRuntimeBuilder::defaults().build()
    }
}

impl<A: Application> MvuRuntime<A> {
    pub async fn run(mut self) {
        tracing::debug!("mvu runtime has started");
        loop {
            if let ControlFlow::Break(()) = self.run_once().await {
                tracing::debug!("mvu runtime is stopping");
                break;
            }
        }
    }

    async fn run_once(&mut self) -> ControlFlow<()> {
        match self.update_actions_rx.next().await {
            Some(action) => self.handle_update(action).await,
            None => return ControlFlow::Break(()),
        };

        ControlFlow::Continue(())
    }

    async fn handle_update(&mut self, action: UpdateAction<A::RootModel>) {
        let mut update_ctx = UpdateContext {
            queue: &mut self.queue,
        };
        action(self.model.clone(), &mut update_ctx);

        let (updater, _) = self.dispatcher.clone().split();
        let mut command_ctx = CommandContext {
            model: self.model.reader(),
            world: &mut self.world,
            updater,
        };
        while let Some(mut command) = self.queue.pop() {
            tracing::debug!(?command, "applying command");
            command.apply(&mut command_ctx).await;
        }
    }
}

impl<A: Application> MvuRuntime<A> {
    pub fn dispatcher(&self) -> Dispatcher<A::RootModel> {
        self.dispatcher.clone()
    }
}

#[derive(Default)]
pub struct World(
    #[cfg(feature = "thread-safe")] type_map::concurrent::TypeMap,
    #[cfg(not(feature = "thread-safe"))] type_map::TypeMap,
);

impl World {
    pub(crate) fn add_with<S: MaybeSendSync + 'static>(mut self, state: S) -> Self {
        self.0.insert(state);
        self
    }

    pub(crate) fn add<S: Default + MaybeSendSync + 'static>(self) -> Self {
        self.add_with(S::default())
    }

    pub fn try_get<S: MaybeSendSync + 'static>(&self) -> Option<&S> {
        self.0.get()
    }

    pub fn get<S: MaybeSendSync + 'static>(&self) -> &S {
        self.try_get()
            .unwrap_or_else(|| panic!("`{}` does not exist in the world", type_name::<S>()))
    }

    pub fn try_get_mut<S: MaybeSendSync + 'static>(&mut self) -> Option<&mut S> {
        self.0.get_mut()
    }

    pub fn get_mut<S: MaybeSendSync + 'static>(&mut self) -> &mut S {
        self.try_get_mut()
            .unwrap_or_else(|| panic!("`{}` does not exist in the world", type_name::<S>()))
    }
}

pub struct MvuRuntimeBuilder<A: Application> {
    model: Option<A::RootModel>,
    world: World,
    interceptors: Vec<Box<dyn DynInterceptor>>,
    buffer_size: usize,
}

impl<A: Application> MvuRuntimeBuilder<A> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn defaults() -> Self
    where
        A::RootModel: Default,
    {
        Self::new().default_model()
    }

    pub fn model(self, value: A::RootModel) -> Self {
        Self {
            model: Some(value),
            ..self
        }
    }

    pub fn state_with<S: MaybeSendSync + 'static>(self, value: S) -> Self {
        Self {
            world: self.world.add_with(value),
            ..self
        }
    }

    pub fn state<S: Default + MaybeSendSync + 'static>(self) -> Self {
        Self {
            world: self.world.add::<S>(),
            ..self
        }
    }

    pub fn dyn_interceptor(mut self, value: impl DynInterceptor) -> Self {
        self.interceptors.push(Box::new(value));
        self
    }

    pub fn interceptor<M, Msg>(self, value: impl Interceptor<M>) -> Self
    where
        M: Model<ForApp = A>,
    {
        self.dyn_interceptor(InterceptorWrapper::new(value))
    }

    pub fn buffer_size(self, value: usize) -> Self {
        Self {
            buffer_size: value,
            ..self
        }
    }

    pub fn default_model(self) -> Self
    where
        A::RootModel: Default,
    {
        self.model(Default::default())
    }

    pub fn build(self) -> MvuRuntime<A> {
        let model = self.model.expect("RootModel was not initialized");
        let model = ModelBase::new(model);

        let (action_tx, action_rx) = mpsc::channel(self.buffer_size);

        MvuRuntime {
            model: model.clone(),
            world: self.world,
            queue: CommandQueue::default(),
            dispatcher: Dispatcher::new_root(action_tx, model, Arc::new(self.interceptors)),
            update_actions_rx: action_rx,
        }
    }
}

impl<A: Application> Default for MvuRuntimeBuilder<A> {
    fn default() -> Self {
        Self {
            model: None,
            world: World::default(),
            interceptors: Vec::new(),
            buffer_size: DEFAULT_CHANNEL_BUFFER_SIZE,
        }
    }
}
