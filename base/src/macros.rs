#[cfg(feature = "frb-compat")]
#[macro_export]
macro_rules! wrap_app_handle_for_frb {
    (
        $(#[$($meta:meta)*])*
        $vis:vis struct $AppHandleWrapper:ident
        where
            Application = $Application:ty,
            SplittableWrappedDispatcher = $SplittableWrappedDispatcher:ty,
            WrappedUpdater = $WrappedUpdater:ty,
            WrappedGetter = $WrappedGetter:ty,
            StreamSink = $StreamSink:ident,
            RegionId = $RegionId:ty,
        {
            let builder_fn = | $builder:ident $(,)? $($bfn_arg:ident: $bfn_arg_ty:ty),* | $builder_fn:block

            $(#[$($new_meta:meta)*])*
            $new_vis:vis fn new;

            $(#[$($dispatcher_meta:meta)*])*
            $dispatcher_vis:vis fn dispatcher;

            $(#[$($updater_meta:meta)*])*
            $updater_vis:vis fn updater;

            $(#[$($getter_meta:meta)*])*
            $getter_vis:vis fn getter;

            $(#[$($should_refresh_meta:meta)*])*
            $should_refresh_vis:vis fn should_refresh;
        }
    ) => {
        #[$crate::__macros::frb(opaque)]
        $(#[$($meta)*])*
        $vis struct $AppHandleWrapper($crate::AppHandle<$Application, $SplittableWrappedDispatcher>);

        impl $AppHandleWrapper {
            $(#[$($new_meta)*])*
            $new_vis async fn new($($bfn_arg: $bfn_arg_ty),*) -> Self {
                let builder_fn = |$builder: $crate::MvuRuntimeBuilder<$Application>, $($bfn_arg: $bfn_arg_ty),*| $builder_fn;
                Self($crate::handle::AppHandle::new::<$crate::handle::FrbSpawner>(|$builder| builder_fn($builder, $($bfn_arg),*)))
            }

            #[$crate::__macros::frb(sync, getter)]
            $(#[$($dispatcher_meta)*])*
            $dispatcher_vis fn dispatcher(&self) -> $SplittableWrappedDispatcher {
                self.0.dispatcher()
            }

            #[$crate::__macros::frb(sync, getter)]
            $(#[$($updater_meta)*])*
            $updater_vis fn updater(&self) -> $WrappedUpdater {
                self.0.updater()
            }

            #[$crate::__macros::frb(sync, getter)]
            $(#[$($getter_meta)*])*
            $getter_vis fn getter(&self) -> $WrappedGetter {
                self.0.getter()
            }

            #[$crate::__macros::frb(sync)]
            $(#[$($should_refresh_meta)*])*
            $should_refresh_vis async fn should_refresh(&mut self, sink: $StreamSink<$RegionId>) -> $crate::__macros::anyhow::Result<()> {
                let mut subscriber = self.0.should_refresh();
                $crate::__macros::flutter_rust_bridge::spawn(async move {
                    while let Some(region) = $crate::__macros::futures::StreamExt::next(&mut subscriber).await {
                        sink.add(region).ok();
                    }
                });
                ::core::result::Result::Ok(())
            }
        }
    };
}