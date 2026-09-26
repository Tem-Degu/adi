/*
 * test_m3.c - Dispatch and reply.
 *
 * This file is also the worked example of the porting contract: it implements
 * the three port functions and one handler, which is everything a manufacturer
 * has to write.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdio.h>
#include <string.h>

#include "adip.h"
#include "adip_cbor.h"
#include "adip_generated_neutral.h"

static int failures = 0;
static int checks = 0;

#define CHECK(cond, msg)                                                       \
    do {                                                                       \
        checks++;                                                              \
        if (!(cond)) {                                                         \
            failures++;                                                        \
            printf("  FAIL %s:%d  %s\n", __FILE__, __LINE__, (msg));           \
        }                                                                      \
    } while (0)

/* ------------------------------------------------------------- the port */

static uint32_t fake_now_ms = 0;

uint32_t adip_port_millis(void)
{
    return fake_now_ms;
}

/* Captured responses, so tests can assert on what went out. */
#define MAX_SENT 16
typedef struct {
    uint32_t       id;
    adip_outcome_t outcome;
    adip_reason_t  reason;
} sent_t;

static sent_t sent[MAX_SENT];
static int    sent_count = 0;
static int    safe_state_calls = 0;

/* Decode a response frame so assertions read as intent rather than bytes. */
static void record(const uint8_t *frame, size_t len)
{
    const uint8_t *payload;
    size_t payload_len;
    size_t used;
    adip_cbor_t c;
    uint64_t count;
    uint64_t i;
    uint64_t key;
    uint64_t v;
    sent_t s;

    if (adip_frame_find(frame, len, &payload, &payload_len, &used) != ADIP_OK) {
        return;
    }

    c.buf = payload;
    c.len = payload_len;
    c.pos = 0;

    memset(&s, 0, sizeof(s));

    /* Response map keys: 1 = invocation id, 2 = outcome, 3 = reason. */
    if (!adip_cbor_map(&c, &count)) {
        return;
    }
    for (i = 0; i < count; i++) {
        if (!adip_cbor_uint(&c, &key) || !adip_cbor_uint(&c, &v)) {
            return;
        }
        switch (key) {
            case 1u: s.id = (uint32_t)v; break;
            case 2u: s.outcome = (adip_outcome_t)v; break;
            case 3u: s.reason = (adip_reason_t)v; break;
            default: break;
        }
    }

    if (sent_count < MAX_SENT) {
        sent[sent_count++] = s;
    }
}

int adip_port_write(const uint8_t *buf, size_t len)
{
    record(buf, len);
    return (int)len;
}

void adip_port_safe_state(void)
{
    safe_state_calls++;
}

/* ----------------------------------------------------------- the handler */

static adip_action_t next_action = ADIP_ACTION_DONE;
static adip_reason_t next_reject_reason = ADIP_REASON_FAULT;
static int handler_calls = 0;
static uint32_t handler_last_id = 0;

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    (void)user;
    handler_calls++;
    handler_last_id = inv->invocation_id;

    if (next_action == ADIP_ACTION_REJECTED) {
        *reason = next_reject_reason;
    }
    return next_action;
}

/* ------------------------------------------------------------- fixtures */

typedef struct {
    uint8_t buf[256];
    size_t  pos;
} builder_t;

static void b_head(builder_t *b, uint8_t mt, uint64_t v)
{
    if (v < 24u) {
        b->buf[b->pos++] = (uint8_t)((mt << 5) | (uint8_t)v);
    } else if (v <= 0xffu) {
        b->buf[b->pos++] = (uint8_t)((mt << 5) | 24u);
        b->buf[b->pos++] = (uint8_t)v;
    } else {
        b->buf[b->pos++] = (uint8_t)((mt << 5) | 25u);
        b->buf[b->pos++] = (uint8_t)((v >> 8) & 0xffu);
        b->buf[b->pos++] = (uint8_t)(v & 0xffu);
    }
}

static void b_uint(builder_t *b, uint64_t v) { b_head(b, 0u, v); }

static void b_int(builder_t *b, int64_t v)
{
    if (v < 0) { b_head(b, 1u, (uint64_t)(-1 - v)); }
    else { b_uint(b, (uint64_t)v); }
}

static void b_text(builder_t *b, const char *s)
{
    size_t n = strlen(s);
    b_head(b, 3u, n);
    memcpy(b->buf + b->pos, s, n);
    b->pos += n;
}

static void b_decimal(builder_t *b, int32_t m, int8_t e)
{
    b_head(b, 6u, 4u);
    b_head(b, 4u, 2u);
    b_int(b, e);
    b_int(b, m);
}

static void b_quantity(builder_t *b, uint8_t unit, int32_t m, int8_t e)
{
    b_head(b, 4u, 2u);      /* [unit, number] */
    b_uint(b, unit);
    b_decimal(b, m, e);
}

/* The unit a parameter is declared in, so a fixture states what the
 * declaration says rather than repeating it by hand. PROTOCOL.md 10.2.3. */
static uint8_t unit_for(const char *tool_id, const char *param)
{
    const adip_tool_t *t;
    uint8_t i;

    t = adip_find_tool_n(&adip_device_neutral, tool_id, strlen(tool_id));
    if (t == NULL) {
        return (uint8_t)ADIP_UNIT_NONE;
    }
    for (i = 0; i < t->param_count; i++) {
        if (strcmp(t->params[i].name, param) == 0) {
            return t->params[i].min.unit;
        }
    }
    return (uint8_t)ADIP_UNIT_NONE;
}


/* One framed call, ready to feed. */
static size_t make_call(uint8_t *out, size_t out_len, const char *tool,
                        uint32_t id, const char *arg, int32_t m, int8_t e)
{
    builder_t b;
    int n;

    b.pos = 0;
    b_head(&b, 5u, 4u);
    b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
    b_uint(&b, 1u); b_text(&b, tool);
    b_uint(&b, 2u); b_uint(&b, id);
    b_uint(&b, 3u);
    b_head(&b, 5u, 1u);
    b_text(&b, arg);
    b_quantity(&b, unit_for(tool, arg), m, e);

    n = adip_frame_wrap(b.buf, b.pos, out, out_len);
    return (n > 0) ? (size_t)n : 0u;
}

static uint8_t rx[ADIP_RX_MIN];
static adip_ctx_t ctx;

static void reset_all(void)
{
    sent_count = 0;
    handler_calls = 0;
    handler_last_id = 0;
    safe_state_calls = 0;
    next_action = ADIP_ACTION_DONE;
    fake_now_ms = 1000;

    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);

    /* The firmware reports its state. Until it does, nothing with a declared
     * precondition is invocable, which is the safe default. */
    {
        uint8_t i;
        for (i = 0; i < adip_device_neutral.precondition_count; i++) {
            adip_set_precondition(&ctx, i, true);
        }
    }
}

static void run(const uint8_t *frame, size_t len)
{
    (void)adip_feed(&ctx, frame, len);
    while (adip_poll(&ctx) == ADIP_PENDING) {
        /* drain */
    }
}

/* ----------------------------------------------------------------- tests */

static void test_init(void)
{
    adip_ctx_t local;
    uint8_t small[8];

    printf("init\n");

    CHECK(adip_init(&local, &adip_device_neutral, rx, sizeof(rx), NULL) ==
              ADIP_OK, "init succeeds with a large enough buffer");

    /* A buffer too small to hold a maximum frame would wedge the receiver in
     * the field. Refuse at init instead. */
    CHECK(adip_init(&local, &adip_device_neutral, small, sizeof(small),
                    NULL) == ADIP_ERR_NO_SPACE,
          "a buffer that could wedge is refused at init");

    CHECK(adip_init(&local, NULL, rx, sizeof(rx), NULL) == ADIP_ERR_ARG,
          "a null device is refused");

    /* Fails closed at startup: no precondition satisfied. */
    (void)adip_init(&local, &adip_device_neutral, rx, sizeof(rx), NULL);
    CHECK(local.satisfied_preconditions == 0u,
          "no precondition is satisfied before the firmware reports state");
    CHECK(local.open_interlocks == 0u, "no interlock is open at init");
    CHECK(!adip_busy(&local), "not busy at init");
}

static void test_synchronous(void)
{
    uint8_t frame[128];
    size_t n;

    printf("synchronous dispatch\n");

    reset_all();
    next_action = ADIP_ACTION_DONE;
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 41u,
                  "mass", 90, -1);
    run(frame, n);

    CHECK(handler_calls == 1, "the handler was called once");
    CHECK(handler_last_id == 41u, "with the right invocation id");
    CHECK(sent_count == 2, "accepted then completed");
    CHECK(sent[0].outcome == ADIP_OUTCOME_ACCEPTED, "first is accepted");
    CHECK(sent[1].outcome == ADIP_OUTCOME_COMPLETED, "second is completed");
    CHECK(sent[0].id == 41u && sent[1].id == 41u, "both carry the id");
    CHECK(!adip_busy(&ctx), "not busy after a synchronous call");
    CHECK(ctx.stat_accepted == 1u, "counted as accepted");
}

static void test_asynchronous(void)
{
    uint8_t frame[128];
    size_t n;

    printf("asynchronous dispatch\n");

    reset_all();
    next_action = ADIP_ACTION_RUNNING;
    n = make_call(frame, sizeof(frame), "adi.example.process.start", 7u,
                  "stop_at_volume", 360, -1);
    run(frame, n);

    CHECK(sent_count == 1, "only accepted so far");
    CHECK(sent[0].outcome == ADIP_OUTCOME_ACCEPTED, "accepted");
    CHECK(adip_busy(&ctx), "busy while the operation runs");
    CHECK(ctx.active_id == 7u, "the operation is tracked");
    CHECK(ctx.active_started_ms == 1000u, "the start time was recorded");

    /* A second call while busy is refused, not queued. Queuing would mean
     * holding an invocation whose preconditions may no longer hold. */
    {
        uint8_t second[128];
        size_t m = make_call(second, sizeof(second), "adi.example.input.by_mass",
                             8u, "mass", 90, -1);
        int before = handler_calls;
        run(second, m);
        CHECK(handler_calls == before,
              "the handler is not called while busy");
        CHECK(sent_count == 2, "the second call was answered");
        CHECK(sent[1].outcome == ADIP_OUTCOME_REJECTED, "rejected");
        CHECK(sent[1].reason == ADIP_REASON_BUSY, "with BUSY");
        CHECK(sent[1].id == 8u, "answering the second call, not the first");
    }

    /* Firmware finishes the movement. */
    fake_now_ms += 6000u;
    CHECK(adip_complete(&ctx, 7u, ADIP_OUTCOME_COMPLETED, ADIP_REASON_NONE) ==
              ADIP_OK, "completing the operation in flight");
    CHECK(!adip_busy(&ctx), "no longer busy");
    CHECK(sent_count == 3, "completion was sent");
    CHECK(sent[2].outcome == ADIP_OUTCOME_COMPLETED, "completed");
    CHECK(sent[2].id == 7u, "with the right id");

    /* Completing something that is not running is a firmware bug, and is
     * reported rather than sent as an unmatched response. */
    CHECK(adip_complete(&ctx, 7u, ADIP_OUTCOME_COMPLETED, ADIP_REASON_NONE) ==
              ADIP_ERR_STATE, "completing twice is refused");
    CHECK(adip_complete(&ctx, 999u, ADIP_OUTCOME_COMPLETED,
                        ADIP_REASON_NONE) == ADIP_ERR_STATE,
          "completing an unknown id is refused");
}

static void test_abort(void)
{
    uint8_t frame[128];
    size_t n;

    printf("abort path\n");

    reset_all();
    next_action = ADIP_ACTION_RUNNING;
    n = make_call(frame, sizeof(frame), "adi.example.process.start", 11u,
                  "stop_at_volume", 360, -1);
    run(frame, n);

    /* A mechanism that jams reports ABORTED with a reason, not silence. */
    CHECK(adip_complete(&ctx, 11u, ADIP_OUTCOME_ABORTED, ADIP_REASON_FAULT) ==
              ADIP_OK, "aborting is allowed");
    CHECK(sent[sent_count - 1].outcome == ADIP_OUTCOME_ABORTED, "aborted");
    CHECK(sent[sent_count - 1].reason == ADIP_REASON_FAULT, "with a reason");
    CHECK(!adip_busy(&ctx), "the machine is free again after an abort");

    /* ACCEPTED is not a valid completion outcome.
     *
     * Time is advanced past the declared 25 s recovery first. Without that the
     * machine rate limits the second run and it never starts, which is correct
     * behaviour and would make this test measure the wrong thing. */
    fake_now_ms += 30000u;
    next_action = ADIP_ACTION_RUNNING;
    n = make_call(frame, sizeof(frame), "adi.example.process.start", 12u,
                  "stop_at_volume", 360, -1);
    run(frame, n);
    CHECK(adip_complete(&ctx, 12u, ADIP_OUTCOME_ACCEPTED, ADIP_REASON_NONE) ==
              ADIP_ERR_ARG, "only completed or aborted may finish an operation");
}

static void test_firmware_refusal(void)
{
    uint8_t frame[128];
    size_t n;

    printf("firmware refusal\n");

    reset_all();
    next_action = ADIP_ACTION_REJECTED;
    next_reject_reason = ADIP_REASON_FAULT;
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 21u,
                  "mass", 90, -1);
    run(frame, n);

    /*
     * The handler is called before ACCEPTED is sent, so firmware can still
     * refuse for a reason only it knows. If ACCEPTED went first, this would
     * have to be an abort of something that never started.
     */
    CHECK(sent_count == 1, "exactly one response");
    CHECK(sent[0].outcome == ADIP_OUTCOME_REJECTED, "rejected");
    CHECK(sent[0].reason == ADIP_REASON_FAULT, "with the firmware's reason");
    CHECK(!adip_busy(&ctx), "nothing is left in flight");
}

static void test_rejections_reach_the_wire(void)
{
    uint8_t frame[128];
    size_t n;

    printf("rejections\n");

    /* Out of range: refused, never clamped, and counted as an engine defect. */
    reset_all();
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 31u,
                  "mass", 400, -1);
    run(frame, n);
    CHECK(handler_calls == 0, "the handler never sees an invalid call");
    CHECK(sent_count == 1 && sent[0].outcome == ADIP_OUTCOME_REJECTED,
          "rejected");
    CHECK(sent[0].reason == ADIP_REASON_OUT_OF_RANGE, "out of range");
    CHECK(ctx.stat_engine_defects == 1u,
          "an out-of-range value is counted as an engine defect");

    /* An open interlock outranks valid arguments. */
    reset_all();
    {
        uint8_t i;
        for (i = 0; i < adip_device_neutral.interlock_count; i++) {
            if (adip_device_neutral.interlocks[i].blocks_mask ==
                ADIP_BLOCKS_ALL) {
                adip_set_interlock(&ctx, i, true);
            }
        }
    }
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 32u,
                  "mass", 90, -1);
    run(frame, n);
    CHECK(sent[0].reason == ADIP_REASON_INTERLOCK, "interlock");
    CHECK(handler_calls == 0, "the handler is not reached");

    /* An unknown tool is named as such. */
    reset_all();
    n = make_call(frame, sizeof(frame), "x.nobody.invented.this", 33u, "mass",
                  90, -1);
    run(frame, n);
    CHECK(sent[0].reason == ADIP_REASON_UNKNOWN_TOOL, "unknown tool");
}

static void test_stream_robustness(void)
{
    uint8_t frame[128];
    size_t n;
    size_t i;

    printf("stream robustness\n");

    /* A frame split across many feeds must still dispatch exactly once. */
    reset_all();
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 51u,
                  "mass", 90, -1);
    for (i = 0; i < n; i++) {
        (void)adip_feed(&ctx, frame + i, 1);
        while (adip_poll(&ctx) == ADIP_PENDING) { }
    }
    CHECK(handler_calls == 1, "a byte-at-a-time frame dispatches once");
    CHECK(sent_count == 2, "and is answered");

    /* Noise before a good frame is skipped without losing the frame. */
    reset_all();
    {
        uint8_t noise[3] = { 0x00u, 0xffu, 0x7fu };
        n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 52u,
                      "mass", 90, -1);
        (void)adip_feed(&ctx, noise, sizeof(noise));
        (void)adip_feed(&ctx, frame, n);
        while (adip_poll(&ctx) == ADIP_PENDING) { }
        CHECK(handler_calls == 1, "the frame survives leading noise");
    }

    /* A corrupted frame is dropped and the next one still works. */
    reset_all();
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 53u,
                  "mass", 90, -1);
    frame[5] ^= 0xffu;
    run(frame, n);
    CHECK(handler_calls == 0, "a corrupted frame is not dispatched");
    CHECK(ctx.stat_malformed > 0u, "and is counted");

    frame[5] ^= 0xffu;
    run(frame, n);
    CHECK(handler_calls == 1, "the next good frame still works");

    /* Two frames in one feed. */
    reset_all();
    {
        uint8_t both[256];
        size_t a = make_call(both, sizeof(both), "adi.example.input.by_mass",
                             61u, "mass", 90, -1);
        size_t b = make_call(both + a, sizeof(both) - a,
                             "adi.example.input.by_mass", 62u, "mass", 100, -1);
        run(both, a + b);
        CHECK(handler_calls == 2, "both frames dispatch");
        CHECK(sent_count == 4, "both are answered");
    }

    /* Garbage that never contains a frame must not wedge the receiver. */
    reset_all();
    {
        uint8_t junk[64];
        size_t fed = 0;
        memset(junk, ADIP_FRAME_SYNC, sizeof(junk));
        while (fed < ADIP_RX_MIN) {
            size_t chunk = sizeof(junk);
            if (fed + chunk > ADIP_RX_MIN) {
                chunk = ADIP_RX_MIN - fed;
            }
            if (adip_feed(&ctx, junk, chunk) != ADIP_OK) {
                break;
            }
            fed += chunk;
            while (adip_poll(&ctx) == ADIP_PENDING) { }
        }
        CHECK(ctx.rx_len < ctx.rx_cap,
              "the receiver does not wedge on unframed garbage");

        n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 71u,
                      "mass", 90, -1);
        run(frame, n);
        CHECK(handler_calls >= 1, "and recovers to accept a real frame");
    }
}

int main(void)
{
    printf("libadip M3 tests\n\n");
    test_init();
    test_synchronous();
    test_asynchronous();
    test_abort();
    test_firmware_refusal();
    test_rejections_reach_the_wire();
    test_stream_robustness();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
