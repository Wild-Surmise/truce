//! Hot-reload mechanics for truce: dylib loading, ABI canary,
//! vtable probe, and the shells (`HotShell`, `StaticShell`) that
//! bridge the user-facing [`truce_gui::PluginLogic`] trait onto
//! [`truce_core::Plugin`] for format wrappers.
//!
//! Plugin authors don't reach into this crate directly. They write
//! `impl PluginLogic for MyPlugin` and the `truce::plugin!` macro
//! picks the static or hot shell based on the `shell` Cargo feature.
//!
//! # ABI boundary
//!
//! Across the dylib boundary the shell holds a
//! `Box<dyn truce_gui::PluginLogic>`. The trait combines DSP and GUI
//! surfaces in one object so a single vtable crosses the boundary.
//!
//! ```ignore
//! use truce_loader::prelude::*;
//!
//! struct MyPlugin { /* ... */ }
//! impl PluginLogic for MyPlugin { /* DSP + GUI */ }
//!
//! #[unsafe(no_mangle)]
//! pub fn truce_create() -> Box<dyn PluginLogic> { Box::new(MyPlugin::new()) }
//!
//! #[unsafe(no_mangle)]
//! pub fn truce_abi_canary() -> AbiCanary { AbiCanary::current() }
//!
//! #[unsafe(no_mangle)]
//! pub fn truce_vtable_probe() -> Box<dyn PluginLogic> { Box::new(ProbePlugin::default()) }
//! ```

#[doc(hidden)]
pub mod __macro_deps {
    pub use truce_core;
    pub use truce_gui;
}

mod canary;
mod safe_types;

#[cfg(feature = "shell")]
mod loader;
#[cfg(feature = "shell")]
pub mod shell;
pub mod static_shell;

pub use canary::{AbiCanary, ProbePlugin, verify_probe};
pub use safe_types::*;
pub use truce_gui::{PluginLogic, PluginLogic64, PluginLogicCore};

#[cfg(feature = "shell")]
pub use loader::NativeLoader;

/// Export the `#[unsafe(no_mangle)]` functions required by the shell.
///
/// `params_ptr` is a raw `Arc<Params>` pointer from the shell.
/// The plugin receives shared params — one copy, no sync.
#[macro_export]
macro_rules! export_plugin {
    ($logic:ty, $params:ty) => {
        // `Sample` here is the prelude's `type Sample = …` alias —
        // `f32` for `prelude` / `prelude32` / `prelude64m`, `f64` for
        // `prelude64`. Lets `prelude64` plugins compile through the
        // dylib export path even though `HotShell` (the loader-side
        // consumer) is currently `f32`-only — a `prelude64` plugin
        // would simply error at dylib-load time if someone tried to
        // hot-reload it, rather than failing to compile in static mode.
        #[unsafe(no_mangle)]
        pub fn truce_create(params_ptr: *const ()) -> Box<dyn $crate::PluginLogicCore<Sample>> {
            let params: Arc<$params> = unsafe {
                Arc::increment_strong_count(params_ptr as *const $params);
                Arc::from_raw(params_ptr as *const $params)
            };
            // The plugin impls one of the leaf traits
            // (`PluginLogic` for f32 or `PluginLogic64` for f64); the
            // blanket impl inside `truce-gui` gives it
            // `PluginLogicCore<Sample>` automatically, so the cast
            // here just picks the right vtable.
            Box::new(<$logic>::new(params))
        }

        #[unsafe(no_mangle)]
        pub fn truce_abi_canary() -> $crate::AbiCanary {
            // `Sample` from the prelude — the dylib stamps its
            // chosen precision into the canary so the shell can
            // reject a mismatched load before vtable-binding.
            $crate::AbiCanary::current::<Sample>()
        }

        #[unsafe(no_mangle)]
        pub fn truce_vtable_probe() -> Box<dyn $crate::PluginLogicCore<Sample>> {
            Box::new($crate::ProbePlugin::default())
        }
    };
}

/// Convenience prelude for logic dylib authors.
pub mod prelude {
    pub use crate::canary::{AbiCanary, ProbePlugin};
    pub use crate::safe_types::*;
    pub use crate::{PluginLogic, PluginLogic64, PluginLogicCore};

    // Re-export param types so the developer can own params in their struct.
    pub use truce_params::{BoolParam, EnumParam, FloatParam, IntParam, ParamEnum, Params};
    pub use truce_params::{Smoother, SmoothingStyle};

    // Re-export utility functions.
    pub use truce_core::util::{db_to_linear, linear_to_db, midi_note_to_freq};
}
