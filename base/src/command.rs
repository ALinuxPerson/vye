use crate::maybe::{MaybeLocalBoxFuture, MaybeSend, MaybeSendSync};
use crate::{Application, CommandContext};
use core::fmt;
use core::fmt::Debug;
use core::marker::PhantomData;
use futures::FutureExt;

maybe_async_trait! {
    pub trait Command: Debug + MaybeSendSync {
        type ForApp: Application;

        async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>);
    }

    impl<C> Command for Option<C>
    where
        C: Command,
    {
        type ForApp = C::ForApp;

        async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>) {
            if let Some(this) = self {
                this.apply(ctx).await
            }
        }
    }
}

type DynCommandFnRepr<ForApp> =
    Box<dyn_Maybe!(SendSync for<'rt> CommandFnHelper<'rt, ForApp, Fut = MaybeLocalBoxFuture<'rt, ()>>)>;

pub type DynCommandFn<ForApp> = CommandFn<DynCommandFnRepr<ForApp>, ForApp>;

pub trait CommandFnHelper<'rt, ForApp: Application>: MaybeSendSync {
    type Fut: Future<Output = ()> + MaybeSend + 'rt;

    fn call(&mut self, ctx: &mut CommandContext<'rt, ForApp>) -> Self::Fut;
}

impl<'rt, F, Fut, ForApp> CommandFnHelper<'rt, ForApp> for F
where
    ForApp: Application,
    F: FnMut(&mut CommandContext<'rt, ForApp>) -> Fut + MaybeSendSync,
    Fut: Future<Output = ()> + MaybeSendSync + 'rt,
{
    type Fut = Fut;

    fn call(&mut self, ctx: &mut CommandContext<'rt, ForApp>) -> Self::Fut {
        self(ctx)
    }
}

struct MakeDyn<F> {
    f: F,
}

impl<'rt, F, ForApp> CommandFnHelper<'rt, ForApp> for MakeDyn<F>
where
    ForApp: Application,
    F: for<'a> CommandFnHelper<'a, ForApp>,
{
    type Fut = MaybeLocalBoxFuture<'rt, ()>;

    fn call(&mut self, ctx: &mut CommandContext<'rt, ForApp>) -> Self::Fut {
        let future = self.f.call(ctx);

        #[cfg(feature = "thread-safe")]
        let ret = future.boxed();

        #[cfg(not(feature = "thread-safe"))]
        let ret = future.boxed_local();

        ret
    }
}

pub struct CommandFn<F, ForApp>(F, PhantomData<ForApp>);

impl<F, ForApp> CommandFn<F, ForApp> {
    pub fn new(f: F) -> Self {
        Self(f, PhantomData)
    }
}

impl<ForApp> DynCommandFn<ForApp> {
    pub fn new_dyn<F>(f: F) -> Self
    where
        F: for<'rt> CommandFnHelper<'rt, ForApp> + 'static,
        ForApp: Application,
    {
        CommandFn::new(f).into_dyn()
    }
}

impl<F, ForApp> CommandFn<F, ForApp>
where
    F: for<'rt> CommandFnHelper<'rt, ForApp> + 'static,
    ForApp: Application,
{
    pub fn into_dyn(self) -> DynCommandFn<ForApp> {
        CommandFn::new(Box::new(MakeDyn { f: self.0 }))
    }
}

impl<'rt, ForApp: Application> CommandFnHelper<'rt, ForApp> for DynCommandFnRepr<ForApp>
{
    type Fut = MaybeLocalBoxFuture<'rt, ()>;

    fn call(&mut self, ctx: &mut CommandContext<'rt, ForApp>) -> Self::Fut {
        (**self).call(ctx)
    }
}

maybe_async_trait! {
    impl<F, ForApp> Command for CommandFn<F, ForApp>
    where
        F: for<'rt> CommandFnHelper<'rt, ForApp>,
        ForApp: Application,
    {
        type ForApp = ForApp;

        async fn apply(&mut self, ctx: &mut CommandContext<'_, Self::ForApp>) {
            self.0.call(ctx).await
        }
    }
}

impl<F, ForApp> Debug for CommandFn<F, ForApp> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CommandFn").finish_non_exhaustive()
    }
}

pub fn command<F, ForApp>(f: F) -> CommandFn<F, ForApp> {
    CommandFn::new(f)
}

pub fn dyn_command<F, ForApp>(f: F) -> DynCommandFn<ForApp>
where
    F: for<'rt> CommandFnHelper<'rt, ForApp> + 'static,
    ForApp: Application,
{
    DynCommandFn::new_dyn(f)
}