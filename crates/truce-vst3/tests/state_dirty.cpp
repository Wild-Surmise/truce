// Build with c++ -std=c++20 -pthread tests/state_dirty.cpp -o /tmp/vst3-dirty.
// On macOS also pass -framework CoreFoundation, then run /tmp/vst3-dirty.
#include "../shim/vst3_shim.cpp"
#include <cassert>
#include <memory>
#include <thread>
#if __APPLE__
#include <CoreFoundation/CoreFoundation.h>
#endif

struct MockPlugin {
    std::atomic<bool> dirty{false};
    unsigned opens = 0;
    unsigned closes = 0;
};

static MockPlugin* lastCreated;
static const auto hostThread = std::this_thread::get_id();

struct MockHost {
    void** table;
    unsigned references = 1;
    unsigned dirtyCalls = 0;
    bool supportsDirty = true;
    std::unique_ptr<TruceComponent>* closeOnDirty = nullptr;
    unsigned restartCalls = 0;
};

static uint32 hostAddRef(void* self) { return ++static_cast<MockHost*>(self)->references; }
static uint32 hostRelease(void* self) { return --static_cast<MockHost*>(self)->references; }

static tresult hostQuery(void* self, const int8_t* iid, void** out) {
    auto* host = static_cast<MockHost*>(self);
    assert(std::this_thread::get_id() == hostThread);
    const TUID expected = MAKE_IID(0xF040B4B3, 0xA36045EC, 0xABCDC045, 0xB4D5A2CC);
    assert(std::memcmp(iid, expected, sizeof(expected)) == 0);
    if (!host->supportsDirty) return kNoInterface;
    hostAddRef(host);
    *out = host;
    return kResultOk;
}

static tresult hostDirty(void* self, TBool dirty) {
    auto* host = static_cast<MockHost*>(self);
    assert(std::this_thread::get_id() == hostThread && dirty == 1);
    ++host->dirtyCalls;
    if (host->closeOnDirty) host->closeOnDirty->reset();
    return kResultOk;
}

static tresult hostRestart(void* self, int32 flags) {
    auto* host = static_cast<MockHost*>(self);
    assert(flags == 8);
    assert(host->references >= 2); // Host plus the notification's retained reference.
    ++host->restartCalls;
    return kResultOk;
}

int main() {
    Vst3Callbacks callbacks{};
    callbacks.create = []() -> void* { return lastCreated = new MockPlugin; };
    callbacks.destroy = [](void* ctx) { delete static_cast<MockPlugin*>(ctx); };
    callbacks.param_get_value = [](void*, uint32_t) { return 0.0; };
    callbacks.param_normalize = [](void*, uint32_t, double value) { return value; };
    callbacks.gui_open = [](void* ctx, void* parent, void* component) {
        assert(parent && component);
        ++static_cast<MockPlugin*>(ctx)->opens;
    };
    callbacks.gui_close = [](void* ctx) {
        ++static_cast<MockPlugin*>(ctx)->closes;
    };
    callbacks.take_state_dirty = [](void* ctx) -> int32_t {
        return static_cast<MockPlugin*>(ctx)->dirty.exchange(false);
    };
    g_cb = &callbacks;

    // Fill the shim's legacy registry. The dirty notification for the next
    // component must still work with its own Rust instance and native view.
    std::vector<std::unique_ptr<TruceComponent>> instances;
    for (int i = 0; i < kMaxInstances; ++i) {
        instances.push_back(std::make_unique<TruceComponent>());
    }
    auto component = std::make_unique<TruceComponent>();
    auto* plugin = lastCreated;
    assert(ctx_lookup(plugin) == nullptr);
    void* vtable[] = {
        (void*)hostQuery, (void*)hostAddRef, (void*)hostRelease, (void*)hostDirty,
        nullptr, nullptr, (void*)hostRestart,
    };
    MockHost host{vtable};
    assert(component->setComponentHandler(&host) == kResultOk);

    std::thread editor([&] {
        plugin->dirty.store(true);
        plugin->dirty.store(true);
    });
    editor.join();
    assert(host.dirtyCalls == 0);
    component->getParamNormalized(0);
    component->getParamNormalized(0);
    assert(host.dirtyCalls == 1 && host.references == 2);

    unsigned delivered = 1;
    host.supportsDirty = false;
    plugin->dirty.store(true);
    component->getParamNormalized(0);
    assert(host.dirtyCalls == delivered && host.references == 2);

    host.supportsDirty = true;
#if __APPLE__
    plugin->dirty.store(true);
    truce_vst3_wake_state_dirty(component.get());
    assert(host.dirtyCalls == delivered); // Marking during a draw never calls the host.
    for (int i = 0; i < 10 && host.dirtyCalls == delivered; ++i) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    }
    assert(host.dirtyCalls == ++delivered && host.references == 2);
#endif

    plugin->dirty.store(true);
    TrucePlugView view{};
    view.comp = component.get();
    view.ctx = plugin;
    component->stateLoaded = true;
    assert(pv_attached(&view, &view, nullptr) == kResultOk && plugin->opens == 1);
#if __APPLE__
    truce_vst3_wake_state_dirty(component.get());
#endif
    pv_removed(&view);
    assert(host.dirtyCalls == ++delivered && host.references == 2 && plugin->closes == 1);
    assert(pv_attached(&view, &view, nullptr) == kResultOk && plugin->opens == 2);
#if __APPLE__
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    assert(host.dirtyCalls == delivered);
#endif

    host.closeOnDirty = &component;
    component->pendingRestart.store(8);
    plugin->dirty.store(true);
#if __APPLE__
    truce_vst3_wake_state_dirty(component.get());
    assert(component && host.dirtyCalls == delivered);
    for (int i = 0; i < 10 && component; ++i) {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    }
#else
    component->getParamNormalized(0);
#endif
    assert(!component && host.dirtyCalls == ++delivered && host.restartCalls == 1 && host.references == 1);

#if __APPLE__
    host.closeOnDirty = nullptr;
    auto disposed = std::make_unique<TruceComponent>();
    assert(disposed->setComponentHandler(&host) == kResultOk);
    lastCreated->dirty.store(true);
    truce_vst3_wake_state_dirty(disposed.get());
    disposed.reset();
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, false);
    assert(host.dirtyCalls == delivered && host.references == 1);
#endif

    instances.clear();
    g_cb = nullptr;
    std::puts("VST3 dirty notifications passed: 65th instance, host-owned drain, close/reopen, queued teardown, re-entrancy.");
}
