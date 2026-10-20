/*
 * test_setpoints.c - What the machine is set to, as the machine reports it.
 *
 * ADI-P 9.1 requires that an operator can take control by physical means at
 * any moment. A setting can therefore change without the engine asking or
 * being told, which makes anything the engine merely remembers stale by
 * mandate. The appliance owns its own state, as it already owns its
 * interlocks, its preconditions and whether it is faulted, and this is the
 * direction that was missing: an engine cannot plan a correction without
 * knowing where the mechanism stands, and had no way to find out.
 *
 * The neutral declaration's tool 0 is `adi.example.actuator.set_position` and
 * its only parameter is `position`, declared in steps.
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

#define MAX_REPORTED 8

typedef struct {
    uint8_t    kind;
    uint8_t    count;
    uint16_t   tool[MAX_REPORTED];
    uint16_t   parameter[MAX_REPORTED];
    adip_num_t value[MAX_REPORTED];
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
            if (!adip_cbor_head(&c, &mt, &n) || mt != 4u || n > MAX_REPORTED) {
                return d;
            }
            for (j = 0; j < n; j++) {
                uint8_t tmt;
                uint64_t triple;
                uint64_t t;
                uint64_t p;
                if (!adip_cbor_head(&c, &tmt, &triple) || tmt != 4u ||
                    triple != 3u) {
                    return d;
                }
                if (!adip_cbor_uint(&c, &t) ||
                    !adip_cbor_uint(&c, &p) ||
                    !adip_cbor_quantity(&c, &d.value[j])) {
                    return d;
                }
                d.tool[j] = (uint16_t)t;
                d.parameter[j] = (uint16_t)p;
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
static adip_setpoint_t held[4];

static void reset_all(void)
{
    now_ms = 1000;
    last_len = 0;
    write_calls = 0;
    memset(last_frame, 0, sizeof(last_frame));
    memset(held, 0, sizeof(held));
    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);
    adip_set_setpoint_buffer(&ctx, held, 4u);
}

/* `position`, the only parameter of tool 0, is declared in steps. */
static adip_num_t steps(int32_t mantissa)
{
    adip_num_t n;
    n.mantissa = mantissa;
    n.exponent = 0;
    n.unit = (uint8_t)ADIP_UNIT_STEP;
    return n;
}

/* ------------------------------------------------------------------ tests */

static void test_a_setpoint_reaches_the_wire(void)
{
    decoded_t d;

    reset_all();

    CHECK(adip_report_setpoint(&ctx, 0u, 0u, steps(24)) == ADIP_OK,
          "a declared parameter in its declared unit is accepted");

    /* Nothing is sent from the report itself: several changes in one control
     * pass should cost one frame, so sending belongs to the poll. */
    CHECK(write_calls == 0, "and nothing is sent before the next poll");

    (void)adip_poll(&ctx);

    CHECK(write_calls == 1, "the poll sends it");
    d = decode_last();
    CHECK(d.ok, "the frame decodes");
    CHECK(d.kind == ADIP_KIND_SETPOINTS, "as a setpoint report");
    CHECK(d.count == 1u, "carrying one entry");
    CHECK(d.tool[0] == 0u, "for the right tool");
    CHECK(d.parameter[0] == 0u, "and the right parameter");
    CHECK(d.value[0].mantissa == 24, "the mantissa survives");
    CHECK(d.value[0].unit == (uint8_t)ADIP_UNIT_STEP, "with its unit");
}

/* A report is sent when something changed. An engine that has been told 24
 * does not need telling again, and a frame per poll would put the link under
 * continuous traffic for no information. */
static void test_an_unchanged_value_sends_nothing(void)
{
    reset_all();

    (void)adip_report_setpoint(&ctx, 0u, 0u, steps(24));
    (void)adip_poll(&ctx);
    CHECK(write_calls == 1, "the first report is sent");

    (void)adip_report_setpoint(&ctx, 0u, 0u, steps(24));
    (void)adip_poll(&ctx);
    CHECK(write_calls == 1, "saying the same thing again sends nothing");

    (void)adip_report_setpoint(&ctx, 0u, 0u, steps(25));
    (void)adip_poll(&ctx);
    CHECK(write_calls == 2, "a change does send");
}

/* Every value the engine plans from is checked against the declaration, in
 * this direction as well as the other. A setpoint in the wrong quantity would
 * have the engine send a value in a unit the machine never had. */
static void test_a_wrong_unit_is_refused(void)
{
    adip_num_t grams;

    reset_all();

    grams.mantissa = 24;
    grams.exponent = 0;
    grams.unit = (uint8_t)ADIP_UNIT_G;

    CHECK(adip_report_setpoint(&ctx, 0u, 0u, grams) == ADIP_ERR_ARG,
          "position is declared in steps, not grams");
    (void)adip_poll(&ctx);
    CHECK(write_calls == 0, "and nothing reaches the wire");
}

static void test_an_undeclared_address_is_refused(void)
{
    reset_all();

    CHECK(adip_report_setpoint(&ctx, 99u, 0u, steps(1)) == ADIP_ERR_ARG,
          "no such tool");
    CHECK(adip_report_setpoint(&ctx, 0u, 99u, steps(1)) == ADIP_ERR_ARG,
          "no such parameter on that tool");
    (void)adip_poll(&ctx);
    CHECK(write_calls == 0, "and nothing reaches the wire");
}

/* The buffer is the caller's, as the receive buffer and the sample batch are,
 * so a machine that reports nothing pays no RAM for the ability. One that has
 * not provided it is refused rather than silently reporting nothing. */
static void test_without_a_buffer_a_report_is_refused(void)
{
    reset_all();
    adip_set_setpoint_buffer(&ctx, NULL, 0u);

    CHECK(adip_report_setpoint(&ctx, 0u, 0u, steps(24)) == ADIP_ERR_ARG,
          "there is nowhere to keep it");
    (void)adip_poll(&ctx);
    CHECK(write_calls == 0, "and nothing is sent");
}

/* An engine that has just connected believes nothing, or worse, inherited
 * what the last one believed. ADI-P 9.1 has the engine open a session and
 * defines no message for it, so a heartbeat after a silence longer than the
 * control timeout is the only evidence an appliance has that the peer is new.
 */
static void test_a_new_session_is_told_again(void)
{
    uint8_t beat[32];
    int framed;

    reset_all();

    (void)adip_report_setpoint(&ctx, 0u, 0u, steps(24));
    (void)adip_poll(&ctx);
    CHECK(write_calls == 1, "the first session was told");

    (void)adip_poll(&ctx);
    CHECK(write_calls == 1, "and is not told repeatedly");

    /* A heartbeat arriving after a long gap. */
    now_ms += 60000u;
    framed = adip_encode_heartbeat(false, false, beat, sizeof(beat));
    CHECK(framed > 0, "a heartbeat encodes");
    if (framed > 0) {
        uint8_t wire[64];
        int wrapped = adip_frame_wrap(beat, (size_t)framed, wire, sizeof(wire));
        CHECK(wrapped > 0, "and frames");
        if (wrapped > 0) {
            int before = write_calls;
            (void)adip_feed(&ctx, wire, (size_t)wrapped);
            while (adip_poll(&ctx) == ADIP_PENDING) {
            }
            (void)adip_poll(&ctx);
            /* The heartbeat reply is one write; the report is another. */
            CHECK(write_calls >= before + 2,
                  "a peer that was not there a moment ago is told afresh");
        }
    }
}

/* Several parameters in one frame, because a machine with four dials should
 * not cost four frames to describe. */
static void test_several_parameters_travel_together(void)
{
    decoded_t d;

    reset_all();

    (void)adip_report_setpoint(&ctx, 0u, 0u, steps(24));
    /* Tool 1's first parameter, whatever the neutral declaration calls it,
     * reported in its own declared unit. */
    if (adip_device_neutral.tool_count > 1u &&
        adip_device_neutral.tools[1].param_count > 0u) {
        adip_num_t v = adip_device_neutral.tools[1].params[0].min;
        (void)adip_report_setpoint(&ctx, 1u, 0u, v);
    }

    (void)adip_poll(&ctx);

    CHECK(write_calls == 1, "one frame, however many changed");
    d = decode_last();
    CHECK(d.ok, "it decodes");
    CHECK(d.kind == ADIP_KIND_SETPOINTS, "as a setpoint report");
    CHECK(d.count >= 1u, "carrying what was reported");
}

int main(void)
{
    test_a_setpoint_reaches_the_wire();
    test_an_unchanged_value_sends_nothing();
    test_a_wrong_unit_is_refused();
    test_an_undeclared_address_is_refused();
    test_without_a_buffer_a_report_is_refused();
    test_a_new_session_is_told_again();
    test_several_parameters_travel_together();

    printf("%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
