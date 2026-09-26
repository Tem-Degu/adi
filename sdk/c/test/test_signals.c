/*
 * test_signals.c - The appliance to engine direction.
 *
 * Until these existed libadip could be told what to do and could acknowledge
 * it, and could report no measurement of any kind. An engine reasons about
 * what the machine observed, so a device that cannot publish is a device the
 * engine has nothing to think about. Conformance Level 0 is signals and no
 * tools at all, and was not implementable.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip.h"
#include "adip_cbor.h"
#include "adip_generated_neutral.h"

#include <stdio.h>
#include <string.h>

static int failures = 0;
static int checks = 0;

#define CHECK(cond, msg)                                                     \
    do {                                                                     \
        checks++;                                                            \
        if (!(cond)) {                                                       \
            failures++;                                                      \
            printf("  FAIL %s:%d  %s\n", __FILE__, __LINE__, (msg));         \
        }                                                                    \
    } while (0)

/* ------------------------------------------------------------- port layer */

static uint32_t now_ms = 1000;
static uint8_t  last_frame[512];
static size_t   last_len = 0;
static int      write_calls = 0;

uint32_t adip_port_millis(void) { return now_ms; }
void adip_port_safe_state(void) { }

int adip_port_write(const uint8_t *buf, size_t len)
{
    write_calls++;
    if (len <= sizeof(last_frame)) {
        memcpy(last_frame, buf, len);
        last_len = len;
    }
    return (int)len;
}

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    (void)inv; (void)reason; (void)user;
    return ADIP_ACTION_DONE;
}

/* --------------------------------------------------------------- decoding */

/* What a decoded signal frame looked like, so assertions read as intent. */
typedef struct {
    uint8_t    kind;
    uint8_t    count;
    uint16_t   signal[ADIP_MAX_BATCH];
    adip_num_t value[ADIP_MAX_BATCH];
    uint32_t   t_ms[ADIP_MAX_BATCH];
    bool       ok;
} decoded_t;

static decoded_t decode_last(void)
{
    decoded_t d;
    const uint8_t *payload;
    size_t payload_len;
    size_t used;
    adip_cbor_t c;
    uint64_t map_count;
    uint64_t i;
    uint64_t key;

    memset(&d, 0, sizeof(d));

    if (adip_frame_find(last_frame, last_len, &payload, &payload_len,
                        &used) != ADIP_OK) {
        return d;
    }

    c.buf = payload;
    c.len = payload_len;
    c.pos = 0;

    if (!adip_cbor_map(&c, &map_count)) {
        return d;
    }

    for (i = 0; i < map_count; i++) {
        if (!adip_cbor_uint(&c, &key)) {
            return d;
        }
        if (key == 0u) {
            uint64_t v;
            if (!adip_cbor_uint(&c, &v)) {
                return d;
            }
            d.kind = (uint8_t)v;
        } else if (key == 1u) {
            uint8_t mt;
            uint64_t n;
            uint64_t j;
            if (!adip_cbor_head(&c, &mt, &n) || mt != 4u ||
                n > ADIP_MAX_BATCH) {
                return d;
            }
            for (j = 0; j < n; j++) {
                uint8_t tmt;
                uint64_t triple;
                uint64_t idx;
                uint64_t t;
                if (!adip_cbor_head(&c, &tmt, &triple) || tmt != 4u ||
                    triple != 3u) {
                    return d;
                }
                if (!adip_cbor_uint(&c, &idx) ||
                    !adip_cbor_quantity(&c, &d.value[j]) ||
                    !adip_cbor_uint(&c, &t)) {
                    return d;
                }
                d.signal[j] = (uint16_t)idx;
                d.t_ms[j] = (uint32_t)t;
            }
            d.count = (uint8_t)n;
        } else if (!adip_cbor_skip(&c)) {
            return d;
        }
    }

    d.ok = true;
    return d;
}

/* ------------------------------------------------------------------ setup */

static uint8_t rx[ADIP_RX_MIN];
static adip_ctx_t ctx;
static adip_sample_t batch[ADIP_MAX_BATCH];

static void reset_all(void)
{
    now_ms = 1000;
    last_len = 0;
    write_calls = 0;
    memset(last_frame, 0, sizeof(last_frame));
    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);
}

/* The mass signal, index 0 in the neutral declaration, is declared in grams. */
static adip_num_t grams(int32_t mantissa, int8_t exponent)
{
    adip_num_t n;
    n.mantissa = mantissa;
    n.exponent = exponent;
    n.unit = (uint8_t)ADIP_UNIT_G;
    return n;
}

/* ------------------------------------------------------------------ tests */

static void test_a_measurement_reaches_the_wire(void)
{
    decoded_t d;

    printf("a measurement reaches the wire\n");
    reset_all();

    CHECK(adip_publish(&ctx, 0u, grams(365, -1), 1234u) == ADIP_OK,
          "36.5 g is published");
    CHECK(write_calls == 1, "and sent immediately with no buffer set");

    d = decode_last();
    CHECK(d.ok, "the frame decodes");
    CHECK(d.kind == ADIP_KIND_SIGNALS, "as a signal frame");
    CHECK(d.count == 1u, "carrying one sample");
    CHECK(d.signal[0] == 0u, "for the right signal");
    CHECK(d.value[0].mantissa == 365, "the mantissa survives");
    CHECK(d.value[0].exponent == -1, "and the exponent");
    CHECK(d.value[0].unit == (uint8_t)ADIP_UNIT_G, "with its unit");
    CHECK(d.t_ms[0] == 1234u, "and the appliance timestamp");
}

/*
 * Exactness is the whole reason for tag 4. 36.5 has no binary float
 * representation, and a range check against a safety limit has to be exact
 * rather than nearly right.
 */
static void test_values_cross_without_rounding(void)
{
    decoded_t d;
    int i;
    const int32_t mantissas[] = { 1, 365, -365, 2147483647, -2147483647 };
    const int8_t  exponents[] = { 0, -1, -3, 2, -8 };

    printf("values cross without rounding\n");

    for (i = 0; i < 5; i++) {
        adip_num_t v = grams(mantissas[i], exponents[i]);
        reset_all();
        CHECK(adip_publish(&ctx, 0u, v, 1u) == ADIP_OK, "published");
        d = decode_last();
        CHECK(d.ok && d.value[0].mantissa == mantissas[i] &&
              d.value[0].exponent == exponents[i],
              "mantissa and exponent are unchanged");
    }
}

/*
 * The mirror of an invocation carrying the wrong unit. A firmware bug that
 * publishes millilitres into a signal declared in grams sends the engine a
 * plausible number, and the engine has no way to know.
 */
static void test_the_wrong_unit_is_refused(void)
{
    adip_num_t millilitres = grams(365, -1);
    millilitres.unit = (uint8_t)ADIP_UNIT_ML;

    printf("the wrong unit is refused\n");
    reset_all();

    CHECK(adip_publish(&ctx, 0u, millilitres, 1u) == ADIP_ERR_ARG,
          "millilitres into a mass signal are refused");
    CHECK(write_calls == 0, "and nothing was sent");
}

static void test_an_undeclared_signal_is_refused(void)
{
    printf("an undeclared signal is refused\n");
    reset_all();

    CHECK(adip_publish(&ctx, 99u, grams(1, 0), 1u) == ADIP_ERR_ARG,
          "a signal index the declaration does not have");
    CHECK(write_calls == 0, "sends nothing");
}

/*
 * PROTOCOL.md 10.1 asks that signal frames batch. A 20 Hz signal sending one
 * frame per sample is 20 notifications a second before anything else is on
 * the link.
 */
static void test_samples_batch(void)
{
    decoded_t d;
    uint8_t i;

    printf("samples batch\n");
    reset_all();
    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);

    for (i = 0; i < ADIP_MAX_BATCH - 1u; i++) {
        CHECK(adip_publish(&ctx, 0u, grams(i, 0), 100u + i) == ADIP_OK,
              "sample accepted");
    }
    CHECK(write_calls == 0, "nothing sent while the batch fills");

    CHECK(adip_publish(&ctx, 0u, grams(7, 0), 107u) == ADIP_OK,
          "the sample that fills it");
    CHECK(write_calls == 1, "flushes by itself");

    d = decode_last();
    CHECK(d.ok && d.count == ADIP_MAX_BATCH, "the whole batch is in one frame");
    CHECK(d.t_ms[0] == 100u && d.t_ms[ADIP_MAX_BATCH - 1u] == 107u,
          "in the order they were published");
}

static void test_flush_sends_a_partial_batch(void)
{
    decoded_t d;

    printf("flush sends a partial batch\n");
    reset_all();
    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);

    (void)adip_publish(&ctx, 0u, grams(1, 0), 10u);
    (void)adip_publish(&ctx, 0u, grams(2, 0), 20u);
    CHECK(write_calls == 0, "still pending");

    CHECK(adip_flush_signals(&ctx) == ADIP_OK, "flush succeeds");
    CHECK(write_calls == 1, "and sends");

    d = decode_last();
    CHECK(d.ok && d.count == 2u, "with just the two pending samples");

    CHECK(adip_flush_signals(&ctx) == ADIP_OK, "a second flush is harmless");
    CHECK(write_calls == 1, "and sends nothing");
}

/*
 * A publish must never stall the appliance's main loop, so a transport that
 * is not accepting bytes cannot be retried or queued indefinitely. Stale
 * telemetry is worse than a gap: it describes a machine that has moved on.
 */
static void test_a_failed_write_does_not_hold_samples(void)
{
    printf("a failed write does not hold samples\n");
    reset_all();
    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);

    (void)adip_publish(&ctx, 0u, grams(1, 0), 10u);
    (void)adip_flush_signals(&ctx);

    CHECK(ctx.sample_count == 0u, "the buffer is cleared after a flush");

    (void)adip_publish(&ctx, 0u, grams(2, 0), 20u);
    CHECK(ctx.sample_count == 1u, "and takes new samples afterwards");
}

/* Publishing is telemetry, not control, so it must not disturb an operation
 * or be blocked by one. */
static void test_publishing_is_independent_of_control(void)
{
    printf("publishing is independent of control\n");
    reset_all();

    CHECK(adip_publish(&ctx, 0u, grams(1, 0), 1u) == ADIP_OK,
          "an idle machine publishes");
    CHECK(ctx.stat_accepted == 0u, "and it is not counted as an invocation");
    CHECK(ctx.stat_rejected == 0u, "nor as a rejection");
    CHECK(!adip_busy(&ctx), "and it does not make the machine busy");
}

/* --------------------------------------------------------------- events */

/*
 * PROTOCOL.md 4.5 requires a fault event before or with entry to a safe
 * state. The library drives the safe state and cannot know what failed, so
 * the firmware raises it; what the library owes is the means.
 */
static void test_an_event_reaches_the_wire(void)
{
    const uint8_t *payload;
    size_t payload_len;
    size_t used;
    adip_cbor_t c;
    uint64_t map_count;
    uint64_t i;
    uint64_t key;
    uint8_t  kind = 0;
    uint64_t index = 99;
    uint64_t severity = 99;
    const char *code = NULL;
    size_t code_len = 0;

    printf("an event reaches the wire\n");
    reset_all();
    now_ms = 4242u;

    CHECK(adip_emit_event(&ctx, 0u, "overheated", 2u) == ADIP_OK,
          "a declared event is raised");
    CHECK(write_calls == 1, "and sent");

    CHECK(adip_frame_find(last_frame, last_len, &payload, &payload_len,
                          &used) == ADIP_OK, "the frame is well formed");
    c.buf = payload; c.len = payload_len; c.pos = 0;
    CHECK(adip_cbor_map(&c, &map_count), "and is a map");

    for (i = 0; i < map_count; i++) {
        if (!adip_cbor_uint(&c, &key)) { break; }
        if (key == 0u) { uint64_t v; (void)adip_cbor_uint(&c, &v); kind = (uint8_t)v; }
        else if (key == 1u) { (void)adip_cbor_uint(&c, &index); }
        else if (key == 3u) { (void)adip_cbor_text(&c, &code, &code_len); }
        else if (key == 4u) { (void)adip_cbor_uint(&c, &severity); }
        else { (void)adip_cbor_skip(&c); }
    }

    CHECK(kind == ADIP_KIND_EVENT, "tagged as an event");
    CHECK(index == 0u, "naming the declared event");
    CHECK(severity == 2u, "at the severity given");
    CHECK(code_len == 10u && memcmp(code, "overheated", 10) == 0,
          "carrying the firmware's own code");
}

static void test_an_undeclared_event_is_refused(void)
{
    printf("an undeclared event is refused\n");
    reset_all();

    CHECK(adip_emit_event(&ctx, 99u, "nonsense", 1u) == ADIP_ERR_ARG,
          "an event index the declaration does not have");
    CHECK(write_calls == 0, "sends nothing");
}

int main(void)
{
    printf("libadip signal tests\n\n");
    test_a_measurement_reaches_the_wire();
    test_values_cross_without_rounding();
    test_the_wrong_unit_is_refused();
    test_an_undeclared_signal_is_refused();
    test_samples_batch();
    test_flush_sends_a_partial_batch();
    test_a_failed_write_does_not_hold_samples();
    test_publishing_is_independent_of_control();
    test_an_event_reaches_the_wire();
    test_an_undeclared_event_is_refused();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
