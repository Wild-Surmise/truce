// Deterministic plugin boundary; the test uses the real Swift AUv3 template
// and Apple's real AUAudioUnit/parameter serializer, with no DSP or GUI.
#include "au_shim_types.h"
#include <stdlib.h>
#include <string.h>
static double value = 0.25;
static void *create(void) { return &value; }
static void destroy(void *ctx) {}
static double get(void *ctx, uint32_t id) { return value; }
static void set(void *ctx, uint32_t id, double v) { value = v; }
static void save(void *ctx, uint8_t **data, uint32_t *len) {
    *len = sizeof(value); *data = malloc(*len); memcpy(*data, &value, *len);
}
static void load(void *ctx, const uint8_t *data, uint32_t len) {
    if (data && len == sizeof(value)) memcpy(&value, data, len);
}
static void free_state(uint8_t *data, uint32_t len) { free(data); }
static uint32_t key_count(void *ctx) { return 1; }
static const char *key_at(void *ctx, uint32_t index) { return "legacy_state"; }
static int32_t load_foreign(void *ctx, const char *key, const uint8_t *data, uint32_t len) {
    if (len != sizeof(value)) return 0;
    load(ctx, data, len); return 1;
}
static const AuCallbacks callbacks = {
    .abi_version = TRUCE_AU_ABI_VERSION, .create = create, .destroy = destroy,
    .param_get_value = get, .param_set_value = set,
    .state_save = save, .state_load = load, .state_free = free_state,
    .legacy_state_key_count = key_count, .legacy_state_key_at = key_at,
    .state_load_foreign = load_foreign,
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
const AuParamDescriptor *g_param_descriptors = params;
uint32_t g_num_params = 1;
void truce_au_init(void) {}
