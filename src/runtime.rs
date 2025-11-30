use crate::base::{Application, Command, Model};
use crate::{Dispatcher, ModelHandler, ModelMessage, ModelWithRegion};
use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use std::any::{Any, TypeId, type_name};
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::ops::ControlFlow;

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

pub struct DirtyRegions<A: Application>(HashSet<A::RegionId>);

impl<A: Application> DirtyRegions<A> {
    pub fn mark_with(&mut self, region: A::RegionId) {
        self.0.insert(region);
    }

    pub fn mark<M: ModelWithRegion<ForApp = A>>(&mut self) {
        self.mark_with(M::REGION)
    }

    pub fn is_dirty(&self, region: &A::RegionId) -> bool {
        self.0.contains(region)
    }
}

impl<A: Application> Default for DirtyRegions<A> {
    fn default() -> Self {
        Self(HashSet::new())
    }
}

pub struct UpdateContext<'rt, A: Application> {
    pub queue: &'rt mut CommandQueue<A>,
    pub dirty_regions: &'rt mut DirtyRegions<A>,
}

impl<'rt, A: Application> UpdateContext<'rt, A> {
    pub fn emit_command<C: Command<ForApp = A> + 'static>(&mut self, command: C) {
        self.queue.emit(command);
    }

    pub fn mark_dirty(&mut self, region: A::RegionId) {
        self.dirty_regions.mark_with(region);
    }
}

pub struct ApplyContext<'rt, A: Application> {
    pub model: &'rt A::RootModel,
    pub world: &'rt mut World,
    pub dispatcher: &'rt mut Dispatcher<A::RootModel>,
}

impl<'rt, A: Application> ApplyContext<'rt, A> {
    pub async fn send_message<Msg>(&mut self, message: Msg)
    where
        Msg: ModelMessage,
        A::RootModel: ModelHandler<Msg>,
    {
        self.dispatcher.send(message).await
    }

    pub fn state<S: Any>(&self) -> &S {
        self.world.get()
    }

    pub fn state_mut<S: Any>(&mut self) -> &mut S {
        self.world.get_mut()
    }
}

pub struct ShouldRefreshSubscriber<A: Application>(mpsc::Receiver<A::RegionId>);

impl<A: Application> ShouldRefreshSubscriber<A> {
    pub async fn recv(&mut self) -> Option<A::RegionId> {
        self.0.next().await
    }
}

pub(crate) type UpdateAction<M> =
    Box<dyn FnOnce(&mut M, &mut UpdateContext<<M as Model>::ForApp>) + Send>;
pub(crate) type GetterAction<M> = Box<dyn for<'a> FnOnce(&'a M) -> BoxFuture<'a, ()> + Send>;

pub(crate) enum Action<M: Model> {
    Update(UpdateAction<M>),
    Getter(GetterAction<M>),
}

pub struct MvuRuntime<A: Application> {
    model: A::RootModel,
    world: World,

    queue: CommandQueue<A>,
    dirty_regions: DirtyRegions<A>,

    dispatcher: Dispatcher<A::RootModel>,
    action_rx: mpsc::Receiver<Action<A::RootModel>>,

    should_refresh_tx: mpsc::Sender<A::RegionId>,
}

impl<A: Application> MvuRuntime<A> {
    pub fn builder() -> MvuRuntimeBuilder<A> {
        MvuRuntimeBuilder::new()
    }

    pub fn new(model: A::RootModel) -> (Self, ShouldRefreshSubscriber<A>) {
        Self::builder().model(model).build()
    }

    pub fn defaults() -> (Self, ShouldRefreshSubscriber<A>)
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
        match self.action_rx.next().await {
            Some(Action::Update(action)) => self.handle_update(action).await,
            Some(Action::Getter(action)) => self.handle_getter(action).await,
            None => return ControlFlow::Break(()),
        };

        ControlFlow::Continue(())
    }

    async fn handle_update(&mut self, action: UpdateAction<A::RootModel>) {
        let mut update_ctx = UpdateContext {
            queue: &mut self.queue,
            dirty_regions: &mut self.dirty_regions,
        };
        action(&mut self.model, &mut update_ctx);

        let mut command_ctx = ApplyContext {
            model: &self.model,
            world: &mut self.world,
            dispatcher: &mut self.dispatcher,
        };
        while let Some(mut command) = self.queue.pop() {
            tracing::debug!(?command, "applying command");
            command.apply(&mut command_ctx).await;
        }

        if !self.dirty_regions.0.is_empty() {
            let dirty_regions = mem::take(&mut self.dirty_regions.0);

            tracing::debug!(count = dirty_regions.len(), "notifying dirty regions");
            for region in dirty_regions {
                if let Err(error) = self.should_refresh_tx.send(region).await {
                    tracing::warn!("failed to send refresh signal: {error:?}")
                }
            }
        }
    }

    async fn handle_getter(&self, action: GetterAction<A::RootModel>) {
        action(&self.model).await
    }
}

impl<A: Application> MvuRuntime<A> {
    pub fn dispatcher(&self) -> Dispatcher<A::RootModel> {
        self.dispatcher.clone()
    }
}

#[derive(Default)]
pub struct World(HashMap<TypeId, Box<dyn Any>>);

impl World {
    pub(crate) fn add_with<S: Any>(mut self, state: S) -> Self {
        self.0.insert(state.type_id(), Box::new(state));
        self
    }

    pub(crate) fn add<S: Any + Default>(self) -> Self {
        self.add_with(S::default())
    }

    pub fn try_get<S: Any>(&self) -> Option<&S> {
        self.0
            .get(&TypeId::of::<S>())
            .and_then(|s| s.downcast_ref())
    }

    pub fn get<S: Any>(&self) -> &S {
        self.try_get()
            .unwrap_or_else(|| panic!("`{}` does not exist in the world", type_name::<S>()))
    }

    pub fn try_get_mut<S: Any>(&mut self) -> Option<&mut S> {
        self.0
            .get_mut(&TypeId::of::<S>())
            .and_then(|s| s.downcast_mut())
    }

    pub fn get_mut<S: Any>(&mut self) -> &mut S {
        self.try_get_mut()
            .unwrap_or_else(|| panic!("`{}` does not exist in the world", type_name::<S>()))
    }
}

pub struct MvuRuntimeBuilder<A: Application> {
    model: Option<A::RootModel>,
    world: World,
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

    pub fn state_with<S: Any>(self, value: S) -> Self {
        Self {
            world: self.world.add_with(value),
            ..self
        }
    }

    pub fn state<S: Any + Default>(self) -> Self {
        Self {
            world: self.world.add::<S>(),
            ..self
        }
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

    pub fn build(self) -> (MvuRuntime<A>, ShouldRefreshSubscriber<A>) {
        let model = self.model.expect("RootModel was not initialized");

        let (action_tx, action_rx) = mpsc::channel(self.buffer_size);
        let (should_refresh_tx, should_refresh_rx) = mpsc::channel(self.buffer_size);

        (
            MvuRuntime {
                model,
                world: self.world,
                queue: CommandQueue::default(),
                dirty_regions: DirtyRegions::default(),
                dispatcher: Dispatcher::new_root(action_tx),
                action_rx,
                should_refresh_tx,
            },
            ShouldRefreshSubscriber(should_refresh_rx),
        )
    }
}

impl<A: Application> Default for MvuRuntimeBuilder<A> {
    fn default() -> Self {
        Self {
            model: None,
            world: World::default(),
            buffer_size: DEFAULT_CHANNEL_BUFFER_SIZE,
        }
    }
}
