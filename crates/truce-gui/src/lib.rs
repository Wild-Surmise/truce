//! Built-in GPU-free GUI for truce plugins.
//!
//! Uses a `RenderBackend` trait to abstract over rendering implementations.
//! The default `CpuBackend` uses tiny-skia for software rasterization.

// Widget-drawing helpers, `RenderBackend` trait methods, and interaction
// dispatch all take many independent geometry / state / theme arguments.
// Bundling them into builder structs is a worthwhile refactor but a
// separate change — for now the long signatures are intentional.
#![allow(clippy::too_many_arguments)]

pub mod backend_cpu;
pub mod blit;
pub mod editor;
pub mod font;
pub mod interaction;
pub mod layout;
#[macro_use]
pub mod macros;
pub mod platform;
pub mod plugin_logic;
pub mod render;
pub mod snapshot;
pub mod theme;
pub mod widgets;

pub use editor::BuiltinEditor;
pub use platform::{EditorScale, to_physical_px};
pub use plugin_logic::{PluginLogic, PluginLogic64, PluginLogicCore, default_hit_test};

// Re-exported at the crate root so the `plugin_logic_leaf_trait!`
// macro's `$crate::__plugin_logic_deps::*` paths resolve when the
// macro is invoked outside `truce_gui` (the macro is `#[macro_export]`
// and could in principle be called from a downstream crate that
// wants to declare its own sample-pinned leaf trait).
#[doc(hidden)]
pub use plugin_logic::__plugin_logic_deps;
pub use render::{ImageId, RenderBackend};
pub use snapshot::ParamSnapshot;
pub use theme::Theme;

/// Get the display scale factor used to size the next editor.
///
/// Screenshot rendering pins this to a deterministic value via
/// [`truce_core::screenshot::override_scale`] (default 2.0) so a
/// reference PNG baked on one host renders at the same physical
/// dimensions on any other. Outside screenshot rendering the
/// override is unset and we return the platform's main-screen DPI
/// query (Retina = 2.0, normal = 1.0).
#[must_use]
pub fn backing_scale() -> f64 {
    if let Some(s) = truce_core::screenshot::override_scale() {
        return s;
    }
    platform::main_screen_scale()
}
