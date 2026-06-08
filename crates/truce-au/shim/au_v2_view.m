/**
 * AU v2 Cocoa UI view factory.
 *
 * Defines the `AUCocoaUIBase` class the host instantiates after
 * reading our `kAudioUnitProperty_CocoaUI`. The class is compiled
 * into every truce plugin dylib so it appears in `__objc_classlist`,
 * which `[NSBundle classNamed:]`-based hosts (REAPER) require.
 *
 * The class name MUST be unique per plugin. AppKit/AudioUnit hosts
 * load every installed `.component` into one process; if two plugins
 * publish a class with the same name, `libobjc` keeps the first one
 * and `[NSBundle classNamed:name]` on the loser's bundle returns nil
 * - the host then thinks the plugin has no GUI. Uniqueness comes
 * from the `TRUCE_AU_PLUGIN_ID` env var that `cargo-truce` sets at
 * build time; the build.rs sanitises and passes it as a `-D` define.
 * Without that env (plain `cargo build` for unit tests), the class
 * falls back to a default name - fine for isolated tests, not for
 * multi-plugin hosting.
 */

@import AppKit;
@import AudioToolbox;
@import QuartzCore;
#import <AudioUnit/AUCocoaUIView.h>

#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "au_shim_types.h"

// Private properties exposed by `au_v2_shim.c`:
//   64000: AuPlugin context pointer (rustCtx)
//   64001: pointer to the AU's AuCallbacks table (g_callbacks of the
//          dylib that owns this AudioUnit). Reading both via the AU
//          dispatch table keeps the methods plugin-agnostic - per-
//          dylib globals reached are always the right ones.
#define kTrucePrivateProperty_RustContext  64000
#define kTrucePrivateProperty_AuCallbacks  64001

#ifndef TRUCE_AU_VIEW_FACTORY_NAME
// Default name when `TRUCE_AU_PLUGIN_ID` is unset - keeps `cargo build`
// of the workspace cdylibs working for unit tests.
#define TRUCE_AU_VIEW_FACTORY_NAME TruceAUCocoaViewProxy
#endif

#ifndef TRUCE_AU_VIEW_CLASS_NAME
#define TRUCE_AU_VIEW_CLASS_NAME TruceAUCocoaEditorView
#endif

int32_t truce_au_v2_is_context_alive(void *ctx);
static BOOL truce_au_responsive_resize_enabled(void);
static void truce_prepare_container_view(NSView *view);
static void truce_prepare_resizable_view(NSView *view);
static void truce_prepare_editor_subviews(NSView *view);

static BOOL truce_resize_debug_enabled(void) {
    static int enabled = -1;
    if (enabled < 0) {
        const char *value = getenv("TRUCE_AU_RESIZE_DEBUG");
        enabled = (value && value[0] && strcmp(value, "0") != 0) ? 1 : 0;
    }
    return enabled != 0;
}

static BOOL truce_au_responsive_resize_enabled(void) {
    static int enabled = -1;
    if (enabled < 0) {
        const char *value = getenv("TRUCE_AU_RESIZE_MODE");
        enabled = (value &&
                   (strcmp(value, "responsive") == 0 ||
                    strcmp(value, "freeform") == 0 ||
                    strcmp(value, "1") == 0 ||
                    strcmp(value, "true") == 0)) ? 1 : 0;
    }
    return enabled != 0;
}

static NSString *truce_view_summary(NSView *view) {
    if (!view) {
        return @"(nil)";
    }

    NSRect frame = [view frame];
    NSRect bounds = [view bounds];
    NSRect windowFrame = [[view window] frame];
    return [NSString stringWithFormat:@"%@ %p frame=(%.1f %.1f %.1f %.1f) bounds=(%.1f %.1f %.1f %.1f) mask=%lu subviews=%lu window=(%.1f %.1f %.1f %.1f)",
        NSStringFromClass([view class]), view,
        frame.origin.x, frame.origin.y, frame.size.width, frame.size.height,
        bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height,
        (unsigned long)[view autoresizingMask],
        (unsigned long)[[view subviews] count],
        windowFrame.origin.x, windowFrame.origin.y, windowFrame.size.width, windowFrame.size.height];
}

static void truce_log_view_chain(NSString *label, NSView *view) {
    if (!truce_resize_debug_enabled()) {
        return;
    }

    NSLog(@"[TRUCE-AU-RESIZE] %@ chain start", label);
    NSView *cur = view;
    for (NSUInteger depth = 0; cur && depth < 8; ++depth, cur = [cur superview]) {
        NSLog(@"[TRUCE-AU-RESIZE] %@ depth=%lu %@", label, (unsigned long)depth, truce_view_summary(cur));
    }
    NSLog(@"[TRUCE-AU-RESIZE] %@ chain end", label);
}

@interface TRUCE_AU_VIEW_CLASS_NAME : NSView {
    void *_truceCtx;
    const AuCallbacks *_truceCallbacks;
    BOOL _truceEditorOpen;
    uint32_t _truceLastWidth;
    uint32_t _truceLastHeight;
    NSView *_truceObservedSuperview;
}
- (void)truceAttachContext:(void *)ctx callbacks:(const AuCallbacks *)callbacks;
- (void)truceNotifySize:(NSSize)size;
- (void)truceCloseEditor;
@end

@implementation TRUCE_AU_VIEW_CLASS_NAME

- (void)truceAttachContext:(void *)ctx callbacks:(const AuCallbacks *)callbacks {
    _truceCtx = ctx;
    _truceCallbacks = callbacks;
    _truceEditorOpen = YES;
    _truceLastWidth = 0;
    _truceLastHeight = 0;
    truce_prepare_container_view(self);
    truce_log_view_chain(@"attach", self);
    [self truceNotifySize:[self frame].size];
}

- (void)truceNotifySize:(NSSize)size {
    if (!_truceEditorOpen || !_truceCtx || !_truceCallbacks ||
        !truce_au_v2_is_context_alive(_truceCtx)) {
        return;
    }

    if (size.width < 1.0 || size.height < 1.0) {
        return;
    }

    uint32_t width = (uint32_t)(size.width + 0.5);
    uint32_t height = (uint32_t)(size.height + 0.5);
    if (width == _truceLastWidth && height == _truceLastHeight) {
        return;
    }

    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] notify-size proposed=%ux%u last=%ux%u %@", width, height, _truceLastWidth, _truceLastHeight, truce_view_summary(self));
    }

    int32_t accepted = _truceCallbacks->gui_set_size(_truceCtx, width, height);
    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] notify-size result=%d proposed=%ux%u", accepted, width, height);
    }
    if (accepted) {
        _truceLastWidth = width;
        _truceLastHeight = height;
    }
}

- (void)setFrameSize:(NSSize)newSize {
    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] container setFrameSize %.1fx%.1f before %@", newSize.width, newSize.height, truce_view_summary(self));
    }
    [super setFrameSize:newSize];
    truce_log_view_chain(@"container setFrameSize after", self);
    [self truceNotifySize:newSize];
}

- (void)setFrame:(NSRect)frameRect {
    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] container setFrame %.1f %.1f %.1f %.1f before %@", frameRect.origin.x, frameRect.origin.y, frameRect.size.width, frameRect.size.height, truce_view_summary(self));
    }
    [super setFrame:frameRect];
    truce_log_view_chain(@"container setFrame after", self);
    [self truceNotifySize:frameRect.size];
}

- (void)viewDidMoveToSuperview {
    [super viewDidMoveToSuperview];

    if (_truceObservedSuperview) {
        [[NSNotificationCenter defaultCenter] removeObserver:self
                                                        name:NSViewFrameDidChangeNotification
                                                      object:_truceObservedSuperview];
        _truceObservedSuperview = nil;
    }

    NSView *superview = [self superview];
    if (superview) {
        _truceObservedSuperview = superview;
        [superview setPostsFrameChangedNotifications:YES];
        [[NSNotificationCenter defaultCenter] addObserver:self
                                                 selector:@selector(truceSuperviewFrameChanged:)
                                                     name:NSViewFrameDidChangeNotification
                                                   object:superview];
        truce_prepare_container_view(self);
        truce_prepare_editor_subviews(self);
    }

    truce_log_view_chain(@"viewDidMoveToSuperview", self);
}

- (void)truceSuperviewFrameChanged:(NSNotification *)notification {
    NSView *superview = (NSView *)[notification object];
    NSRect superBounds = [superview bounds];
    NSRect frame = [self frame];

    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] observed-superview-frame-change %@", truce_view_summary(superview));
        truce_log_view_chain(@"observed-superview-frame-change before-sync", self);
    }

    if (superBounds.size.width >= 1.0 && superBounds.size.height >= 1.0 &&
        truce_au_responsive_resize_enabled() &&
        (fabs(frame.size.width - superBounds.size.width) > 0.5 ||
         fabs(frame.size.height - superBounds.size.height) > 0.5)) {
        [self setFrame:NSMakeRect(0, 0, superBounds.size.width, superBounds.size.height)];
        truce_prepare_editor_subviews(self);
    }

    truce_log_view_chain(@"observed-superview-frame-change after-sync", self);
}

- (void)truceCloseEditor {
    void *ctx = _truceCtx;
    const AuCallbacks *callbacks = _truceCallbacks;
    BOOL shouldClose = _truceEditorOpen && ctx && callbacks && truce_au_v2_is_context_alive(ctx);

    if (_truceObservedSuperview) {
        [[NSNotificationCenter defaultCenter] removeObserver:self
                                                        name:NSViewFrameDidChangeNotification
                                                      object:_truceObservedSuperview];
        _truceObservedSuperview = nil;
    }

    _truceEditorOpen = NO;
    _truceCtx = NULL;
    _truceCallbacks = NULL;
    _truceLastWidth = 0;
    _truceLastHeight = 0;

    if (shouldClose) {
        callbacks->gui_close(ctx);
    }
}

- (void)dealloc {
    [self truceCloseEditor];
}

@end

static void truce_prepare_resizable_view(NSView *view) {
    if (!view) {
        return;
    }

    [view setPostsFrameChangedNotifications:YES];
    [view setAutoresizingMask:(NSViewWidthSizable | NSViewHeightSizable)];
    [view invalidateIntrinsicContentSize];
    [view setNeedsLayout:YES];
    [view setNeedsDisplay:YES];
}

static void truce_prepare_container_view(NSView *view) {
    if (!view) {
        return;
    }

    if (truce_au_responsive_resize_enabled()) {
        truce_prepare_resizable_view(view);
    } else {
        [view setPostsFrameChangedNotifications:YES];
        [view setAutoresizingMask:0];
        [view invalidateIntrinsicContentSize];
        [view setNeedsLayout:YES];
        [view setNeedsDisplay:YES];
    }
}

static void truce_prepare_editor_subviews(NSView *view) {
    if (!view) {
        return;
    }

    NSRect bounds = [view bounds];
    for (NSView *child in [view subviews]) {
        [child setFrame:bounds];
        truce_prepare_resizable_view(child);
    }
}

static void truce_resize_editor_view(NSView *view, NSSize size) {
    if (!view) {
        return;
    }

    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] resize_editor_view begin target=%.1fx%.1f", size.width, size.height);
        truce_log_view_chain(@"resize_editor_view before", view);
    }

    NSView *superview = [view superview];
    if (superview) {
        NSRect superFrame = [superview frame];
        superFrame.size = size;
        if (truce_resize_debug_enabled()) {
            NSLog(@"[TRUCE-AU-RESIZE] resize_editor_view set superview frame %.1f %.1f %.1f %.1f before %@", superFrame.origin.x, superFrame.origin.y, superFrame.size.width, superFrame.size.height, truce_view_summary(superview));
        }
        [superview setFrame:superFrame];
        if (truce_resize_debug_enabled()) {
            NSLog(@"[TRUCE-AU-RESIZE] resize_editor_view superview after %@", truce_view_summary(superview));
        }
    }

    if (truce_resize_debug_enabled()) {
        NSLog(@"[TRUCE-AU-RESIZE] resize_editor_view set container frame target=%.1fx%.1f before %@", size.width, size.height, truce_view_summary(view));
    }
    [view setFrame:NSMakeRect([view frame].origin.x, [view frame].origin.y, size.width, size.height)];
    truce_prepare_container_view(view);

    // Match JUCE's AU holder model: keep the host container and the
    // hosted editor child in sync, but do not rewrite arbitrary
    // descendants. Deeper views/layers own their own coordinate systems.
    truce_prepare_editor_subviews(view);

    truce_log_view_chain(@"resize_editor_view after", view);
}

@interface TRUCE_AU_VIEW_FACTORY_NAME : NSObject <AUCocoaUIBase>
@end

@implementation TRUCE_AU_VIEW_FACTORY_NAME

- (unsigned)interfaceVersion {
    return 0;
}

- (NSView *)uiViewForAudioUnit:(AudioUnit)au withSize:(NSSize)preferredSize {
    void *ctx = NULL;
    UInt32 ctxSize = sizeof(ctx);
    if (AudioUnitGetProperty(au, kTrucePrivateProperty_RustContext,
            kAudioUnitScope_Global, 0, &ctx, &ctxSize) != noErr || !ctx) {
        return nil;
    }

    const AuCallbacks *cb = NULL;
    UInt32 cbSize = sizeof(cb);
    if (AudioUnitGetProperty(au, kTrucePrivateProperty_AuCallbacks,
            kAudioUnitScope_Global, 0, &cb, &cbSize) != noErr || !cb) {
        return nil;
    }

    if (!cb->gui_has_editor(ctx)) return nil;

    uint32_t w = 0, h = 0;
    cb->gui_get_size(ctx, &w, &h);
    if (w == 0 || h == 0) return nil;

    NSRect frame = NSMakeRect(0, 0, w, h);
    TRUCE_AU_VIEW_CLASS_NAME *container = [[TRUCE_AU_VIEW_CLASS_NAME alloc] initWithFrame:frame];
    [container truceAttachContext:ctx callbacks:cb];
    [container setPostsFrameChangedNotifications:YES];
    cb->gui_open(ctx, (__bridge void *)container);
    truce_prepare_editor_subviews(container);
    return container;
}

@end

int32_t truce_au_v2_resize_editor_view(void *viewPtr, uint32_t width, uint32_t height) {
    if (!viewPtr || width == 0 || height == 0) {
        return 0;
    }

    NSView *view = (__bridge NSView *)viewPtr;
    NSSize size = NSMakeSize((CGFloat)width, (CGFloat)height);
    int32_t ok = 1;

    [CATransaction begin];
    [CATransaction setDisableActions:YES];

    @try {
        if (truce_resize_debug_enabled()) {
            NSLog(@"[TRUCE-AU-RESIZE] resize_editor_view ffi request=%ux%u", width, height);
        }
        truce_resize_editor_view(view, size);
        [view layoutSubtreeIfNeeded];
        truce_log_view_chain(@"resize_editor_view ffi after layout", view);
    } @catch (NSException *exception) {
        ok = 0;
        NSLog(@"Truce AU resize failed: %@ %@", [exception name], [exception reason]);
    }

    [CATransaction commit];

    return ok;
}

// Stringify the class name for the v2 shim's `kAudioUnitProperty_CocoaUI`
// response. Two-step macro so the argument is expanded before stringification.
#define _TRUCE_STRINGIFY(x) #x
#define TRUCE_STRINGIFY(x) _TRUCE_STRINGIFY(x)

const char *truce_au_view_factory_class_name(void) {
    return TRUCE_STRINGIFY(TRUCE_AU_VIEW_FACTORY_NAME);
}
