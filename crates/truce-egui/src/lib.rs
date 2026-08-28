//! egui-based GUI backend for truce audio plugins.
//!
//! Provides `EguiEditor`, an implementation of `truce_core::Editor` that
//! renders using egui's immediate-mode UI via egui-wgpu. Gives plugin
//! developers access to egui's full widget library, layout system, and
//! ecosystem while retaining truce's parameter binding and host integration.
//!
//! # Quick Start
//!
//! ```ignore
//! use truce_egui::EguiEditor;
//! use truce_egui::widgets::{param_knob, param_slider};
//! use truce_core::editor::PluginContext;
//!
//! let editor = EguiEditor::new(params, (800, 600), |ui: &mut egui::Ui, state: &PluginContext<MyParams>| {
//!     ui.heading("My Plugin");
//!     param_slider(ui, state, 0u32);
//! });
//! ```

// `editor.rs` is the baseview-driven desktop path; `editor_ios.rs`
// drives the UIView + CADisplayLink + CAMetalLayer host on iOS.
// `renderer.rs` (egui-wgpu wrapper) is shared - it has both a
// baseview-window and a raw-CAMetalLayer constructor.
#[cfg(not(target_os = "ios"))]
pub mod editor;
pub mod font;
pub mod platform;
#[cfg(target_os = "windows")]
mod render_thread;
pub mod renderer;
#[cfg(not(target_os = "ios"))]
mod screenshot;
pub mod theme;
pub mod widgets;

#[cfg(target_os = "ios")]
mod editor_ios;

#[cfg(not(target_os = "ios"))]
pub use editor::{EditorUi, EguiEditor};

#[cfg(target_os = "ios")]
pub use editor_ios::{EditorUi, EguiEditor};

fn actual_window_size_id() -> egui::Id {
    egui::Id::new("truce_egui_actual_window_size")
}

/// Stash the true OS window size (logical points) in egui ctx data so a
/// plugin's `ui()` can read it back via [`actual_window_size`].
///
/// This exists because the editor feeds `screen_rect = self.size / zoom`
/// to egui (so layout is consistent when a plugin applies
/// `ctx.set_zoom_factor`). A plugin that scales its UI from the window size
/// therefore cannot recover the real window size from `ctx.screen_rect()`
/// alone — egui also transiently rescales `screen_rect` on zoom-change
/// frames. Reading this out-of-band value avoids that resize feedback loop.
pub fn set_actual_window_size(ctx: &egui::Context, size: (u32, u32)) {
    ctx.data_mut(|data| data.insert_temp(actual_window_size_id(), size));
}

/// The true OS window size (logical points) as stored by the editor each
/// frame. See [`set_actual_window_size`]. Returns `None` before the first
/// frame has run.
#[must_use]
pub fn actual_window_size(ctx: &egui::Context) -> Option<(u32, u32)> {
    ctx.data(|data| data.get_temp(actual_window_size_id()))
}

/// Translate committed native text input into the egui events consumed by
/// text widgets on the next frame.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn native_text_input_events(text: &str) -> Vec<egui::Event> {
    if text.is_empty() {
        Vec::new()
    } else if matches!(text, "\n" | "\r" | "\r\n") {
        let modifiers = egui::Modifiers::default();
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .collect()
    } else {
        vec![egui::Event::Text(text.to_owned())]
    }
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn native_text_replacement_events(text: &str) -> Vec<egui::Event> {
    let command = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    let mut events = Vec::with_capacity(3);
    push_key(&mut events, egui::Key::A, command);
    if text.is_empty() {
        push_key(
            &mut events,
            egui::Key::Backspace,
            egui::Modifiers::default(),
        );
    } else {
        events.push(egui::Event::Text(text.to_owned()));
    }
    events
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeKeyboardAction {
    None,
    BecomeFirstResponder,
    ResignFirstResponder,
    SurrenderEguiFocus,
    DismissAndSurrender,
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn native_keyboard_action(
    wants_keyboard: bool,
    session_active: bool,
    is_first_responder: bool,
    dismiss_requested: bool,
) -> NativeKeyboardAction {
    if dismiss_requested {
        return NativeKeyboardAction::DismissAndSurrender;
    }
    match (wants_keyboard, session_active, is_first_responder) {
        (true, false, false) => NativeKeyboardAction::BecomeFirstResponder,
        (true, true, false) => NativeKeyboardAction::SurrenderEguiFocus,
        (false, _, true) => NativeKeyboardAction::ResignFirstResponder,
        _ => NativeKeyboardAction::None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct NativeTextSelection {
    pub start: usize,
    pub end: usize,
}

impl NativeTextSelection {
    pub(crate) fn normalized(self) -> Self {
        Self {
            start: self.start.min(self.end),
            end: self.start.max(self.end),
        }
    }
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn utf16_offset_for_char_index(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(char::len_utf16).sum()
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn char_index_for_utf16_offset(text: &str, utf16_offset: usize) -> usize {
    let mut consumed = 0;
    for (index, character) in text.chars().enumerate() {
        if consumed >= utf16_offset {
            return index;
        }
        consumed += character.len_utf16();
        if consumed >= utf16_offset {
            return index + 1;
        }
    }
    text.chars().count()
}

fn push_key(events: &mut Vec<egui::Event>, key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
    }
}

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn apply_native_selection_to_focused_text_edit(
    ctx: &egui::Context,
    selection: NativeTextSelection,
) -> bool {
    let Some(id) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };
    let Some(mut state) = egui::widgets::text_edit::TextEditState::load(ctx, id) else {
        return false;
    };
    let selection = selection.normalized();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(selection.start),
            egui::text::CCursor::new(selection.end),
        )));
    state.store(ctx, id);
    ctx.request_repaint();
    true
}

/// Execute egui commands that require an operating-system integration.
///
/// Rendering backends must consume these commands after each frame. In
/// particular, [`egui::Context::open_url`] only queues an `OpenUrl` command;
/// it does not launch the external system browser by itself.
pub(crate) fn handle_platform_output(platform_output: &egui::PlatformOutput) {
    for command in &platform_output.commands {
        if let egui::OutputCommand::OpenUrl(open_url) = command
            && let Err(error) = webbrowser::open(&open_url.url)
        {
            log::warn!("failed to open URL '{}': {error}", open_url.url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeKeyboardAction, NativeTextSelection, apply_native_selection_to_focused_text_edit,
        char_index_for_utf16_offset, native_keyboard_action, native_text_input_events,
        native_text_replacement_events, utf16_offset_for_char_index,
    };

    #[test]
    fn native_return_is_an_enter_key_press_not_literal_text() {
        let events = native_text_input_events("\n");

        assert!(matches!(
            events.as_slice(),
            [
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    ..
                },
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: false,
                    ..
                }
            ]
        ));
    }

    #[test]
    fn native_committed_text_stays_a_text_event() {
        assert!(matches!(
            native_text_input_events("https://example.com").as_slice(),
            [egui::Event::Text(text)] if text == "https://example.com"
        ));
    }

    #[test]
    fn native_shadow_text_replaces_the_focused_egui_value() {
        let events = native_text_replacement_events("https://example.com/xyz");

        assert!(matches!(
            events.as_slice(),
            [
                egui::Event::Key {
                    key: egui::Key::A,
                    pressed: true,
                    modifiers,
                    ..
                },
                egui::Event::Key {
                    key: egui::Key::A,
                    pressed: false,
                    ..
                },
                egui::Event::Text(text)
            ] if modifiers.command && text == "https://example.com/xyz"
        ));
    }

    #[test]
    fn external_responder_loss_surrenders_stale_egui_focus() {
        assert_eq!(
            native_keyboard_action(true, true, false, false),
            NativeKeyboardAction::SurrenderEguiFocus
        );
    }

    #[test]
    fn return_dismissal_overrides_stale_egui_keyboard_focus() {
        assert_eq!(
            native_keyboard_action(true, true, true, true),
            NativeKeyboardAction::DismissAndSurrender
        );
    }

    #[test]
    fn native_utf16_offsets_round_trip_non_bmp_characters() {
        let text = "a💡b";
        assert_eq!(utf16_offset_for_char_index(text, 2), 3);
        assert_eq!(char_index_for_utf16_offset(text, 3), 2);
        assert_eq!(char_index_for_utf16_offset(text, usize::MAX), 3);
    }

    #[test]
    fn native_selection_updates_text_edit_state_without_key_replay() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("native-selection-test");
        let mut text = "https://example.com/".to_owned();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .show(ui)
                .response
                .request_focus();
        });

        assert!(apply_native_selection_to_focused_text_edit(
            &ctx,
            NativeTextSelection { start: 8, end: 15 }
        ));
        let state = egui::widgets::text_edit::TextEditState::load(&ctx, id)
            .expect("focused text edit state");
        let range = state.cursor.char_range().expect("cursor range");
        assert_eq!(range.primary.index, 15);
        assert_eq!(range.secondary.index, 8);
    }

    #[test]
    fn open_url_is_emitted_as_a_platform_command() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |_| {
            ctx.open_url(egui::OpenUrl::new_tab("https://example.com/contact"));
        });

        assert!(matches!(
            output.platform_output.commands.as_slice(),
            [egui::OutputCommand::OpenUrl(open_url)]
                if open_url.url == "https://example.com/contact" && open_url.new_tab
        ));
    }
}
