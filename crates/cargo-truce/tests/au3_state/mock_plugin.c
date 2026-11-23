// Deterministic plugin boundary; the test uses the real Swift AUv3 template
// and Apple's real AUAudioUnit/parameter serializer, with no DSP or GUI.
#include "au_shim_types.h"
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static void *create(void) {
    double *value = malloc(sizeof(*value));
    if (value) *value = 0.25;
    return value;
}
static void destroy(void *ctx) { free(ctx); }
static double get(void *ctx, uint32_t id) { return *(double *)ctx; }
static void set(void *ctx, uint32_t id, double value) { *(double *)ctx = value; }
static void save(void *ctx, uint8_t **data, uint32_t *len) {
    *len = sizeof(double); *data = malloc(*len); memcpy(*data, ctx, *len);
}
static void load(void *ctx, const uint8_t *data, uint32_t len) {
    if (data && len == sizeof(double)) memcpy(ctx, data, len);
}
static void free_state(uint8_t *data, uint32_t len) { free(data); }
static uint32_t key_count(void *ctx) { return 1; }
static const char *key_at(void *ctx, uint32_t index) { return "legacy_state"; }
static int32_t load_foreign(void *ctx, const char *key, const uint8_t *data, uint32_t len) {
    if (len != sizeof(double)) return 0;
    load(ctx, data, len); return 1;
}
static int32_t state_dirty;
void mock_mark_state_dirty(void) { state_dirty = 1; }
static int32_t take_dirty(void *ctx) {
    int32_t dirty = state_dirty;
    state_dirty = 0;
    return dirty;
}
void *mock_au_v2_target;
void *mock_au_v2_ctx;
static void bind_v2_target(void *ctx, void *target) {
    if (target || ctx == mock_au_v2_ctx) {
        mock_au_v2_ctx = target ? ctx : NULL;
        mock_au_v2_target = target;
    }
}

static const AuCallbacks callbacks = {
    .abi_version = TRUCE_AU_ABI_VERSION, .create = create, .destroy = destroy,
    .param_get_value = get, .param_set_value = set,
    .state_save = save, .state_load = load, .state_free = free_state,
    .legacy_state_key_count = key_count, .legacy_state_key_at = key_at,
    .state_load_foreign = load_foreign,
    .gui_take_state_dirty = take_dirty,
    .gui_set_au_v2_state_target = bind_v2_target,
};
static const AuPluginDescriptor descriptor = {
    .num_inputs = 2, .num_outputs = 2, .bypass_param_id = UINT32_MAX,
};
static const AuParamDescriptor params[] = {
    {.id = 0, .name = "gain", .min = 0, .max = 1, .default_value = 0.25,
     .unit = "", .group = ""},
};
const AuPluginDescriptor *g_descriptor = &descriptor;
const AuCallbacks *g_callbacks = &callbacks;

// The v5 table ends at the inaccessible page, so even reading either
// appended v6 function pointer would fail this test.
void mock_use_previous_abi(void) {
    const size_t page = (size_t)sysconf(_SC_PAGESIZE);
    const size_t len = offsetof(AuCallbacks, gui_take_state_dirty);
    if (len > page) abort();
    unsigned char *region = mmap(NULL, page * 2, PROT_READ | PROT_WRITE,
                                 MAP_PRIVATE | MAP_ANON, -1, 0);
    if (region == MAP_FAILED || mprotect(region + page, page, PROT_NONE) != 0) abort();
    AuCallbacks *old = (AuCallbacks *)(region + page - len);
    memcpy(old, &callbacks, len);
    old->abi_version = TRUCE_AU_ABI_MAGIC | 5u;
    g_callbacks = old;
}
const AuParamDescriptor *g_param_descriptors = params;
uint32_t g_num_params = 1;
void truce_au_init(void) {}
