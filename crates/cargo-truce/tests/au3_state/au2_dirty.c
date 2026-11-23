// Exercise the AUv2 document-state signal through both Apple listener APIs.
#include <AudioToolbox/AudioToolbox.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <pthread.h>
#include <string.h>
#include "au_shim_types.h"

extern void *truce_au_v2_factory_bridge(const void *desc);
extern void truce_au_v2_host_state_dirty(void *target);
extern void mock_use_previous_abi(void);
extern void *mock_au_v2_target;
extern void *mock_au_v2_ctx;

const char *truce_au_view_factory_class_name(void) { return "NoView"; }

static unsigned property_count;
static unsigned event_count;
static bool dispose_in_listener;

static void property(void *data, AudioUnit unit, AudioUnitPropertyID id,
                     AudioUnitScope scope, AudioUnitElement element) {
    if (data) abort(); // The second listener must never see a disposed unit.
    if (id != kAudioUnitProperty_ClassInfo || scope != kAudioUnitScope_Global || element != 0)
        abort();
    CFPropertyListRef state = NULL;
    UInt32 size = sizeof(state);
    if (AudioUnitGetProperty(unit, id, scope, element, &state, &size) != noErr) abort();
    CFDataRef blob = CFDictionaryGetValue(state, CFSTR("truce_state"));
    double saved = 0.0;
    CFDataGetBytes(blob, CFRangeMake(0, sizeof(saved)), (UInt8 *)&saved);
    CFRelease(state);
    if (saved != 0.75) abort();
    if (!pthread_main_np()) abort();
    property_count++;
    if (dispose_in_listener && AudioComponentInstanceDispose(unit) != noErr) abort();
}

static void event(void *data, void *object, const AudioUnitEvent *ev, UInt64 when, Float32 value) {
    if (ev->mEventType != kAudioUnitEvent_PropertyChange
        || ev->mArgument.mProperty.mPropertyID != kAudioUnitProperty_ClassInfo)
        abort();
    event_count++;
}

static void *queue_edit(void *target) {
    truce_au_v2_host_state_dirty(target);
    truce_au_v2_host_state_dirty(target);
    return NULL;
}

int main(int argc, char **argv) {
    const bool previous_abi = argc > 1 && strcmp(argv[1], "previous-abi") == 0;
    const bool dispose_before_delivery = argc > 1 && strcmp(argv[1], "queued-close") == 0;
    dispose_in_listener = argc > 1 && strcmp(argv[1], "reentrant-close") == 0;
    if (previous_abi) mock_use_previous_abi();
    AudioComponentDescription desc = {'aufx', 'Drt1', 'Trce', kAudioComponentFlag_SandboxSafe, 0};
    AudioComponent component = AudioComponentRegister(
        &desc, CFSTR("Document state test"), 0x10000,
        (AudioComponentFactoryFunction)truce_au_v2_factory_bridge);
    // The existing parameter-gesture map contains only 64 entries.
    // The 65th AU must still get its own dirty-notification target.
    AudioUnit units[65] = {0};
    for (unsigned i = 0; i < 65; ++i) {
        if (AudioComponentInstanceNew(component, &units[i]) != noErr) return 1;
    }
    AudioUnit unit = units[64];
    if (previous_abi) {
        if (mock_au_v2_target) return 2;
        for (unsigned i = 0; i < 65; ++i) AudioComponentInstanceDispose(units[i]);
        puts("PASS dirty-v2-previous-abi");
        return 0;
    }
    UInt32 size = 0;
    if (AudioUnitGetPropertyInfo(unit, kAudioUnitProperty_ClassInfoFromDocument,
                                 kAudioUnitScope_Global, 0, &size, NULL) != kAudioUnitErr_InvalidProperty)
        return 2;
    if (AudioUnitAddPropertyListener(unit, kAudioUnitProperty_ClassInfo, property, NULL) != noErr)
        return 2;
    if (dispose_in_listener &&
        AudioUnitAddPropertyListener(unit, kAudioUnitProperty_ClassInfo, property, &event_count) != noErr)
        return 2;

    AUEventListenerRef listener;
    if (AUEventListenerCreate(event, NULL, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode,
                             .001, .001, &listener) != noErr)
        return 3;
    AudioUnitEvent expected = { .mEventType = kAudioUnitEvent_PropertyChange,
        .mArgument.mProperty = {unit, kAudioUnitProperty_ClassInfo, kAudioUnitScope_Global, 0} };
    if (!dispose_in_listener && !dispose_before_delivery &&
        AUEventListenerAddEventType(listener, NULL, &expected) != noErr) return 4;

    if (!mock_au_v2_target || !mock_au_v2_ctx) return 5;
    g_callbacks->param_set_value(mock_au_v2_ctx, 0, 0.75);
    pthread_t caller;
    if (pthread_create(&caller, NULL, queue_edit, mock_au_v2_target) != 0 ||
        pthread_join(caller, NULL) != 0 || property_count != 0)
        return 6;
    if (dispose_before_delivery) AudioComponentInstanceDispose(unit);
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.10, false);
    AUListenerDispose(listener);
    if (!dispose_in_listener && !dispose_before_delivery) AudioComponentInstanceDispose(unit);
    if (mock_au_v2_target || mock_au_v2_ctx) return 7;
    for (unsigned i = 0; i < 64; ++i) AudioComponentInstanceDispose(units[i]);
    if (property_count != !dispose_before_delivery ||
        event_count != (!dispose_in_listener && !dispose_before_delivery)) {
        fprintf(stderr, "ClassInfo: %u direct notifications, %u AUEventListener notifications\n",
                property_count, event_count);
        return 8;
    }
    printf("PASS dirty-v2%s%s\n", argc > 1 ? "-" : "", argc > 1 ? argv[1] : "");
    return 0;
}
