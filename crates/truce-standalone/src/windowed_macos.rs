//! macOS window helpers for standalone hosts.

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle as RwhHandle};
use truce_core::editor::ResizeConstraints;

/// Make baseview's outer standalone `NSWindow` user-resizable.
///
/// baseview opens standalone windows with titled/closable/miniaturizable
/// style masks only. Resizable plugin editors need the native `Resizable`
/// style bit on the parent window so AppKit exposes corner/edge resize
/// cursors and sends normal resize events.
pub fn enable_resize(window: &impl HasRawWindowHandle, constraints: ResizeConstraints) {
    use objc::{msg_send, sel, sel_impl};

    let RwhHandle::AppKit(handle) = window.raw_window_handle() else {
        return;
    };
    if handle.ns_window.is_null() {
        return;
    }

    unsafe {
        let ns_window = handle.ns_window.cast::<objc::runtime::Object>();

        // NSWindowStyleMaskResizable = 1 << 3.
        let style: usize = msg_send![ns_window, styleMask];
        let _: () = msg_send![ns_window, setStyleMask: style | (1usize << 3)];

        let min_size = NSSize {
            width: f64::from(constraints.min_width),
            height: f64::from(constraints.min_height),
        };
        let max_size = NSSize {
            width: f64::from(constraints.max_width),
            height: f64::from(constraints.max_height),
        };
        let _: () = msg_send![ns_window, setContentMinSize: min_size];
        let _: () = msg_send![ns_window, setContentMaxSize: max_size];
    }
}

/// Current outer standalone window content size in logical points.
pub fn content_size(window: &impl HasRawWindowHandle) -> Option<(u32, u32)> {
    use objc::{msg_send, sel, sel_impl};

    let RwhHandle::AppKit(handle) = window.raw_window_handle() else {
        return None;
    };
    if handle.ns_window.is_null() {
        return None;
    }

    unsafe {
        let ns_window = handle.ns_window.cast::<objc::runtime::Object>();
        let content_view: *mut objc::runtime::Object = msg_send![ns_window, contentView];
        if content_view.is_null() {
            return None;
        }

        let frame: NSRect = msg_send![content_view, frame];
        Some((
            frame.size.width.round().max(1.0) as u32,
            frame.size.height.round().max(1.0) as u32,
        ))
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}
