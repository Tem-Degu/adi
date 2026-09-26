/*
 * test_m5.c - Failsafe and timing.
 *
 * These test the paths that only run when something has gone wrong, which are
 * the paths least likely to be exercised in normal use and most likely to
 * matter when they are.
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

/* ------------------------------------------------------------------- port */

static uint32_t now_ms = 1000;
static int safe_state_calls = 0;
static uint32_t handler_delay_ms = 0;

uint32_t adip_port_millis(void) { return now_ms; }
void adip_port_safe_state(void) { safe_state_calls++; }

typedef struct {
    uint32_t       id;
    adip_outcome_t outcome;
    adip_reason_t  reason;
} sent_t;

#define MAX_SENT 16
static sent_t sent[MAX_SENT];
static int    sent_count = 0;

int adip_port_write(const uint8_t *buf, size_t len)
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

    if (adip_frame_find(buf, len, &payload, &payload_len, &used) != ADIP_OK) {
        return (int)len;
    }

    c.buf = payload; c.len = payload_len; c.pos = 0;
    memset(&s, 0, sizeof(s));
    if (!adip_cbor_map(&c, &count)) {
        return (int)len;
    }
    for (i = 0; i < count; i++) {
        if (!adip_cbor_uint(&c, &key) || !adip_cbor_uint(&c, &v)) {
            break;
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
    return (int)len;
}

static adip_action_t next_action = ADIP_ACTION_RUNNING;

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    (void)inv; (void)reason; (void)user;
    /* A handler that blocks is the thing the deadline counter exists to
     * expose, so one test makes it do exactly that. */
    now_ms += handler_delay_ms;
    return next_action;
}

/* --------------------------------------------------------------- fixtures */

typedef struct { uint8_t buf[256]; size_t pos; } builder_t;

static void b_head(builder_t *b, uint8_t mt, uint64_t v)
{
    if (v < 24u) { b->buf[b->pos++] = (uint8_t)((mt << 5) | (uint8_t)v); }
    else if (v <= 0xffu) {
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
    if (v < 0) { b_head(b, 1u, (uint64_t)(-1 - v)); } else { b_uint(b, (uint64_t)v); }
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
    b_head(b, 6u, 4u); b_head(b, 4u, 2u); b_int(b, e); b_int(b, m);
}

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
    b_decimal(&b, m, e);
    n = adip_frame_wrap(b.buf, b.pos, out, out_len);
    return (n > 0) ? (size_t)n : 0u;
}

/* A heartbeat, as the engine actually sends one. PROTOCOL.md 10.2.5. */
static size_t make_heartbeat(uint8_t *out, size_t out_len)
{
    builder_t b;
    int n;
    b.pos = 0;
    b_head(&b, 5u, 1u);
    b_uint(&b, 0u); b_uint(&b, ADIP_KIND_HEARTBEAT);
    n = adip_frame_wrap(b.buf, b.pos, out, out_len);
    return (n > 0) ? (size_t)n : 0u;
}

static uint8_t rx[ADIP_RX_MIN];
static adip_ctx_t ctx;

static void reset_all(void)
{
    uint8_t i;
    now_ms = 1000;
    sent_count = 0;
    safe_state_calls = 0;
    handler_delay_ms = 0;
    next_action = ADIP_ACTION_RUNNING;
    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);
    for (i = 0; i < adip_device_neutral.precondition_count; i++) {
        adip_set_precondition(&ctx, i, true);
    }
}

static void run(const uint8_t *frame, size_t len)
{
    (void)adip_feed(&ctx, frame, len);
    while (adip_poll(&ctx) == ADIP_PENDING) { }
}

/* Start a non-reversible operation and leave it running. */
static void start_process(uint32_t id)
{
    uint8_t frame[128];
    size_t n = make_call(frame, sizeof(frame), "adi.example.process.start", id,
                         "stop_at_volume", 360, -1);
    run(frame, n);
}

/* ----------------------------------------------------------------- tests */

static void test_link_timeout(void)
{
    printf("link timeout\n");
    reset_all();

    start_process(1u);
    CHECK(adip_busy(&ctx), "the operation is running");
    CHECK(safe_state_calls == 0, "no failsafe yet");

    /* Inside the declared 2000 ms control timeout. */
    now_ms += 1500u;
    (void)adip_poll(&ctx);
    CHECK(adip_busy(&ctx), "still running inside the timeout");
    CHECK(safe_state_calls == 0, "and no failsafe");

    /* Past it. */
    now_ms += 1000u;
    (void)adip_poll(&ctx);
    CHECK(!adip_busy(&ctx), "the operation was stopped");
    CHECK(safe_state_calls == 1, "the safe state was driven exactly once");
    CHECK(adip_faulted(&ctx), "the machine is faulted");
    CHECK(ctx.stat_link_timeouts == 1u, "counted");

    CHECK(sent_count >= 2, "an abort was sent");
    CHECK(sent[sent_count - 1].outcome == ADIP_OUTCOME_ABORTED, "aborted");
    CHECK(sent[sent_count - 1].id == 1u, "for the operation that was running");

    /* Repeated polls must not drive the safe state again. */
    now_ms += 5000u;
    (void)adip_poll(&ctx);
    (void)adip_poll(&ctx);
    CHECK(safe_state_calls == 1, "the failsafe does not re-fire");
}

static void test_idle_link_loss_is_not_a_fault(void)
{
    printf("idle link loss\n");
    reset_all();

    /* Nothing is running. A quiet link is then merely a quiet link, and
     * faulting an idle machine would be a nuisance rather than a safeguard. */
    now_ms += 60000u;
    (void)adip_poll(&ctx);
    CHECK(safe_state_calls == 0, "an idle machine does not fault");
    CHECK(!adip_faulted(&ctx), "and is not marked faulted");

    /* And it still works afterwards. */
    start_process(2u);
    CHECK(adip_busy(&ctx), "a later invocation is accepted");
}

static void test_reversible_is_stopped_but_does_not_fault(void)
{
    uint8_t frame[128];
    size_t n;

    printf("reversible operations\n");
    reset_all();

    /* set_position is declared reversible. */
    n = make_call(frame, sizeof(frame), "adi.example.actuator.set_position", 3u,
                  "position", 24, 0);
    run(frame, n);
    CHECK(adip_busy(&ctx), "the reversible operation is running");

    now_ms += 5000u;
    (void)adip_poll(&ctx);

    /*
     * PROTOCOL.md 4.6 requirement 4 scopes the MUST to non-reversible
     * operations. A reversible one is still stopped, because continuing to
     * move a machine nobody is talking to is not defensible, but it does not
     * fault the appliance.
     */
    CHECK(!adip_busy(&ctx), "it was stopped");
    CHECK(safe_state_calls == 0, "but the safe state was not driven");
    CHECK(!adip_faulted(&ctx), "and the machine is not faulted");
}

static void test_fault_gate(void)
{
    printf("fault gate\n");
    reset_all();

    start_process(4u);
    now_ms += 5000u;
    (void)adip_poll(&ctx);
    CHECK(adip_faulted(&ctx), "faulted after the timeout");

    /* A perfectly valid invocation is refused while faulted. The machine may
     * hold a part-finished cycle that the engine knows nothing about. */
    sent_count = 0;
    start_process(5u);
    CHECK(sent_count >= 1, "the call was answered");
    CHECK(sent[0].outcome == ADIP_OUTCOME_REJECTED, "rejected");
    CHECK(sent[0].reason == ADIP_REASON_FAULT, "with FAULT");
    CHECK(!adip_busy(&ctx), "nothing started");

    /* Only the firmware can say the machine is fit again. */
    adip_clear_fault(&ctx);
    CHECK(!adip_faulted(&ctx), "the fault is cleared");

    sent_count = 0;
    now_ms += 30000u;  /* clear the rate limit too */
    start_process(6u);
    CHECK(adip_busy(&ctx), "work resumes once the firmware says so");
}

static void test_duration_overrun(void)
{
    printf("duration overrun\n");
    reset_all();

    start_process(7u);
    CHECK(adip_busy(&ctx), "running");

    /*
     * process.start declares duration_ms_max of 60000. Keep the link alive so
     * this is the duration firing and not the control timeout.
     */
    {
        int i;
        for (i = 0; i < 45; i++) {   /* 67.5 s, comfortably past 60 s */
            now_ms += 1500u;
            {   /* the engine is still heartbeating, for real */
                uint8_t hb[32];
                size_t hn = make_heartbeat(hb, sizeof(hb));
                run(hb, hn);
            }
            (void)adip_poll(&ctx);
        }
    }

    CHECK(!adip_busy(&ctx), "an operation past its declared maximum is stopped");
    CHECK(ctx.stat_duration_aborts == 1u, "counted as a duration abort");
    CHECK(ctx.stat_link_timeouts == 0u, "and not as a link timeout");
    CHECK(safe_state_calls == 1, "the safe state was driven");
}

/*
 * The failsafe must not fire during a long operation that the engine is still
 * attending to.
 *
 * This is the case that had no message. control_timeout_ms is 2000 and the
 * process tool may run far longer, so without a heartbeat every non-reversible
 * operation aborted itself two seconds in. Both halves were individually
 * correct and the pair could not complete a cycle.
 */
static void test_heartbeat_holds_the_failsafe_off(void)
{
    uint8_t hb[32];
    size_t n;
    int i;

    printf("heartbeat holds the failsafe off\n");
    reset_all();

    start_process(1u);
    CHECK(adip_busy(&ctx), "a long operation is running");

    n = make_heartbeat(hb, sizeof(hb));
    CHECK(n > 0, "a heartbeat frame is built");

    /* Ten seconds, five times the control timeout, with the engine sending
     * nothing but heartbeats because it has nothing else to say. */
    for (i = 0; i < 10; i++) {
        now_ms += 1000u;
        run(hb, n);
    }

    CHECK(adip_busy(&ctx), "the operation is still running after 10 s");
    CHECK(safe_state_calls == 0, "the safe state was never driven");
    CHECK(!adip_faulted(&ctx), "and the machine did not fault");
    CHECK(ctx.stat_link_timeouts == 0u, "no link timeout was counted");

    /* The reply carries the two flags, so the engine learns the machine is
     * busy without having to provoke a rejection. */
    CHECK(sent_count > 0, "the appliance replied to the heartbeat");

    /* Stop heartbeating and the failsafe returns. */
    now_ms += 3000u;
    (void)adip_poll(&ctx);
    CHECK(safe_state_calls == 1, "silence still fires the failsafe");
    CHECK(ctx.stat_link_timeouts == 1u, "counted as a link timeout");
}

/*
 * A heartbeat is answered whatever the machine is doing. An engine that only
 * heard back when the appliance was idle could not tell a busy machine from a
 * dead one, which is the question it is asking.
 */
static void test_heartbeat_is_answered_when_busy_and_when_faulted(void)
{
    uint8_t hb[32];
    size_t n;

    printf("heartbeat is answered in every state\n");
    reset_all();
    n = make_heartbeat(hb, sizeof(hb));

    run(hb, n);
    CHECK(ctx.stat_malformed == 0u, "an idle machine answers");

    start_process(2u);
    run(hb, n);
    CHECK(adip_busy(&ctx), "still busy");
    CHECK(ctx.stat_malformed == 0u, "a busy machine answers too");
    CHECK(ctx.stat_rejected == 0u, "and does not reject it as BUSY");

    /* Drive it into fault, then ask again. */
    now_ms += 5000u;
    (void)adip_poll(&ctx);
    CHECK(adip_faulted(&ctx), "faulted");
    run(hb, n);
    CHECK(ctx.stat_malformed == 0u, "a faulted machine answers as well");
}

/* An outbound kind arriving inbound is an echo, not an instruction. */
static void test_outbound_kinds_are_refused_inbound(void)
{
    uint8_t frame[64];
    builder_t b;
    int n;
    uint32_t before;

    printf("outbound kinds refused inbound\n");
    reset_all();
    before = ctx.stat_malformed;

    b.pos = 0;
    b_head(&b, 5u, 1u);
    b_uint(&b, 0u); b_uint(&b, ADIP_KIND_SIGNALS);
    n = adip_frame_wrap(b.buf, b.pos, frame, sizeof(frame));
    run(frame, (size_t)n);

    CHECK(ctx.stat_malformed == before + 1u, "a signal frame inbound is refused");
    CHECK(!adip_busy(&ctx), "and nothing was started");
}

static void test_reply_deadline(void)
{
    uint8_t frame[128];
    size_t n;

    printf("reply deadline\n");
    reset_all();
    next_action = ADIP_ACTION_DONE;

    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 8u,
                  "mass", 90, -1);
    run(frame, n);
    CHECK(ctx.stat_worst_reply_ms == 0u, "a prompt handler costs nothing");
    CHECK(ctx.stat_deadline_misses == 0u, "and misses no deadline");

    /* A handler that blocks for longer than the protocol allows. This is the
     * single most likely mistake a firmware author will make, so it is
     * measured rather than assumed away. */
    reset_all();
    next_action = ADIP_ACTION_DONE;
    handler_delay_ms = 250u;
    n = make_call(frame, sizeof(frame), "adi.example.input.by_mass", 9u,
                  "mass", 90, -1);
    run(frame, n);
    CHECK(ctx.stat_worst_reply_ms >= 250u, "the block was measured");
    CHECK(ctx.stat_deadline_misses == 1u, "and reported as a deadline miss");
    CHECK(ADIP_REPLY_DEADLINE_MS == 200u, "the deadline is the protocol's");
}

static void test_noise_does_not_keep_the_link_alive(void)
{
    uint8_t junk[16];

    printf("noise is not liveness\n");
    reset_all();
    start_process(10u);

    /*
     * A stuck transmitter spraying bytes must not hold the failsafe off. Only
     * a frame that passes its CRC counts as the engine talking.
     */
    memset(junk, 0x5Au, sizeof(junk));
    {
        int i;
        for (i = 0; i < 4; i++) {
            now_ms += 700u;
            (void)adip_feed(&ctx, junk, sizeof(junk));
            while (adip_poll(&ctx) == ADIP_PENDING) { }
        }
    }

    CHECK(safe_state_calls == 1, "the failsafe fired despite the noise");
    CHECK(ctx.stat_link_timeouts == 1u, "counted as a link timeout");
}

static void test_timer_wrap(void)
{
    printf("failsafe across the timer wrap\n");
    reset_all();

    /* Start just before the 32 bit millisecond counter wraps. */
    now_ms = 0xFFFFFF00u;
    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);
    {
        uint8_t i;
        for (i = 0; i < adip_device_neutral.precondition_count; i++) {
            adip_set_precondition(&ctx, i, true);
        }
    }
    safe_state_calls = 0;
    start_process(11u);
    CHECK(adip_busy(&ctx), "running just before the wrap");

    /* 1000 ms later, having wrapped through zero. Inside the 2000 ms timeout. */
    now_ms = 0x000002E8u;
    (void)adip_poll(&ctx);
    CHECK(adip_busy(&ctx), "still running across the wrap");

    /* 3000 ms after the start. Past it. */
    now_ms = 0x00000AB8u;
    (void)adip_poll(&ctx);
    CHECK(!adip_busy(&ctx), "and times out correctly across the wrap");
    CHECK(safe_state_calls == 1, "the safe state was driven once");
}

int main(void)
{
    printf("libadip M5 tests\n\n");
    test_link_timeout();
    test_idle_link_loss_is_not_a_fault();
    test_reversible_is_stopped_but_does_not_fault();
    test_fault_gate();
    test_duration_overrun();
    test_heartbeat_holds_the_failsafe_off();
    test_heartbeat_is_answered_when_busy_and_when_faulted();
    test_outbound_kinds_are_refused_inbound();
    test_reply_deadline();
    test_noise_does_not_keep_the_link_alive();
    test_timer_wrap();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
