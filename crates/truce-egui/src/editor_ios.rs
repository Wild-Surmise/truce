//! egui editor on iOS: `CAMetalLayer`-backed `UIView` running
//! egui-wgpu, driven by `CADisplayLink`, with `UITouch` events
//! translated into `egui::RawInput`.
//!
//! The runtime `UIView` subclass overrides `+layerClass` to return
//! `CAMetalLayer` so the backing layer is a Metal layer wgpu can
//! draw into directly. Per-frame work runs `egui::Context::run` with
//! a `RawInput` built from pending touch events + screen size, then
//! hands the tessellated primitives to `EguiRenderer::render` which
//! presents the next surface texture. Touch handlers push
//! `egui::Event::PointerMoved` / `PointerButton{pressed: bool}` into
//! a shared queue the next tick drains.

#![cfg(target_os = "ios")]

use std::sync::{Arc, Mutex};

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, ClassBuilder, Sel};
use objc2::sel;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use truce_core::editor::{Editor, PluginContext, RawWindowHandle};
use truce_gui::ios::{TouchPhase, fnv1a_64, ivar_offset};
use truce_params::Params;

use crate::renderer::EguiRenderer;

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

pub struct EguiEditor<P: Params + ?Sized> {
    params: Arc<P>,
    size: (u32, u32),
    /// Resize-capability flag exposed via `Editor::can_resize`. The
    /// AU v3 view controller only fits the editor to the host's
    /// safe-area frame when this is `true`; the default keeps a
    /// fixed-size GUI pinned to its built size.
    can_resize: bool,
    /// `Editor::min_size` / `max_size` bounds. The AU shim clamps
    /// host-driven resizes against these before calling `set_size`.
    min_size: (u32, u32),
    max_size: (u32, u32),
    /// `Editor::aspect_ratio` lock (numerator, denominator). The AU
    /// shim's `fit_logical_size` clamps host-driven resizes to it.
    aspect_ratio: Option<(u32, u32)>,
    ui: Arc<Mutex<Box<dyn EditorUi<P>>>>,
    visuals: Option<egui::Visuals>,
    font: Option<&'static [u8]>,
    inner: Arc<Mutex<Option<Inner<P>>>>,
}

// SAFETY: see truce-gui::editor_ios for the symmetric rationale -
// UIKit + CADisplayLink + wgpu surface presentation all happen on
// the main thread, where the AUv3 host calls Editor methods.
unsafe impl<P: Params + ?Sized> Send for EguiEditor<P> {}

struct Inner<P: Params + ?Sized> {
    child_view: *mut AnyObject,
    text_input: *mut AnyObject,
    display_link: *mut AnyObject,
    logical_w: u32,
    logical_h: u32,
    scale: f32,
    egui_ctx: egui::Context,
    renderer: EguiRenderer,
    ui: Arc<Mutex<Box<dyn EditorUi<P>>>>,
    params: Arc<P>,
    context: PluginContext<P>,
    pending_events: Vec<egui::Event>,
    native_text: String,
    native_selection: crate::NativeTextSelection,
    native_selection_dirty: bool,
    keyboard_session_active: bool,
    keyboard_dismiss_requested: bool,
    last_pointer: egui::Pos2,
}

impl<P: Params + 'static> EguiEditor<P> {
    pub fn new(
        params: Arc<P>,
        size: (u32, u32),
        ui: impl FnMut(&mut egui::Ui, &PluginContext<P>) + Send + 'static,
    ) -> Self {
        Self::with_ui_impl(params, size, Box::new(ui))
    }

    pub fn with_ui_impl(params: Arc<P>, size: (u32, u32), ui: Box<dyn EditorUi<P>>) -> Self {
        Self {
            params,
            size,
            can_resize: false,
            min_size: (1, 1),
            max_size: (u32::MAX, u32::MAX),
            aspect_ratio: None,
            ui: Arc::new(Mutex::new(ui)),
            visuals: None,
            font: None,
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_ui(params: Arc<P>, size: (u32, u32), ui: impl EditorUi<P> + 'static) -> Self {
        Self::with_ui_impl(params, size, Box::new(ui))
    }

    #[must_use]
    pub fn with_visuals(mut self, visuals: egui::Visuals) -> Self {
        self.visuals = Some(visuals);
        self
    }

    #[must_use]
    pub fn with_font(mut self, font: &'static [u8]) -> Self {
        self.font = Some(font);
        self
    }

    /// Opt into host-driven resizing. When `true`, the AU v3 view
    /// controller fits the editor to the host plug-in pane's
    /// safe-area frame (driving `set_size` through the AU shim), so
    /// the editor reflows to the real device viewport instead of
    /// sitting at its built portrait size. The default (`false`)
    /// keeps a deliberately fixed-size GUI pinned. Mirrors the
    /// desktop `EguiEditor::resizable` so the same builder call works
    /// on every target.
    #[must_use]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.can_resize = resizable;
        self
    }

    /// Minimum logical-point size the editor accepts. The AU shim
    /// consults this before driving `set_size`. See [`Self::resizable`].
    #[must_use]
    pub fn min_size(mut self, min: (u32, u32)) -> Self {
        self.min_size = min;
        self
    }

    /// Maximum logical-point size the editor accepts. See
    /// [`Self::min_size`].
    #[must_use]
    pub fn max_size(mut self, max: (u32, u32)) -> Self {
        self.max_size = max;
        self
    }

    /// Lock the editor's aspect ratio as `(numerator, denominator)`.
    /// Host-driven resizes are clamped to it by the AU shim's
    /// `fit_logical_size`. See [`Self::resizable`].
    #[must_use]
    pub fn aspect_ratio(mut self, ratio: Option<(u32, u32)>) -> Self {
        self.aspect_ratio = ratio;
        self
    }

    /// No-op on iOS. See [`Self::resizable`].
    #[must_use]
    pub fn prefers_pow2(self, _prefers: bool) -> Self {
        self
    }
}

impl<P: Params + 'static> Editor for EguiEditor<P> {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn open(&mut self, parent: RawWindowHandle, context: PluginContext) {
        let RawWindowHandle::UiKit(parent_ptr) = parent else {
            log::warn!("EguiEditor (iOS) got non-UiKit parent handle");
            return;
        };
        if parent_ptr.is_null() {
            return;
        }
        let (lw, lh) = self.size;
        // `query_backing_scale(parent)` reads `parent.contentScaleFactor`
        // - but at `gui_open` time the AUv3 container UIView hasn't
        // been attached to a visible window yet, so the property
        // still returns its default 1.0 instead of the device's
        // actual scale. `main_screen_scale()` goes through
        // `UIScreen.mainScreen.scale` and returns 3.0 on iPhone
        // Retina regardless of the view hierarchy state. Without
        // this, egui paints into a 1x wgpu surface that
        // CoreAnimation upscales 3x with visibly grainy edges. Cap
        // high-density iPhones at 2x: native 3x is sharp but expensive
        // for a continuously animated Metal editor.
        let native_scale = truce_gui::platform::main_screen_scale();
        let scale = native_scale.clamp(1.0, IOS_MAX_RENDER_SCALE);
        // Physical-pixel math bounded by editor size × backing
        // scale (max ~4000 px in practice); the cast loss is
        // irrelevant. `scale` won't exceed 4.0 on any Apple device.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let scalef = scale as f32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let phys_w = (f64::from(lw) * scale).round() as u32;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let phys_h = (f64::from(lh) * scale).round() as u32;

        // Create the runtime UIView subclass + CAMetalLayer, attach
        // to the parent, return (view*, layer*, link*).
        // SAFETY: see install_editor_view's contract.
        let (view, layer, link, slot_ptr) =
            unsafe { install_editor_view::<P>(parent_ptr.cast(), lw, lh, scalef, &self.inner) };
        if view.is_null() || layer.is_null() {
            log::warn!("egui iOS: install_editor_view returned null");
            // A non-null `view` here means the ivar Arc + display link
            // were already set up before this null check tripped; tear
            // them down so the partial open doesn't leak.
            // SAFETY: `view`/`link` come straight from `install_editor_view`.
            unsafe { teardown_editor_view::<P>(view, link) };
            return;
        }

        // Build the egui-wgpu renderer from the metal layer.
        // SAFETY: layer outlives the renderer (held by the view, view
        // pinned via ivar Arc).
        let Some(renderer) =
            (unsafe { EguiRenderer::from_metal_layer(layer.cast(), phys_w, phys_h) })
        else {
            log::warn!("egui iOS: failed to create EguiRenderer from metal layer");
            // `install_editor_view` already pinned the ivar Arc, retained
            // the display link and scheduled it on the run loop; tear it
            // all down so a failed open doesn't leave a zombie display
            // link firing `tick:` with the view/layer/Arc graph leaked.
            // SAFETY: `view`/`link` come straight from `install_editor_view`.
            unsafe { teardown_editor_view::<P>(view, link) };
            return;
        };

        let egui_ctx = egui::Context::default();
        // Pin egui's logical→physical scale to the device backing
        // scale (3x on Retina iPhones). Without this, egui paints
        // at 1x pixels-per-point into a 3x-sized wgpu surface and
        // Core Animation upscales the result, visible as grainy
        // edges on every widget.
        egui_ctx.set_pixels_per_point(scalef);
        if let Some(v) = self.visuals.clone() {
            egui_ctx.set_visuals(v);
        }
        if let Some(font_bytes) = self.font {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "truce-egui".into(),
                Arc::new(egui::FontData::from_static(font_bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "truce-egui".into());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "truce-egui".into());
            egui_ctx.set_fonts(fonts);
        }

        let typed_ctx = context.with_params(Arc::clone(&self.params));
        self.ui
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .opened(&typed_ctx);

        let inner = Inner {
            child_view: view,
            text_input: unsafe { install_text_input_proxy::<P>(view) },
            display_link: link,
            logical_w: lw,
            logical_h: lh,
            scale: scalef,
            egui_ctx,
            renderer,
            ui: Arc::clone(&self.ui),
            params: Arc::clone(&self.params),
            context: typed_ctx,
            pending_events: Vec::with_capacity(16),
            native_text: String::new(),
            native_selection: crate::NativeTextSelection::default(),
            native_selection_dirty: false,
            keyboard_session_active: false,
            keyboard_dismiss_requested: false,
            last_pointer: egui::pos2(-1.0, -1.0),
        };
        *self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(inner);
        let _ = slot_ptr; // pin already taken into the ivar
    }

    fn close(&mut self) {
        let Some(inner) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        // SAFETY: `child_view`/`display_link` were built by
        // `install_editor_view` for this `P`; `take()` above guarantees
        // no other path touches them.
        unsafe { teardown_editor_view::<P>(inner.child_view, inner.display_link) };
        // EguiRenderer drops here, releasing wgpu surface / device / queue.
        drop(inner);
    }

    fn set_size(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        self.size = (width, height);
        // The AUv3 view controller calls `gui_set_size` on the main
        // thread from `viewDidLayoutSubviews`, the same thread the
        // `CADisplayLink` runs `run_frame` on, so resizing the live
        // view + surface inline here is safe (the tick and this never
        // nest - they take the same `inner` mutex on the same thread).
        if let Some(inner) = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            resize_inner(inner, width, height);
        }
        true
    }

    fn can_resize(&self) -> bool {
        self.can_resize
    }

    fn min_size(&self) -> (u32, u32) {
        self.min_size
    }

    fn max_size(&self) -> (u32, u32) {
        self.max_size
    }

    fn aspect_ratio(&self) -> Option<(u32, u32)> {
        self.aspect_ratio
    }
}

// UIView subclass with CAMetalLayer + CADisplayLink + touch handlers

const INNER_PTR_IVAR: &std::ffi::CStr = c"_truce_egui_inner_ptr";
/// Cap the `CADisplayLink` at 30 fps: a plugin editor doesn't need
/// the display's native 60/120 Hz, and the halved wake-up rate is a
/// meaningful battery / thermal win inside an AU v3 host.
const IOS_DISPLAY_LINK_FPS: isize = 30;
/// Cap the Metal backing scale. Native 3x on modern iPhones is sharp
/// but expensive for a continuously animated editor; 2x is the
/// quality/perf sweet spot.
const IOS_MAX_RENDER_SCALE: f64 = 2.0;

unsafe extern "C" {
    static NSRunLoopCommonModes: *const AnyObject;
    // AUv3 extension-host lifecycle (Foundation).
    static NSExtensionHostDidBecomeActiveNotification: *const AnyObject;
    static NSExtensionHostDidEnterBackgroundNotification: *const AnyObject;
    static NSExtensionHostWillEnterForegroundNotification: *const AnyObject;
    static NSExtensionHostWillResignActiveNotification: *const AnyObject;
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIApplicationDidBecomeActiveNotification: *const AnyObject;
    static UIApplicationDidEnterBackgroundNotification: *const AnyObject;
    static UIApplicationWillEnterForegroundNotification: *const AnyObject;
    static UIApplicationWillResignActiveNotification: *const AnyObject;
    static UISceneDidActivateNotification: *const AnyObject;
    static UISceneDidEnterBackgroundNotification: *const AnyObject;
    static UISceneWillDeactivateNotification: *const AnyObject;
    static UISceneWillEnterForegroundNotification: *const AnyObject;
}

/// `+[Class layerClass]` override that returns `CAMetalLayer`. Class
/// method (not instance), takes only `self` and `_cmd`; the return
/// is an `objc_class *`.
unsafe extern "C" fn layer_class_thunk(_cls: &AnyClass, _cmd: Sel) -> *const AnyClass {
    AnyClass::get(c"CAMetalLayer").expect("CAMetalLayer missing")
}

unsafe fn install_editor_view<P: Params + 'static>(
    parent: *mut AnyObject,
    logical_w: u32,
    logical_h: u32,
    scale: f32,
    slot: &Arc<Mutex<Option<Inner<P>>>>,
) -> (
    *mut AnyObject,
    *mut AnyObject,
    *mut AnyObject,
    *const Mutex<Option<Inner<P>>>,
) {
    use std::any::type_name;
    unsafe {
        let class_name_owned = format!(
            "TruceEguiiOSEditorView_{:x}",
            fnv1a_64(type_name::<Inner<P>>().as_bytes())
        );
        let class_name = std::ffi::CString::new(class_name_owned).expect("ascii");
        let uiview = AnyClass::get(c"UIView").expect("UIView missing");

        let cls: &AnyClass = if let Some(existing) = AnyClass::get(class_name.as_c_str()) {
            existing
        } else {
            let mut builder = ClassBuilder::new(class_name.as_c_str(), uiview)
                .expect("unique class name per monomorphization");
            builder.add_ivar::<*mut std::ffi::c_void>(INNER_PTR_IVAR);
            // `+layerClass` returns the class that backs every
            // instance's `layer` property. Returning `CAMetalLayer`
            // here makes `self.layer` a CAMetalLayer directly, so
            // wgpu can draw into it without manual sublayer
            // attachment + resize bookkeeping.
            builder.add_class_method(
                sel!(layerClass),
                layer_class_thunk as unsafe extern "C" fn(_, _) -> _,
            );
            builder.add_method(
                sel!(tick:),
                tick_thunk::<P> as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(trucePauseDisplayLink:),
                pause_display_link_notification::<P> as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(truceResumeDisplayLink:),
                resume_display_link_notification::<P> as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(touchesBegan:withEvent:),
                touches_began::<P> as unsafe extern "C" fn(_, _, _, _),
            );
            builder.add_method(
                sel!(touchesMoved:withEvent:),
                touches_moved::<P> as unsafe extern "C" fn(_, _, _, _),
            );
            builder.add_method(
                sel!(touchesEnded:withEvent:),
                touches_ended::<P> as unsafe extern "C" fn(_, _, _, _),
            );
            builder.add_method(
                sel!(touchesCancelled:withEvent:),
                touches_cancelled::<P> as unsafe extern "C" fn(_, _, _, _),
            );
            builder.add_method(
                sel!(truceTextFieldChanged:),
                text_field_changed::<P> as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(truceTextFieldReturn:),
                text_field_return::<P> as unsafe extern "C" fn(_, _, _),
            );
            // `UIKeyInput` conformance - implemented as raw selector
            // additions because ObjC protocols are duck-typed at
            // dispatch time. The runtime calls `respondsToSelector:`
            // before delivering keyboard events; presence of these
            // three selectors is what flips the view into a usable
            // text input. `canBecomeFirstResponder` overrides
            // `UIResponder`'s default `NO` so `becomeFirstResponder`
            // actually presents the soft keyboard.
            builder.add_method(
                sel!(canBecomeFirstResponder),
                can_become_first_responder as unsafe extern "C" fn(_, _) -> Bool,
            );
            builder.add_method(
                sel!(hasText),
                has_text as unsafe extern "C" fn(_, _) -> Bool,
            );
            builder.add_method(
                sel!(insertText:),
                insert_text::<P> as unsafe extern "C" fn(_, _, _),
            );
            builder.add_method(
                sel!(deleteBackward),
                delete_backward::<P> as unsafe extern "C" fn(_, _),
            );
            // Explicitly declare `UIKeyInput` conformance. UIKit
            // checks `[obj conformsToProtocol:@protocol(UIKeyInput)]`
            // before presenting the soft keyboard for a first
            // responder - just implementing the three selectors via
            // `respondsToSelector:` isn't sufficient. Same `UIView`
            // also has to implement `UITextInputTraits` (which
            // `UIKeyInput` inherits from); empty trait methods
            // default to sensible values so no extra selectors
            // needed.
            if let Some(proto) = AnyProtocol::get(c"UIKeyInput") {
                builder.add_protocol(proto);
            }
            if let Some(proto) = AnyProtocol::get(c"UITextInputTraits") {
                builder.add_protocol(proto);
            }
            builder.register()
        };

        let frame = NSRect {
            origin: NSPoint { x: 0.0, y: 0.0 },
            size: NSSize {
                width: f64::from(logical_w),
                height: f64::from(logical_h),
            },
        };
        let alloc: *mut AnyObject = msg_send![cls, alloc];
        let view: *mut AnyObject = msg_send![alloc, initWithFrame: frame];
        if view.is_null() {
            return (
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
        }
        let _: () = msg_send![view, setUserInteractionEnabled: true];
        let _: () = msg_send![view, setContentScaleFactor: f64::from(scale)];

        // Reach into the view's layer (already a CAMetalLayer thanks
        // to +layerClass) and configure its drawable scale + size.
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setContentsScale: f64::from(scale)];
        let drawable_size = NSSize {
            width: f64::from(logical_w) * f64::from(scale),
            height: f64::from(logical_h) * f64::from(scale),
        };
        let _: () = msg_send![layer, setDrawableSize: drawable_size];

        // Pin the Arc into the ivar (released in close() via
        // `Arc::from_raw`).
        let leaked: *const Mutex<Option<Inner<P>>> = Arc::into_raw(Arc::clone(slot));
        let base = view.cast::<u8>();
        let ivar_ptr: *mut *mut std::ffi::c_void =
            base.add(ivar_offset(cls, INNER_PTR_IVAR)).cast();
        *ivar_ptr = leaked as *mut std::ffi::c_void;

        let _: () = msg_send![parent, addSubview: view];

        // CADisplayLink → tick: every refresh.
        let dl_cls = AnyClass::get(c"CADisplayLink").expect("CADisplayLink missing");
        let link: *mut AnyObject =
            msg_send![dl_cls, displayLinkWithTarget: view, selector: sel!(tick:)];
        if link.is_null() {
            return (view, layer, std::ptr::null_mut(), leaked);
        }
        retain_obj(link);
        set_display_link_preferred_fps(link, IOS_DISPLAY_LINK_FPS);
        register_display_link_lifecycle_observers(view);
        let run_loop_cls = AnyClass::get(c"NSRunLoop").expect("NSRunLoop missing");
        let main: *mut AnyObject = msg_send![run_loop_cls, mainRunLoop];
        let mode: *const AnyObject = NSRunLoopCommonModes;
        let _: () = msg_send![link, addToRunLoop: main, forMode: mode];

        (view, layer, link, leaked)
    }
}

/// Install a native, off-screen `UITextField` that owns UIKit's text editing
/// session while egui continues to paint the visible widget. `UITextField`
/// already implements the complete `UITextInput` contract, including the
/// keyboard's hold-space cursor trackpad. `UIControl` edit/Return actions
/// mirror its value into egui; selection is synchronized once per display
/// frame in [`run_frame`].
unsafe fn install_text_input_proxy<P: Params + 'static>(
    editor_view: *mut AnyObject,
) -> *mut AnyObject {
    unsafe {
        if editor_view.is_null() {
            return std::ptr::null_mut();
        }

        let frame = NSRect {
            origin: NSPoint {
                x: -1000.0,
                y: -1000.0,
            },
            size: NSSize {
                width: 1.0,
                height: 1.0,
            },
        };
        let text_field = AnyClass::get(c"UITextField").expect("UITextField missing");
        let alloc: *mut AnyObject = msg_send![text_field, alloc];
        let input: *mut AnyObject = msg_send![alloc, initWithFrame: frame];
        if input.is_null() {
            return std::ptr::null_mut();
        }
        let _: () = msg_send![input, setAccessibilityElementsHidden: Bool::YES];
        let editing_changed: usize = 1 << 17;
        let editing_did_end_on_exit: usize = 1 << 19;
        let _: () = msg_send![
            input,
            addTarget: editor_view,
            action: sel!(truceTextFieldChanged:),
            forControlEvents: editing_changed
        ];
        let _: () = msg_send![
            input,
            addTarget: editor_view,
            action: sel!(truceTextFieldReturn:),
            forControlEvents: editing_did_end_on_exit
        ];
        let _: () = msg_send![editor_view, addSubview: input];
        // `addSubview:` retained the field. Balance alloc/init so its lifetime
        // is now exactly the editor view's lifetime.
        release_obj(input);
        input
    }
}

unsafe fn retain_obj(obj: *mut AnyObject) {
    unsafe {
        if !obj.is_null() {
            let _: *mut AnyObject = msg_send![obj, retain];
        }
    }
}

unsafe fn release_obj(obj: *mut AnyObject) {
    unsafe {
        if !obj.is_null() {
            let _: () = msg_send![obj, release];
        }
    }
}

unsafe fn set_display_link_preferred_fps(link: *mut AnyObject, fps: isize) {
    unsafe {
        if link.is_null() || fps <= 0 {
            return;
        }
        let _: () = msg_send![link, setPreferredFramesPerSecond: fps];
    }
}

/// Tear down the editor's `UIView` + `CADisplayLink`: unregister the
/// lifecycle observers, invalidate and release the display link,
/// reclaim the `Arc` pinned in the view's ivar, then detach the view.
/// Shared by `close()` and the early-return error paths in `open()`.
/// `install_editor_view` pins the ivar `Arc`, retains the link and
/// schedules it on the run loop *before* `open()` has a chance to fail,
/// so without this a failed open leaves a zombie display link firing
/// `tick:` forever with the view/layer/`Arc` graph leaked.
///
/// SAFETY: `child_view` (if non-null) must be a view built by
/// `install_editor_view` for the same `P` (so the ivar holds an
/// `Arc<Mutex<Option<Inner<P>>>>`), and `display_link` (if non-null)
/// the link returned alongside it. Both pointers are consumed - the
/// link is released and the ivar `Arc` reclaimed - so callers must not
/// reuse them afterwards. Must run on the main thread.
unsafe fn teardown_editor_view<P: Params + 'static>(
    child_view: *mut AnyObject,
    display_link: *mut AnyObject,
) {
    unsafe {
        if !display_link.is_null() {
            if !child_view.is_null() {
                unregister_display_link_lifecycle_observers(child_view);
            }
            let _: () = msg_send![display_link, invalidate];
            release_obj(display_link);
        }
        if !child_view.is_null() {
            // Reclaim the Arc the view's ivar holds, then null the ivar
            // *before* dropping it: the view outlives this call until its
            // own retain is released below, and a late selector delivery
            // (an in-flight responder/notification racing teardown) would
            // otherwise reconstruct a dangling Arc from freed memory.
            let cls: &AnyClass = msg_send![child_view, class];
            let base: *mut u8 = child_view.cast();
            let ivar_ptr: *mut *mut std::ffi::c_void =
                base.add(ivar_offset(cls, INNER_PTR_IVAR)).cast();
            let leaked = (*ivar_ptr).cast_const().cast::<Mutex<Option<Inner<P>>>>();
            *ivar_ptr = std::ptr::null_mut();
            if !leaked.is_null() {
                let _ = Arc::from_raw(leaked);
            }
            let _: () = msg_send![child_view, removeFromSuperview];
            // Balance the alloc/initWithFrame reference from
            // `install_editor_view`. `removeFromSuperview` dropped the
            // superview's retain and `invalidate` the display link's, so
            // this drops the last reference and frees the view + layer.
            release_obj(child_view);
        }
    }
}

unsafe fn borrow_inner_arc<P: Params + 'static>(
    self_: &AnyObject,
) -> Option<Arc<Mutex<Option<Inner<P>>>>> {
    unsafe {
        let cls: &AnyClass = msg_send![self_, class];
        let base: *const u8 = std::ptr::from_ref::<AnyObject>(self_).cast();
        let ivar_ptr: *const *mut std::ffi::c_void =
            base.add(ivar_offset(cls, INNER_PTR_IVAR)).cast();
        let leaked = (*ivar_ptr).cast_const().cast::<Mutex<Option<Inner<P>>>>();
        if leaked.is_null() {
            return None;
        }
        let arc = Arc::from_raw(leaked);
        let cloned = Arc::clone(&arc);
        let _ = Arc::into_raw(arc);
        Some(cloned)
    }
}

unsafe fn notification_center() -> *mut AnyObject {
    unsafe {
        let center_cls =
            AnyClass::get(c"NSNotificationCenter").expect("NSNotificationCenter missing");
        msg_send![center_cls, defaultCenter]
    }
}

unsafe fn add_notification_observer(
    center: *mut AnyObject,
    observer: *mut AnyObject,
    selector: Sel,
    name: *const AnyObject,
) {
    unsafe {
        if center.is_null() || observer.is_null() || name.is_null() {
            return;
        }
        let _: () = msg_send![
            center,
            addObserver: observer,
            selector: selector,
            name: name,
            object: std::ptr::null_mut::<AnyObject>(),
        ];
    }
}

/// Register the view for the app / scene / AUv3-extension lifecycle
/// notifications that pause and resume the `CADisplayLink`, so it stops
/// firing `tick:` (and the wgpu present that goes with it) whenever the
/// editor is hidden or the host is backgrounded.
unsafe fn register_display_link_lifecycle_observers(view: *mut AnyObject) {
    unsafe {
        let center = notification_center();
        let pause = sel!(trucePauseDisplayLink:);
        let resume = sel!(truceResumeDisplayLink:);

        // Standalone app / scene lifecycle.
        add_notification_observer(
            center,
            view,
            pause,
            UIApplicationWillResignActiveNotification,
        );
        add_notification_observer(
            center,
            view,
            pause,
            UIApplicationDidEnterBackgroundNotification,
        );
        add_notification_observer(center, view, pause, UISceneWillDeactivateNotification);
        add_notification_observer(center, view, pause, UISceneDidEnterBackgroundNotification);
        add_notification_observer(
            center,
            view,
            resume,
            UIApplicationDidBecomeActiveNotification,
        );
        add_notification_observer(
            center,
            view,
            resume,
            UIApplicationWillEnterForegroundNotification,
        );
        add_notification_observer(center, view, resume, UISceneDidActivateNotification);
        add_notification_observer(center, view, resume, UISceneWillEnterForegroundNotification);

        // AUv3 extension host lifecycle.
        add_notification_observer(
            center,
            view,
            pause,
            NSExtensionHostWillResignActiveNotification,
        );
        add_notification_observer(
            center,
            view,
            pause,
            NSExtensionHostDidEnterBackgroundNotification,
        );
        add_notification_observer(
            center,
            view,
            resume,
            NSExtensionHostDidBecomeActiveNotification,
        );
        add_notification_observer(
            center,
            view,
            resume,
            NSExtensionHostWillEnterForegroundNotification,
        );
    }
}

unsafe fn unregister_display_link_lifecycle_observers(view: *mut AnyObject) {
    unsafe {
        let center = notification_center();
        if !center.is_null() && !view.is_null() {
            let _: () = msg_send![center, removeObserver: view];
        }
    }
}

unsafe fn set_display_link_paused<P: Params + 'static>(view: &AnyObject, paused: bool) {
    unsafe {
        let Some(arc) = borrow_inner_arc::<P>(view) else {
            return;
        };
        let guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_ref() else { return };
        if inner.display_link.is_null() {
            return;
        }
        let _: () = msg_send![inner.display_link, setPaused: Bool::new(paused)];
    }
}

/// Run an iOS `extern "C"` thunk body under `catch_unwind`. `UIKit` invokes
/// these selectors across an Obj-C boundary that can't carry a Rust unwind;
/// an escaping panic (a bug in author UI code - a failed `unwrap`, an
/// out-of-bounds index, a tripped assertion) would become an uncaught Obj-C
/// exception and abort the `AUv3` host. Swallow and log it instead, matching
/// the desktop handlers. (An allocation failure aborts through
/// `handle_alloc_error` without unwinding, so `catch_unwind` can't cover it.)
fn ffi_firewall(label: &str, f: impl FnOnce()) {
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let msg = e
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        log::error!("truce-egui iOS {label} thunk panic swallowed: {msg}");
    }
}

unsafe extern "C" fn pause_display_link_notification<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    _notification: *mut AnyObject,
) {
    ffi_firewall("pause_display_link", || unsafe {
        set_display_link_paused::<P>(self_, true);
    });
}

unsafe extern "C" fn resume_display_link_notification<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    _notification: *mut AnyObject,
) {
    ffi_firewall("resume_display_link", || unsafe {
        set_display_link_paused::<P>(self_, false);
    });
}

unsafe extern "C" fn tick_thunk<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) {
    ffi_firewall("tick", || unsafe {
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };
        run_frame(inner);
    });
}

fn run_frame<P: Params + 'static>(inner: &mut Inner<P>) {
    // UIKit updates the proxy's selectedTextRange directly while the user
    // drags the keyboard trackpad. Mirror that range into egui before this
    // frame consumes input events.
    unsafe { sync_native_proxy_to_egui(inner) };
    // Publish the true host-view logical size so a plugin's `ui()` can
    // read it back via `truce_egui::actual_window_size` on iOS the same
    // way the desktop `editor.rs::run_frame` does each frame.
    crate::set_actual_window_size(&inner.egui_ctx, (inner.logical_w, inner.logical_h));
    // `as f32` from u32: editor logical dimensions stay well below
    // 2^23, so the f32 mantissa loss never matters.
    #[allow(clippy::cast_precision_loss)]
    let screen_rect = egui::Rect::from_min_size(
        egui::pos2(0.0, 0.0),
        egui::vec2(inner.logical_w as f32, inner.logical_h as f32),
    );
    let mut raw_input = egui::RawInput {
        screen_rect: Some(screen_rect),
        time: Some(timestamp_seconds()),
        ..Default::default()
    };
    raw_input.events = std::mem::take(&mut inner.pending_events);
    // Pin the root viewport's pixels_per_point on every frame.
    // `RawInput::default()` seeds the viewport map with
    // `native_pixels_per_point: None`, which egui then resolves to
    // 1.0 - overriding `Context::set_pixels_per_point` we ran at
    // open. Without this, every frame's tessellation snaps back to
    // 1× DPI and Core Animation upscales the result, producing the
    // grainy edges we saw.
    raw_input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .native_pixels_per_point = Some(inner.scale);

    let output = inner.egui_ctx.run_ui(raw_input, |root_ui| {
        // Recover a poisoned `ui` mutex (into_inner) rather than skip the
        // frame forever: a panic in author `ui` code is caught by the thunk
        // firewall, and the editor must keep rendering afterward - matching
        // the built-in editor's non-poisoning RefCell.
        inner
            .ui
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ui(root_ui, &inner.context);
    });
    unsafe { sync_native_text_input_from_egui(inner, &output.platform_output) };
    if inner.native_selection_dirty
        && crate::apply_native_selection_to_focused_text_edit(
            &inner.egui_ctx,
            inner.native_selection,
        )
    {
        inner.native_selection_dirty = false;
    }
    crate::handle_platform_output(&output.platform_output);
    let clipped = inner
        .egui_ctx
        .tessellate(output.shapes, output.pixels_per_point);
    inner
        .renderer
        .render(&output.textures_delta, &clipped, inner.scale);
    // Drive the iOS soft keyboard from egui's focus state. egui
    // sets `egui_wants_keyboard_input()` while a `TextEdit` (or
    // other focus-grabbing widget) is the active receiver; we
    // mirror that into `becomeFirstResponder` /
    // `resignFirstResponder` on the host UIView. UIKit only
    // presents the keyboard for the current first responder *and*
    // only if that responder conforms to `UIKeyInput`; the class
    // declares both `UIKeyInput` + `UITextInputTraits` at class-
    // build time (via `add_protocol` in `install_editor_view`) so
    // UIKit accepts our `becomeFirstResponder`. Tapping a non-text
    // egui widget makes the flag go false again on the next frame;
    // we resign and the keyboard dismisses.
    let wants_kb = inner.egui_ctx.egui_wants_keyboard_input();
    let responder = if inner.text_input.is_null() {
        inner.child_view
    } else {
        inner.text_input
    };
    if !responder.is_null() {
        unsafe {
            let is_first: Bool = msg_send![responder, isFirstResponder];
            match crate::native_keyboard_action(
                wants_kb,
                inner.keyboard_session_active,
                is_first.as_bool(),
                inner.keyboard_dismiss_requested,
            ) {
                crate::NativeKeyboardAction::BecomeFirstResponder => {
                    let became: Bool = msg_send![responder, becomeFirstResponder];
                    inner.keyboard_session_active = became.as_bool();
                }
                crate::NativeKeyboardAction::ResignFirstResponder => {
                    let _: Bool = msg_send![responder, resignFirstResponder];
                    inner.keyboard_session_active = false;
                }
                crate::NativeKeyboardAction::SurrenderEguiFocus => {
                    if let Some(id) = inner.egui_ctx.memory(|memory| memory.focused()) {
                        inner
                            .egui_ctx
                            .memory_mut(|memory| memory.surrender_focus(id));
                    }
                    inner.native_selection_dirty = false;
                    inner.keyboard_session_active = false;
                }
                crate::NativeKeyboardAction::DismissAndSurrender => {
                    let _: Bool = msg_send![responder, resignFirstResponder];
                    let _: Bool = msg_send![inner.child_view, endEditing: Bool::YES];
                    if let Some(id) = inner.egui_ctx.memory(|memory| memory.focused()) {
                        inner
                            .egui_ctx
                            .memory_mut(|memory| memory.surrender_focus(id));
                    }
                    inner.native_selection_dirty = false;
                    inner.keyboard_session_active = false;
                    inner.keyboard_dismiss_requested = false;
                }
                crate::NativeKeyboardAction::None => {
                    if is_first.as_bool() {
                        inner.keyboard_session_active = true;
                    } else if !wants_kb {
                        inner.keyboard_session_active = false;
                        inner.native_selection_dirty = false;
                    }
                }
            }
        }
    }
    let _ = inner.params;
}

unsafe fn native_proxy_text(input: *mut AnyObject) -> Option<String> {
    unsafe {
        if input.is_null() {
            return None;
        }
        let text: *mut AnyObject = msg_send![input, text];
        if text.is_null() {
            return Some(String::new());
        }
        let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
        if utf8.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(utf8)
            .to_str()
            .ok()
            .map(ToOwned::to_owned)
    }
}

unsafe fn native_proxy_selection(
    input: *mut AnyObject,
    text: &str,
) -> Option<crate::NativeTextSelection> {
    unsafe {
        if input.is_null() {
            return None;
        }
        let range: *mut AnyObject = msg_send![input, selectedTextRange];
        let beginning: *mut AnyObject = msg_send![input, beginningOfDocument];
        if range.is_null() || beginning.is_null() {
            return None;
        }
        let start: *mut AnyObject = msg_send![range, start];
        let end: *mut AnyObject = msg_send![range, end];
        if start.is_null() || end.is_null() {
            return None;
        }
        let start_offset: isize =
            msg_send![input, offsetFromPosition: beginning, toPosition: start];
        let end_offset: isize = msg_send![input, offsetFromPosition: beginning, toPosition: end];
        if start_offset < 0 || end_offset < 0 {
            return None;
        }
        Some(
            crate::NativeTextSelection {
                start: crate::char_index_for_utf16_offset(text, start_offset as usize),
                end: crate::char_index_for_utf16_offset(text, end_offset as usize),
            }
            .normalized(),
        )
    }
}

unsafe fn set_native_proxy_text(input: *mut AnyObject, text: &str) {
    unsafe {
        if input.is_null() {
            return;
        }
        let text = NSString::from_str(text);
        let _: () = msg_send![input, setText: &*text];
    }
}

unsafe fn set_native_proxy_selection(
    input: *mut AnyObject,
    text: &str,
    selection: crate::NativeTextSelection,
) {
    unsafe {
        if input.is_null() {
            return;
        }
        let selection = selection.normalized();
        let beginning: *mut AnyObject = msg_send![input, beginningOfDocument];
        if beginning.is_null() {
            return;
        }
        let start_offset = crate::utf16_offset_for_char_index(text, selection.start);
        let end_offset = crate::utf16_offset_for_char_index(text, selection.end);
        let Ok(start_offset) = isize::try_from(start_offset) else {
            return;
        };
        let Ok(end_offset) = isize::try_from(end_offset) else {
            return;
        };
        let start: *mut AnyObject =
            msg_send![input, positionFromPosition: beginning, offset: start_offset];
        let end: *mut AnyObject =
            msg_send![input, positionFromPosition: beginning, offset: end_offset];
        if start.is_null() || end.is_null() {
            return;
        }
        let range: *mut AnyObject = msg_send![input, textRangeFromPosition: start, toPosition: end];
        if !range.is_null() {
            let _: () = msg_send![input, setSelectedTextRange: range];
        }
    }
}

unsafe fn queue_native_proxy_change<P: Params + 'static>(
    inner: &mut Inner<P>,
    input: *mut AnyObject,
) {
    unsafe {
        let Some(text) = native_proxy_text(input) else {
            return;
        };
        let selection = native_proxy_selection(input, &text).unwrap_or_else(|| {
            let end = text.chars().count();
            crate::NativeTextSelection { start: end, end }
        });

        if text != inner.native_text {
            inner
                .pending_events
                .extend(crate::native_text_replacement_events(&text));
            inner.native_text = text;
            inner.native_selection_dirty = true;
        }
        if selection != inner.native_selection {
            inner.native_selection = selection;
            inner.native_selection_dirty = true;
        }
    }
}

unsafe fn sync_native_proxy_to_egui<P: Params + 'static>(inner: &mut Inner<P>) {
    unsafe { queue_native_proxy_change(inner, inner.text_input) }
}

unsafe fn sync_native_text_input_from_egui<P: Params + 'static>(
    inner: &mut Inner<P>,
    output: &egui::PlatformOutput,
) {
    unsafe {
        if inner.text_input.is_null() || output.ime.is_none() {
            return;
        }

        let mut latest_text = None;
        let mut latest_selection = None;
        let mut gained_focus = false;
        for event in &output.events {
            let info = event.widget_info();
            let Some(text) = info.current_text_value.as_deref() else {
                continue;
            };
            latest_text = Some(text);
            if let Some(selection) = info.text_selection.as_ref() {
                latest_selection = Some(
                    crate::NativeTextSelection {
                        start: *selection.start(),
                        end: *selection.end(),
                    }
                    .normalized(),
                );
            }
            gained_focus |= matches!(event, egui::output::OutputEvent::FocusGained(_));
        }

        let Some(text) = latest_text else { return };
        if text != inner.native_text {
            set_native_proxy_text(inner.text_input, text);
            inner.native_text.clear();
            inner.native_text.push_str(text);
        }

        let char_count = text.chars().count();
        let selection = latest_selection.unwrap_or_else(|| {
            if gained_focus {
                crate::NativeTextSelection {
                    start: char_count,
                    end: char_count,
                }
            } else {
                crate::NativeTextSelection {
                    start: inner.native_selection.start.min(char_count),
                    end: inner.native_selection.end.min(char_count),
                }
            }
        });
        if latest_selection.is_some() || gained_focus {
            set_native_proxy_selection(inner.text_input, text, selection);
        }
        inner.native_selection = selection;
    }
}

/// Re-size the live editor surface to `logical_w` x `logical_h` logical
/// points. Updates the cached logical size, the `UIView` frame, the
/// `CAMetalLayer` drawable (in physical pixels), and the wgpu surface.
/// The next `run_frame` reflows egui because its `screen_rect` reads
/// `inner.logical_w/h`. No-op when the size is unchanged so redundant
/// layout passes are cheap.
fn resize_inner<P: Params + ?Sized>(inner: &mut Inner<P>, logical_w: u32, logical_h: u32) {
    if logical_w == 0 || logical_h == 0 {
        return;
    }
    if inner.logical_w == logical_w && inner.logical_h == logical_h {
        return;
    }

    inner.logical_w = logical_w;
    inner.logical_h = logical_h;

    let frame = NSRect {
        origin: NSPoint { x: 0.0, y: 0.0 },
        size: NSSize {
            width: f64::from(logical_w),
            height: f64::from(logical_h),
        },
    };

    // Physical-pixel math bounded by editor size x backing scale; the
    // cast loss is irrelevant (`scale` <= 2.0, dims < 2^23).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let phys_w = (f64::from(logical_w) * f64::from(inner.scale)).round() as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let phys_h = (f64::from(logical_h) * f64::from(inner.scale)).round() as u32;

    // SAFETY: `child_view` is the pinned UIView (its `layer` is the
    // CAMetalLayer wgpu draws into); both outlive `inner`. Frame /
    // drawable updates are main-thread UIKit calls, which is where
    // `set_size` runs.
    unsafe {
        let _: () = msg_send![inner.child_view, setFrame: frame];
        let layer: *mut AnyObject = msg_send![inner.child_view, layer];
        let drawable_size = NSSize {
            width: f64::from(phys_w),
            height: f64::from(phys_h),
        };
        let _: () = msg_send![layer, setDrawableSize: drawable_size];
    }

    inner.renderer.resize(phys_w, phys_h);
}

fn timestamp_seconds() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

unsafe extern "C" fn touches_began<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    touches: *mut AnyObject,
    _event: *mut AnyObject,
) {
    ffi_firewall("touches_began", || unsafe {
        dispatch_touch::<P>(self_, touches, TouchPhase::Began);
    });
}

unsafe extern "C" fn touches_moved<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    touches: *mut AnyObject,
    _event: *mut AnyObject,
) {
    ffi_firewall("touches_moved", || unsafe {
        dispatch_touch::<P>(self_, touches, TouchPhase::Moved);
    });
}

unsafe extern "C" fn touches_ended<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    touches: *mut AnyObject,
    _event: *mut AnyObject,
) {
    ffi_firewall("touches_ended", || unsafe {
        dispatch_touch::<P>(self_, touches, TouchPhase::Ended);
    });
}

unsafe extern "C" fn touches_cancelled<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    touches: *mut AnyObject,
    _event: *mut AnyObject,
) {
    ffi_firewall("touches_cancelled", || unsafe {
        dispatch_touch::<P>(self_, touches, TouchPhase::Ended);
    });
}

// UIKeyInput conformance - drives the iOS soft keyboard for egui text widgets

unsafe extern "C" fn can_become_first_responder(_self: &AnyObject, _cmd: Sel) -> Bool {
    Bool::YES
}

/// `UIKeyInput.hasText` - `UIKit` reads this to decide whether
/// to allow `deleteBackward` to fire. Always returning true
/// matches what egui-internal text widgets do (their delete
/// handler is a no-op when the field is empty, so over-firing
/// is harmless and the `UIKit` predictive-text bar lights up
/// correctly).
unsafe extern "C" fn has_text(_self: &AnyObject, _cmd: Sel) -> Bool {
    Bool::YES
}

/// `UIKeyInput.insertText:` - `UIKit` hands us the user's typed
/// characters as an `NSString*` (one keystroke per call for
/// regular keys, longer strings for IME composition commits).
/// We forward to egui as a `Text` event; egui's `TextEdit`
/// widget appends it at the cursor.
unsafe extern "C" fn insert_text<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    text: *mut AnyObject,
) {
    ffi_firewall("insert_text", || unsafe {
        if text.is_null() {
            return;
        }
        let utf8: *const std::os::raw::c_char = msg_send![text, UTF8String];
        if utf8.is_null() {
            return;
        }
        let Ok(s) = std::ffi::CStr::from_ptr(utf8).to_str() else {
            return;
        };
        if s.is_empty() {
            return;
        }
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };
        inner
            .pending_events
            .extend(crate::native_text_input_events(s));
    });
}

/// `UIKeyInput.deleteBackward` - Backspace. egui maps this to a
/// pressed+released `Key::Backspace` event; the `TextEdit`
/// widget removes the character before the cursor (or the
/// selection).
unsafe extern "C" fn delete_backward<P: Params + 'static>(self_: &AnyObject, _cmd: Sel) {
    ffi_firewall("delete_backward", || unsafe {
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };
        let modifiers = egui::Modifiers::default();
        inner.pending_events.push(egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        inner.pending_events.push(egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        });
    });
}

/// `UIControlEventEditingChanged` target for the native text proxy. UIKit
/// mutates `UITextField`'s private storage without reliably dispatching its
/// public `UIKeyInput` overrides, so mirror the complete native value here.
unsafe extern "C" fn text_field_changed<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    sender: *mut AnyObject,
) {
    ffi_firewall("text_field_changed", || unsafe {
        if sender.is_null() {
            return;
        }
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };
        queue_native_proxy_change(inner, sender);
    });
}

/// `UIControlEventEditingDidEndOnExit` target. `UITextField` consumes Return
/// as a control action instead of inserting a newline into its value.
unsafe extern "C" fn text_field_return<P: Params + 'static>(
    self_: &AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) {
    ffi_firewall("text_field_return", || unsafe {
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };
        inner
            .pending_events
            .extend(crate::native_text_input_events("\n"));
        inner.keyboard_dismiss_requested = true;
    });
}

unsafe fn dispatch_touch<P: Params + 'static>(
    self_: &AnyObject,
    touches: *mut AnyObject,
    phase: TouchPhase,
) {
    unsafe {
        let Some(arc) = borrow_inner_arc::<P>(self_) else {
            return;
        };
        let mut guard = arc
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(inner) = guard.as_mut() else { return };

        // Pick one touch - egui is single-pointer. Real multi-touch
        // would need an `egui::Event` per finger which egui doesn't
        // model natively; "first finger wins" is the standard
        // single-pointer reduction.
        let touch: *mut AnyObject = msg_send![touches, anyObject];
        if touch.is_null() {
            return;
        }
        let view_ptr: *mut AnyObject = std::ptr::from_ref::<AnyObject>(self_).cast_mut();
        let pt: NSPoint = msg_send![touch, locationInView: view_ptr];
        #[allow(clippy::cast_possible_truncation)]
        let pos = egui::pos2(pt.x as f32, pt.y as f32);
        inner.last_pointer = pos;
        inner.pending_events.push(egui::Event::PointerMoved(pos));
        match phase {
            TouchPhase::Began => {
                inner.pending_events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                });
            }
            TouchPhase::Ended => {
                inner.pending_events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                });
                inner.pending_events.push(egui::Event::PointerGone);
            }
            TouchPhase::Moved => {}
        }
    }
}
