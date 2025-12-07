use crate::base::{Application, Command, Model, ModelGetterHandler, ModelGetterMessage};
use crate::{FlushSignals, Interceptor, ModelBase, ModelBaseReader};
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use core::any::type_name;
use core::ops::ControlFlow;
use futures::StreamExt;
use futures::channel::mpsc;
use crate::{Getter, Updater};
use crate::maybe::{MaybeRwLockReadGuard, MaybeSendSync, Shared};

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
    pub fn read(&self) -> MaybeRwLockReadGuard<'_, A::RootModel> {
        self.model.read()
    }

    pub fn getter<Msg>(&self, message: Msg) -> Msg::Data
    where
        Msg: ModelGetterMessage,
        A::RootModel: ModelGetterHandler<Msg>,
    {
        self.model.get(message)
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

type RootMessage<A> = <<A as Application>::RootModel as Model>::Message;

pub struct MvuRuntime<A: Application> {
    model: ModelBase<A::RootModel>,
    world: World,
    interceptors: Vec<Box<dyn Interceptor<A>>>,
    queue: CommandQueue<A>,
    signals: VecDeque<Shared<dyn FlushSignals>>,
    updater: Updater<A::RootModel>,
    message_rx: mpsc::Receiver<RootMessage<A>>,
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
        match self.message_rx.next().await {
            Some(action) => self.handle_message(action).await,
            None => return ControlFlow::Break(()),
        };

        ControlFlow::Continue(())
    }

    async fn handle_message(&mut self, message: RootMessage<A>) {
        for interceptor in &mut self.interceptors {
            interceptor.intercept(self.model.reader(), &message);
        }
        let mut update_ctx = UpdateContext {
            queue: &mut self.queue,
        };
        self.model.write().update(message, &mut update_ctx);
        self.model.__accumulate_signals(&mut self.signals, crate::__token());
        let mut command_ctx = CommandContext {
            model: self.model.reader(),
            world: &mut self.world,
            updater: self.updater.clone(),
        };
        while let Some(mut command) = self.queue.pop() {
            tracing::debug!(?command, "applying command");
            command.apply(&mut command_ctx).await;
        }
        while let Some(signal) = self.signals.pop_front() {
            signal.__flush(crate::__token());
        }
    }
}

impl<A: Application> MvuRuntime<A> {
    pub fn updater(&self) -> Updater<A::RootModel> {
        self.updater.clone()
    }

    pub fn getter(&self) -> Getter<A::RootModel> {
        Getter::new(self.model.clone())
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
    interceptors: Vec<Box<dyn Interceptor<A>>>,
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

    pub fn interceptor(mut self, value: impl Interceptor<A>) -> Self {
        self.interceptors.push(Box::new(value));
        self
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

        let (message_tx, message_rx) = mpsc::channel(self.buffer_size);

        MvuRuntime {
            model: model.clone(),
            world: self.world,
            interceptors: self.interceptors,
            queue: CommandQueue::default(),
            signals: VecDeque::new(),
            updater: Updater::new(message_tx),
            message_rx,
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
