use crate::base::{Application, Command, Model, ModelGetter, RootModelGetter};
use crate::dispatcher::MessageDispatcher;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use std::collections::{HashSet, VecDeque};
use std::mem;
use std::ops::ControlFlow;
use crate::DispatcherExt;

const DEFAULT_CHANNEL_BUFFER_SIZE: usize = 64;

type RootMessage<A> = <<A as Application>::RootModel as Model>::Message;
type ModelGetterMessage<A> =
    <<<A as Application>::RootModel as Model>::Getter as ModelGetter>::Message;

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
    pub fn mark(&mut self, region: A::RegionId) {
        self.0.insert(region);
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
        self.dirty_regions.mark(region);
    }
}

pub struct ApplyContext<'rt, A: Application> {
    pub model: &'rt mut A::RootModel,
    pub state: &'rt mut A::State,
    pub message_dispatcher: &'rt MessageDispatcher<A>,
}

impl<'rt, A: Application> ApplyContext<'rt, A> {
    pub async fn dispatch_message(&self, message: RootMessage<A>) {
        self.message_dispatcher.dispatch(message).await
    }
}

pub struct ShouldRefreshSubscriber<A: Application>(mpsc::Receiver<A::RegionId>);

impl<A: Application> ShouldRefreshSubscriber<A> {
    pub async fn recv(&mut self) -> Option<A::RegionId> {
        self.0.next().await
    }
}

pub struct MvuRuntime<A: Application> {
    model: A::RootModel,
    state: A::State,

    queue: CommandQueue<A>,
    dirty_regions: DirtyRegions<A>,

    message_dispatcher: MessageDispatcher<A>,
    message_rx: mpsc::Receiver<RootMessage<A>>,

    model_getter_tx: mpsc::Sender<ModelGetterMessage<A>>,
    model_getter_rx: mpsc::Receiver<ModelGetterMessage<A>>,

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
        A::State: Default,
    {
        MvuRuntimeBuilder::defaults().build()
    }
}

impl<A: Application> MvuRuntime<A> {
    pub fn message_dispatcher(&self) -> MessageDispatcher<A> {
        self.message_dispatcher.clone()
    }

    pub fn model_getter(&self) -> <A::RootModel as Model>::Getter {
        <_>::new(self.model_getter_tx.clone())
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
        futures::select! {
            message = self.message_rx.next() => match message {
                Some(message) => self.handle_message(message).await,
                None => {
                    tracing::debug!("message channel closed");
                    return ControlFlow::Break(());
                },
            },
            message = self.model_getter_rx.next() => match message {
                Some(message) => self.model.getter(message),
                None => {
                    tracing::debug!("model getter channel closed");
                    return ControlFlow::Break(())
                },
            },
            complete => return ControlFlow::Break(()),
        }

        ControlFlow::Continue(())
    }

    #[tracing::instrument(skip(self, message), fields(msg_type = ?std::any::type_name::<RootMessage<A>>()))]
    async fn handle_message(&mut self, message: RootMessage<A>) {
        tracing::debug!(?message, "handling message");

        let mut update_ctx = UpdateContext {
            queue: &mut self.queue,
            dirty_regions: &mut self.dirty_regions,
        };
        self.model.update(message, &mut update_ctx);

        let mut command_ctx = ApplyContext {
            model: &mut self.model,
            state: &mut self.state,
            message_dispatcher: &self.message_dispatcher,
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
}

pub struct MvuRuntimeBuilder<A: Application> {
    model: Option<A::RootModel>,
    state: Option<A::State>,
    buffer_size: usize,
}

impl<A: Application> MvuRuntimeBuilder<A> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn defaults() -> Self
    where
        A::RootModel: Default,
        A::State: Default,
    {
        Self::new().default_model().default_state()
    }

    pub fn model(self, value: A::RootModel) -> Self {
        Self {
            model: Some(value),
            ..self
        }
    }

    pub fn state(self, value: A::State) -> Self {
        Self {
            state: Some(value),
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

    pub fn default_state(self) -> Self
    where
        A::State: Default,
    {
        self.state(Default::default())
    }

    pub fn build(self) -> (MvuRuntime<A>, ShouldRefreshSubscriber<A>) {
        let model = self.model.expect("RootModel was not initialized");
        let state = self.state.expect("State was not initialized");

        let (message_tx, message_rx) = mpsc::channel(self.buffer_size);
        let (model_getter_tx, model_getter_rx) = mpsc::channel(self.buffer_size);
        let (should_refresh_tx, should_refresh_rx) = mpsc::channel(self.buffer_size);

        (
            MvuRuntime {
                model,
                state,
                queue: CommandQueue::default(),
                dirty_regions: DirtyRegions::default(),
                message_dispatcher: MessageDispatcher(message_tx),
                message_rx,
                model_getter_tx,
                model_getter_rx,
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
            state: None,
            buffer_size: DEFAULT_CHANNEL_BUFFER_SIZE,
        }
    }
}
