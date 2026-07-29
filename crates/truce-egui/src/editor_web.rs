//! Web (`wasm32`) editor surface.
//!
//! Unlike the desktop (`editor.rs`) and iOS (`editor_ios.rs`) paths, there is
//! no `EguiEditor` here: on the web there is no plugin host to hand an
//! `Editor` to, and no window to own. The embedder creates the canvas, the
//! wgpu surface and the egui context itself, then drives an `EditorUi`
//! directly — exactly as the headless screenshot harnesses do.
//!
//! This module therefore exports only the `EditorUi` trait, so that a single
//! plugin editor type can satisfy the desktop, iOS and web backends without
//! conditional compilation in the plugin crate.

use truce_core::editor::PluginContext;
use truce_params::Params;

/// Stateful plugin editor UI. Mirrors the desktop and iOS definitions in
/// `editor.rs` and `editor_ios.rs`.
pub trait EditorUi<P: Params + ?Sized>: Send {
    fn ui(&mut self, ui: &mut egui::Ui, state: &PluginContext<P>);
    fn opened(&mut self, _state: &PluginContext<P>) {}
    fn state_changed(&mut self, _state: &PluginContext<P>) {}
}

impl<P: Params + ?Sized, F: FnMut(&mut egui::Ui, &PluginContext<P>) + Send> EditorUi<P> for F {
    fn ui(&mut self, ui: &mut egui::Ui, state: &PluginContext<P>) {
        self(ui, state);
    }
}
