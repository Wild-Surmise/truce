//! Platform window bridging for baseview - re-exports from
//! `truce_gui::platform`.
//!
//! iOS uses the stub editor in `editor_ios.rs` and wasm uses
//! `editor_web.rs`; neither goes through the baseview / wgpu path, so
//! the heavier re-exports (`ParentWindow`, `create_wgpu_surface`) gate
//! to desktop only. `query_backing_scale` has no wasm arm in
//! `truce_gui::platform` either (the embedder queries the browser's
//! device-pixel-ratio itself), so it gates the same way.

#[cfg(all(not(target_os = "ios"), not(target_arch = "wasm32")))]
pub use truce_gui::platform::{ParentWindow, create_wgpu_surface};

#[cfg(not(target_arch = "wasm32"))]
pub use truce_gui::platform::query_backing_scale;
