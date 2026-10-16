// Bridging header: exposes C types from the Rust AU framework to Swift.
#import "au_shim_types.h"
#import <AudioToolbox/AudioToolbox.h>
#import <objc/message.h>

// AUAudioUnit's parameter deserializer raises NSException for incompatible
// archives. Swift's error handling cannot catch it. Call the base implementation
// here (not unit.fullState, which would recurse into the Swift override) so no
// Swift frame lies between the throwing decoder and the exception handler.
// This is a host state-load operation, never part of the render callback.
static inline BOOL truce_au_restore_parameter_state(
    AUAudioUnit *unit, NSDictionary<NSString *, id> *state) {
    id data = state[@"data"];
    if (![data isKindOfClass:[NSData class]] || [data length] == 0) return NO;
    @try {
        struct objc_super base = { unit, [AUAudioUnit class] };
        ((void (*)(struct objc_super *, SEL, NSDictionary *))objc_msgSendSuper)(
            &base, @selector(setFullState:), state);
        return YES;
    } @catch (NSException *exception) {
        NSLog(@"Truce AU state restore: %@", exception.reason);
        return NO;
    }
}

// Swift can't use C macros, so provide inline helpers.
static inline uint32_t au_fourcc(const uint8_t b[4]) {
    return ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) |
           ((uint32_t)b[2] << 8)  | (uint32_t)b[3];
}
