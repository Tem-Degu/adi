/*
 * adip_runtime.c - Feed, poll, dispatch, reply.
 *
 * The order of operations in dispatch is deliberate and worth reading before
 * changing anything here.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip.h"

#include <string.h>

/* ------------------------------------------------------------------ replies */

static void send_response(adip_ctx_t *ctx, uint32_t id, adip_outcome_t outcome,
                          adip_reason_t reason)
{
    uint8_t payload[ADIP_TX_SIZE];
    int n;
    int framed;

    n = adip_encode_response(id, outcome, reason, payload, sizeof(payload));
    if (n <= 0) {
        return;
    }

    framed = adip_frame_wrap(payload, (size_t)n, ctx->tx, sizeof(ctx->tx));
    if (framed <= 0) {
        return;
    }

    /*
     * A transport that cannot take the bytes is not an error we can do
     * anything about here. The engine's own control timeout will notice, and
     * blocking to retry would violate the non-blocking contract.
     */
    (void)adip_port_write(ctx->tx, (size_t)framed);
}

/*
 * Answer a heartbeat.
 *
 * Deliberately unconditional: it is refused by nothing, not by BUSY and not by
 * FAULT. A heartbeat asks whether the appliance is alive, and an appliance
 * that is mid-operation or waiting for a person to clear a fault is alive and
 * is exactly when the engine most needs to hear so. The two flags are how it
 * finds out without having to provoke a rejection.
 */
static void send_heartbeat(adip_ctx_t *ctx)
{
    uint8_t payload[ADIP_TX_SIZE];
    int n;
    int framed;

    n = adip_encode_heartbeat(ctx->busy, ctx->faulted, payload, sizeof(payload));
    if (n <= 0) {
        return;
    }

    framed = adip_frame_wrap(payload, (size_t)n, ctx->tx, sizeof(ctx->tx));
    if (framed <= 0) {
        return;
    }

    (void)adip_port_write(ctx->tx, (size_t)framed);
}

static void reject(adip_ctx_t *ctx, uint32_t id, adip_reason_t reason)
{
    ctx->stat_rejected++;

    /*
     * An out-of-range or unit-mismatched value means the constraint layer in
     * the engine failed to clamp before sending. That is an engine defect
     * rather than an unlucky value, and it is counted separately so a
     * manufacturer can see it across a fleet. PROTOCOL.md 9.2.
     */
    if (reason == ADIP_REASON_OUT_OF_RANGE ||
        reason == ADIP_REASON_UNIT_MISMATCH) {
        ctx->stat_engine_defects++;
    }

    send_response(ctx, id, ADIP_OUTCOME_REJECTED, reason);
}

/* ------------------------------------------------------------------ signals */

void adip_set_signal_buffer(adip_ctx_t *ctx, adip_sample_t *buf, uint8_t count)
{
    if (ctx == NULL) {
        return;
    }
    ctx->samples = buf;
    ctx->sample_cap = (count > ADIP_MAX_BATCH) ? (uint8_t)ADIP_MAX_BATCH : count;
    ctx->sample_count = 0u;
}

static adip_status_t send_samples(adip_ctx_t *ctx, const adip_sample_t *s,
                                  uint8_t n)
{
    uint8_t payload[ADIP_TX_SIZE];
    int len;
    int framed;

    len = adip_encode_signals(s, n, payload, sizeof(payload));
    if (len <= 0) {
        return ADIP_ERR_NO_SPACE;
    }

    framed = adip_frame_wrap(payload, (size_t)len, ctx->tx, sizeof(ctx->tx));
    if (framed <= 0) {
        return ADIP_ERR_NO_SPACE;
    }

    (void)adip_port_write(ctx->tx, (size_t)framed);
    return ADIP_OK;
}

adip_status_t adip_publish(adip_ctx_t *ctx, uint16_t signal,
                           adip_num_t value, uint32_t t_ms)
{
    adip_sample_t one;

    if (ctx == NULL || ctx->dev == NULL) {
        return ADIP_ERR_ARG;
    }

    /* The declaration is the authority on both counts. */
    if (signal >= ctx->dev->signal_count) {
        return ADIP_ERR_ARG;
    }

    /*
     * A signal published in the wrong quantity is the mirror of an invocation
     * carrying one: the same defect, seen from the appliance's side. Refusing
     * it here keeps a firmware bug from reaching the engine as a plausible
     * looking number.
     */
    if (value.unit != ctx->dev->signals[signal].unit) {
        return ADIP_ERR_ARG;
    }

    if (ctx->samples == NULL || ctx->sample_cap == 0u) {
        /* No buffer: correct, just chattier. */
        one.signal = signal;
        one.value = value;
        one.t_ms = t_ms;
        return send_samples(ctx, &one, 1u);
    }

    ctx->samples[ctx->sample_count] = (adip_sample_t){ signal, value, t_ms };
    ctx->sample_count++;

    if (ctx->sample_count >= ctx->sample_cap) {
        return adip_flush_signals(ctx);
    }
    return ADIP_OK;
}

adip_status_t adip_flush_signals(adip_ctx_t *ctx)
{
    adip_status_t st;

    if (ctx == NULL) {
        return ADIP_ERR_ARG;
    }
    if (ctx->sample_count == 0u) {
        return ADIP_OK;
    }

    st = send_samples(ctx, ctx->samples, ctx->sample_count);

    /*
     * Cleared whether or not the write succeeded. Holding samples for a
     * transport that is not taking them would grow a backlog of measurements
     * that were true a while ago, and stale telemetry is worse than a gap.
     */
    ctx->sample_count = 0u;
    return st;
}

/* -------------------------------------------------------------------- setup */

adip_status_t adip_init(adip_ctx_t *ctx, const adip_device_t *dev,
                        uint8_t *rx, size_t rx_len, void *user)
{
    if (ctx == NULL || dev == NULL || rx == NULL) {
        return ADIP_ERR_ARG;
    }
    if (rx_len < ADIP_RX_MIN) {
        /* Too small a buffer would let a legal maximum frame wedge the
         * receiver permanently. Refuse at init rather than in the field. */
        return ADIP_ERR_NO_SPACE;
    }

    /*
     * Refuse a declaration this build cannot fully enforce.
     *
     * Silently tracking the first eight invariants and ignoring the ninth
     * would mean the machine reports a limit it does not enforce, which is
     * worse than not starting. Fail at init, loudly, in the lab.
     */
    if (dev->invariant_count > ADIP_MAX_INVARIANTS ||
        dev->rate_limit_count > ADIP_MAX_RATE_LIMITS ||
        dev->resource_count > ADIP_MAX_RESOURCES) {
        return ADIP_ERR_UNSUPPORTED;
    }
    {
        uint8_t i;
        for (i = 0; i < dev->rate_limit_count; i++) {
            if (dev->rate_limits[i].max_invocations > ADIP_MAX_RATE_HISTORY) {
                return ADIP_ERR_UNSUPPORTED;
            }
        }
    }

    memset(ctx, 0, sizeof(*ctx));
    ctx->dev = dev;
    ctx->rx = rx;
    ctx->rx_cap = rx_len;
    ctx->user = user;
    ctx->last_rx_ms = adip_port_millis();
    ctx->link_ms = ctx->last_rx_ms;

    /* Accumulators start empty, carrying the unit the invariant declares so
     * the first addition has something to align against. Resources start
     * full; firmware corrects them with adip_set_resource if that is wrong. */
    {
        uint8_t i;
        for (i = 0; i < dev->invariant_count; i++) {
            ctx->accumulated[i] = ADIP_NUM_ZERO;
            ctx->accumulated[i].unit = dev->invariants[i].max.unit;
        }
        for (i = 0; i < dev->resource_count; i++) {
            ctx->resource_left[i] = dev->resources[i].has_capacity
                                        ? dev->resources[i].capacity
                                        : ADIP_NUM_ZERO;
        }
    }

    /*
     * Both bitmaps start at zero. No precondition is satisfied until the
     * firmware says so, so nothing with a declared precondition is invocable
     * before the machine has reported its state. Failing closed at startup
     * matters more than being immediately useful.
     */
    return ADIP_OK;
}

void adip_set_interlock(adip_ctx_t *ctx, uint8_t interlock, bool open)
{
    if (ctx == NULL || interlock >= ADIP_MAX_INTERLOCKS) {
        return;
    }
    if (open) {
        ctx->open_interlocks |= (uint32_t)(1u << interlock);
    } else {
        ctx->open_interlocks &= ~(uint32_t)(1u << interlock);
    }
}

void adip_set_precondition(adip_ctx_t *ctx, uint8_t precondition, bool satisfied)
{
    if (ctx == NULL || precondition >= ADIP_MAX_PRECONDITIONS) {
        return;
    }
    if (satisfied) {
        ctx->satisfied_preconditions |= (uint32_t)(1u << precondition);
    } else {
        ctx->satisfied_preconditions &= ~(uint32_t)(1u << precondition);
    }
}

bool adip_busy(const adip_ctx_t *ctx)
{
    return (ctx != NULL) && ctx->busy;
}

bool adip_faulted(const adip_ctx_t *ctx)
{
    return (ctx != NULL) && ctx->faulted;
}

void adip_clear_fault(adip_ctx_t *ctx)
{
    if (ctx != NULL) {
        ctx->faulted = false;
    }
}

/* ------------------------------------------------------------------- input */

adip_status_t adip_feed(adip_ctx_t *ctx, const uint8_t *bytes, size_t len)
{
    if (ctx == NULL || bytes == NULL) {
        return ADIP_ERR_ARG;
    }
    if (len == 0) {
        return ADIP_OK;
    }
    if (ctx->rx_len + len > ctx->rx_cap) {
        return ADIP_ERR_NO_SPACE;
    }

    memcpy(ctx->rx + ctx->rx_len, bytes, len);
    ctx->rx_len += len;

    /*
     * Bytes arriving are recorded, but they deliberately do NOT refresh
     * `link_ms`. Only a frame that passes its CRC does that, in adip_poll.
     * A stuck transmitter spraying noise must not hold the failsafe off,
     * because noise is not the engine talking.
     */
    ctx->last_rx_ms = adip_port_millis();
    return ADIP_OK;
}

static void consume(adip_ctx_t *ctx, size_t n)
{
    if (n >= ctx->rx_len) {
        ctx->rx_len = 0;
        return;
    }
    memmove(ctx->rx, ctx->rx + n, ctx->rx_len - n);
    ctx->rx_len -= n;
}

/* ---------------------------------------------------------------- failsafe */

/*
 * Stop whatever is running and, where the operation cannot be undone, drive
 * the machine to its declared safe state.
 *
 * The response is sent even though the link is probably dead. It costs
 * nothing, and if the link recovered in the meantime the engine learns what
 * happened instead of waiting on an operation that ended.
 */
static void enter_failsafe(adip_ctx_t *ctx, adip_reason_t reason)
{
    const adip_tool_t *tool = NULL;
    bool irreversible = true;
    uint32_t id;

    if (!ctx->busy) {
        return;
    }

    if (ctx->active_tool < ctx->dev->tool_count) {
        tool = &ctx->dev->tools[ctx->active_tool];
        irreversible = (tool->flags & ADIP_TOOL_F_REVERSIBLE) == 0u;
    }

    id = ctx->active_id;
    ctx->busy = false;

    /*
     * PROTOCOL.md 4.6 requirement 4 scopes the MUST to non-reversible
     * operations, and that is where the safe state is driven. A reversible
     * operation is still stopped, because continuing to move a machine nobody
     * is talking to is not defensible either, but it does not fault the
     * appliance.
     */
    if (irreversible) {
        ctx->faulted = true;
        adip_port_safe_state();
    }

    send_response(ctx, id, ADIP_OUTCOME_ABORTED, reason);
}

/*
 * Time-driven work. Runs on every poll, including when no bytes have arrived,
 * which is why adip_poll must be called from the main loop rather than only
 * when the transport signals data.
 */
static void service_timers(adip_ctx_t *ctx)
{
    uint32_t now = adip_port_millis();

    if (!ctx->busy) {
        return;
    }

    /* The control link has gone quiet during an operation. */
    if (ctx->dev->control_timeout_ms > 0u &&
        (now - ctx->link_ms) > ctx->dev->control_timeout_ms) {
        ctx->stat_link_timeouts++;
        enter_failsafe(ctx, ADIP_REASON_FAULT);
        return;
    }

    /*
     * The operation has run past the duration its own declaration promised.
     * The firmware is expected to time its own mechanisms out, and this is the
     * second line of defence rather than the first.
     */
    if (ctx->active_tool < ctx->dev->tool_count) {
        uint32_t limit = ctx->dev->tools[ctx->active_tool].duration_ms_max;
        if (limit > 0u && (now - ctx->active_started_ms) > limit) {
            ctx->stat_duration_aborts++;
            enter_failsafe(ctx, ADIP_REASON_FAULT);
        }
    }
}

/* ---------------------------------------------------------------- dispatch */

static void dispatch(adip_ctx_t *ctx, const uint8_t *payload, size_t len)
{
    adip_invocation_t inv;
    adip_reason_t reason;
    adip_action_t action;
    int tool_idx;
    uint32_t now_ms;
    uint8_t kind;

    /*
     * 0. Route on kind, before decoding anything else. PROTOCOL.md 10.2.4.
     *
     * Decoding an invocation needs the declaration, and a heartbeat is not an
     * invocation. Peeking at key 0 first is what lets one transport carry
     * more than one kind of message.
     *
     * Liveness is already recorded by this point: adip_poll refreshes link_ms
     * for any frame that passes its CRC, which is what makes a heartbeat hold
     * the failsafe off during a long operation.
     */
    if (!adip_peek_kind(payload, len, &kind)) {
        ctx->stat_malformed++;
        return;
    }

    if (kind == ADIP_KIND_HEARTBEAT) {
        send_heartbeat(ctx);
        return;
    }

    /*
     * Kinds 2, 3 and 4 travel appliance to engine only. One arriving here is
     * either an echo or a confused peer, and acting on an echo of our own
     * response is worse than ignoring it.
     */
    if (kind != ADIP_KIND_INVOCATION) {
        ctx->stat_malformed++;
        return;
    }

    /*
     * Order matters, and this is the order.
     *
     * 1. Decode. A malformed frame has no invocation id, so there is nothing
     *    to answer and nothing useful to say.
     */
    reason = adip_decode_invocation(ctx->dev, payload, len, &inv);
    if (reason != ADIP_REASON_NONE) {
        if (inv.invocation_id != 0u) {
            reject(ctx, inv.invocation_id, reason);
        } else {
            ctx->stat_malformed++;
        }
        return;
    }

    /*
     * 2. Faulted, before anything else. A machine that hit its failsafe stays
     *    refusing until the firmware says it is fit, because only the firmware
     *    knows what physical state the interruption left behind.
     */
    if (ctx->faulted) {
        reject(ctx, inv.invocation_id, ADIP_REASON_FAULT);
        return;
    }

    /*
     * 3. Busy, before validation. If something is already moving, the answer
     *    is the same whatever the arguments say, and checking cheaply first
     *    avoids doing work whose result cannot matter.
     */
    if (ctx->busy) {
        reject(ctx, inv.invocation_id, ADIP_REASON_BUSY);
        return;
    }

    /* 4. Validate against the declaration. */
    reason = adip_validate(ctx->dev, &inv, ctx->open_interlocks,
                           ctx->satisfied_preconditions);
    if (reason != ADIP_REASON_NONE) {
        reject(ctx, inv.invocation_id, reason);
        return;
    }

    /*
     * 5. The stateful envelope, after the per-call checks and before the
     *    firmware sees anything. A rejection here must leave no trace, which
     *    is why checking and committing are separate calls.
     */
    now_ms = adip_port_millis();
    reason = adip_check_stateful(ctx, &inv, now_ms);
    if (reason != ADIP_REASON_NONE) {
        reject(ctx, inv.invocation_id, reason);
        return;
    }

    /*
     * 6. Hand it to the firmware.
     *
     * The handler is called before ACCEPTED is sent, so the firmware can still
     * refuse for a reason only it knows, such as a hardware fault. That is
     * only sound because handlers must not block: the contract is what keeps
     * the 200 ms reply deadline in PROTOCOL.md 9.2 reachable.
     */
    reason = ADIP_REASON_FAULT;
    action = adip_on_invoke(&inv, &reason, ctx->user);

    if (action == ADIP_ACTION_REJECTED) {
        reject(ctx, inv.invocation_id, reason);
        return;
    }

    /*
     * Committed on acceptance rather than on completion. The material is
     * consumed and the machine is occupied from the moment the action starts,
     * so a limit that only updated at completion would be wrong for the whole
     * duration of every operation.
     */
    /* The same instant the check used, so a rate limit cannot be tested
     * against one time and recorded against another. */
    adip_commit_stateful(ctx, &inv, now_ms);

    ctx->stat_accepted++;
    send_response(ctx, inv.invocation_id, ADIP_OUTCOME_ACCEPTED,
                  ADIP_REASON_NONE);

    if (action == ADIP_ACTION_DONE) {
        send_response(ctx, inv.invocation_id, ADIP_OUTCOME_COMPLETED,
                      ADIP_REASON_NONE);
        return;
    }

    /* ADIP_ACTION_RUNNING. Track it so adip_complete can match, and so M5 can
     * time it out. */
    tool_idx = adip_tool_index(ctx->dev, inv.tool);
    ctx->busy = true;
    ctx->active_id = inv.invocation_id;
    ctx->active_tool = (tool_idx >= 0) ? (uint16_t)tool_idx : 0u;
    ctx->active_started_ms = adip_port_millis();
}

adip_status_t adip_poll(adip_ctx_t *ctx)
{
    const uint8_t *payload = NULL;
    size_t payload_len = 0;
    size_t used = 0;
    adip_status_t st;

    if (ctx == NULL) {
        return ADIP_ERR_ARG;
    }

    /* Timers first, so a link that dies while the buffer is empty is still
     * noticed. This is why poll belongs in the main loop rather than only on
     * a transport interrupt. */
    service_timers(ctx);

    if (ctx->rx_len == 0) {
        return ADIP_OK;
    }

    st = adip_frame_find(ctx->rx, ctx->rx_len, &payload, &payload_len, &used);

    if (st == ADIP_OK) {
        uint32_t began = adip_port_millis();
        uint32_t took;

        /* A frame that passed its CRC is the engine talking. */
        ctx->link_ms = began;

        dispatch(ctx, payload, payload_len);
        consume(ctx, used);

        took = adip_port_millis() - began;
        if (took > ctx->stat_worst_reply_ms) {
            ctx->stat_worst_reply_ms = took;
        }
        if (took > ADIP_REPLY_DEADLINE_MS) {
            /* Almost always a handler that blocked. See the contract in
             * adip.h: handlers must return promptly and finish through
             * adip_complete(). */
            ctx->stat_deadline_misses++;
        }
        return ADIP_PENDING;
    }

    if (st == ADIP_ERR_MALFORMED) {
        ctx->stat_malformed++;
        consume(ctx, (used > 0) ? used : 1u);
        return ADIP_PENDING;
    }

    /* Pending. Either more bytes are needed, or leading noise was identified
     * and can be dropped. */
    if (used > 0) {
        consume(ctx, used);
        return ADIP_PENDING;
    }

    /*
     * Nothing consumable and the buffer is full. Without this the receiver
     * would wedge forever on a stream that never contains a valid frame.
     * Discard and resynchronise, counting it so the fault is visible.
     */
    if (ctx->rx_len == ctx->rx_cap) {
        ctx->stat_malformed++;
        ctx->rx_len = 0;
        return ADIP_PENDING;
    }

    return ADIP_OK;
}

adip_status_t adip_complete(adip_ctx_t *ctx, uint32_t invocation_id,
                            adip_outcome_t outcome, adip_reason_t reason)
{
    if (ctx == NULL) {
        return ADIP_ERR_ARG;
    }

    /*
     * Completing something that is not in flight is a firmware bug. Report it
     * rather than emitting a response the engine cannot match to anything.
     */
    if (!ctx->busy || ctx->active_id != invocation_id) {
        return ADIP_ERR_STATE;
    }
    if (outcome != ADIP_OUTCOME_COMPLETED && outcome != ADIP_OUTCOME_ABORTED) {
        return ADIP_ERR_ARG;
    }

    ctx->busy = false;
    send_response(ctx, invocation_id, outcome, reason);
    return ADIP_OK;
}
