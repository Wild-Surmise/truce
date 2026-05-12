//! Hot-reload mechanics for truce: dylib loading, ABI canary,
//! vtable probe, and the shells (`HotShell<P, S>`, `StaticShell<P, L, S>`)
//! that bridge the user-facing `truce_plugin::PluginLogic` /
//! `truce_plugin::PluginLogic64` leaf traits onto
//! [`truce_core::Plugin`] for format wrappers.
//!
//! Plugin authors don't reach into this crate directly. They write
//! `impl PluginLogic for MyPlugin` (the leaf trait is sample-pinned
//! via the prelude re-export) and the `truce::plugin!` macro picks
//! the static or hot shell based on the `shell` Cargo feature.
//!
//! # ABI boundary
//!
//! Across the dylib boundary the shell holds a
//! `Box<dyn truce_plugin::PluginLogicCore<S>>` — the generic
//! wrapper-facing trait that both leaf traits forward into via
//! blanket impls in `truce-plugin`. The single trait object
//! carries DSP and GUI methods through one vtable, with `S` baked
//! in by the shell's generic parameter (and recorded in
//! `AbiCanary::sample_precision` so a precision mismatch fails
//! the canary check before vtable-binding).
//!
//! ```ignore
//! use truce_loader::prelude::*;
//!
//! struct MyPlugin { /* ... */ }
//! impl PluginLogic for MyPlugin { /* DSP + GUI */ }
//!
//! // Emitted by `truce::plugin!`; plugin authors don't write these
//! // by hand. `Sample` resolves through the prelude alias
//! // (`f32` for `prelude` / `prelude32` / `prelude64m`,
//! // `f64` for `prelude64`).
//! #[unsafe(no_mangle)]
//! pub fn truce_create(p: *const ()) -> Box<dyn PluginLogicCore<Sample>> {
//!     Box::new(MyPlugin::new(/* params from p */))
//! }
//!
//! #[unsafe(no_mangle)]
//! pub fn truce_abi_canary() -> AbiCanary { AbiCanary::current::<Sample>() }
//!
//! #[unsafe(no_mangle)]
//! pub fn truce_vtable_probe() -> Box<dyn PluginLogicCore<Sample>> {
//!     Box::new(ProbePlugin::default())
//! }
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

pub use canary::{AbiCanary, ProbePlugin};
// `verify_probe` + `ProbeError` are loader-internal and only used
// by `NativeLoader::build_candidate` (gated on `feature = "shell"`).
// Plugin authors / format wrappers reach the probe via the
// `export_plugin!` macro's emitted `truce_vtable_probe` symbol
// path, which dlopens by name rather than by `use` import.
// `loader.rs` imports them directly from `crate::canary`; no
// crate-root re-export needed.
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
