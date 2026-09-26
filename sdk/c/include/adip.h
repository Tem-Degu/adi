/*
 * adip.h - The public interface a manufacturer's firmware uses.
 *
 * Everything a firmware engineer needs is here. adip_cbor.h and the internals
 * are not part of the contract.
 *
 * Two properties shape this API, both from sdk/c/DESIGN.md section 2:
 *
 *   The library owns no memory. The caller supplies the receive buffer at
 *   init, and nothing is allocated afterwards.
 *
 *   The library owns no loop. It never blocks and never waits. Firmware calls
 *   adip_poll() from whatever scheduler it already has.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#ifndef ADIP_H
#define ADIP_H

#include "adip_tables.h"
#include "adip_types.h"
#include "adip_wire.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Minimum receive buffer. One maximum frame plus its header and trailer, so a
 * legal frame can always be assembled without the buffer wedging.
 */
#define ADIP_RX_MIN (ADIP_MAX_FRAME + ADIP_FRAME_HEADER + ADIP_FRAME_TRAILER)

/*
 * The context carries its own transmit buffer rather than asking the caller
 * for a second one. Sized for a full signal batch, which is the largest thing
 * this library sends; a response needs well under fifty bytes.
 */
#define ADIP_TX_SIZE 256

/*
 * Samples per signal frame. PROTOCOL.md 10.1 asks that signal frames batch to
 * keep the notification rate under 50 Hz, so a 20 Hz mass signal becomes a
 * frame every few hundred milliseconds rather than twenty a second.
 */
#define ADIP_MAX_BATCH 8

/*
 * Capacity of the stateful envelope, held in the context.
 *
 * These are compile-time maxima rather than caller-sized arrays, because the
 * state is small and a second caller buffer is a second thing to get wrong.
 * adip_init() REFUSES a declaration that exceeds any of them, so a mismatch is
 * caught at startup rather than by silently enforcing fewer limits than the
 * machine declared.
 */
#define ADIP_MAX_INVARIANTS   8
#define ADIP_MAX_RATE_LIMITS  4
#define ADIP_MAX_RESOURCES    4

/* Timestamps retained per rate limit, for the max_invocations-in-window form.
 * A declaration asking for more than this is refused at init. */
#define ADIP_MAX_RATE_HISTORY 16

/* ------------------------------------------------------- the port layer */

/*
 * Three functions the manufacturer implements. That is the whole platform
 * dependency.
 */

/* A monotonic millisecond counter. Wrapping is expected and handled; the
 * library only ever takes unsigned differences. */
uint32_t adip_port_millis(void);

/* Send bytes to the engine. Returns the number written, or negative on error.
 * Must not block: a transport that cannot accept the bytes should drop them
 * and let the engine's timeout handle it. */
int adip_port_write(const uint8_t *buf, size_t len);

/*
 * Drive the machine to its declared safe state.
 *
 * Called when the control link goes quiet during a non-reversible operation.
 * This must reach the safe state within the time the declaration promises,
 * and it must not depend on the engine, the transport, or anything else being
 * healthy. See PROTOCOL.md 4.6 requirement 4.
 */
void adip_port_safe_state(void);

/* ---------------------------------------------------- the handler contract */

/*
 * Called when an invocation has passed every check and the machine should act.
 *
 * Return ADIP_ACTION_DONE for something instantaneous, ADIP_ACTION_RUNNING for
 * anything that takes time, or ADIP_ACTION_REJECTED with *reason set when the
 * firmware refuses for a reason only it can know, such as a hardware fault.
 *
 * HANDLERS MUST NOT BLOCK. A six second tray wipe cannot stall the appliance's
 * main loop. Start the movement, return ADIP_ACTION_RUNNING, and call
 * adip_complete() when it finishes or fails. The whole API is shaped to make
 * that the natural thing to write.
 *
 * `inv` and anything it points at are valid only for the duration of the call.
 * Text arguments point into the receive buffer and are not NUL terminated.
 * Copy anything you need to keep.
 */
adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason,
                             void *user);

/* ------------------------------------------------------------------ context */

typedef struct {
    const adip_device_t *dev;

    uint8_t *rx;
    size_t   rx_cap;
    size_t   rx_len;

    /* State the firmware maintains, as bitmaps indexed like the generated
     * tables. Both default to the safe reading: no preconditions satisfied,
     * so nothing is invocable until the firmware says otherwise. */
    uint32_t open_interlocks;
    uint32_t satisfied_preconditions;

    /* One operation in flight. A second arrival is refused with BUSY rather
     * than queued, because queuing would mean holding an invocation whose
     * preconditions may no longer hold when it runs. */
    bool     busy;
    uint32_t active_id;
    uint16_t active_tool;
    uint32_t active_started_ms;

    /*
     * Liveness. `link_ms` is the last VALID frame, not the last byte. Noise on
     * the line is not the engine talking, and treating it as such would hold
     * the failsafe off while the link was in fact dead.
     */
    uint32_t link_ms;
    uint32_t last_rx_ms;   /* last byte of any kind, for diagnostics */

    /*
     * Set when the failsafe has fired. New invocations are refused with FAULT
     * until the firmware calls adip_clear_fault().
     *
     * A machine that entered its safe state mid-operation may be in a physical
     * state the engine knows nothing about, such as a part-finished cycle still
     * loaded. Only the firmware can say when it is fit to continue, so resuming
     * is not automatic.
     */
    bool     faulted;

    /*
     * The stateful envelope. These are the constraints that exist only across
     * a sequence: individually valid actions composing into an invalid one.
     * See PROTOCOL.md 1.1 point 3.
     */
    adip_num_t accumulated[ADIP_MAX_INVARIANTS];
    adip_num_t resource_left[ADIP_MAX_RESOURCES];
    uint32_t   rate_history[ADIP_MAX_RATE_LIMITS][ADIP_MAX_RATE_HISTORY];
    uint8_t    rate_head[ADIP_MAX_RATE_LIMITS];   /* next slot to write */
    uint8_t    rate_used[ADIP_MAX_RATE_LIMITS];   /* entries in use */

    void    *user;

    /*
     * Pending signal samples. The caller owns the storage, as with rx, so a
     * device that publishes nothing pays no RAM for the ability.
     */
    adip_sample_t *samples;
    uint8_t        sample_cap;
    uint8_t        sample_count;

    /*
     * Counters. An out-of-range invocation means the engine is defective, and
     * PROTOCOL.md 9.2 says the appliance SHOULD log it. Counting is the
     * smallest honest form of that, and it is how a manufacturer notices a
     * misbehaving engine across a fleet.
     */
    uint32_t stat_accepted;
    uint32_t stat_rejected;
    uint32_t stat_malformed;
    uint32_t stat_engine_defects;
    uint32_t stat_link_timeouts;
    uint32_t stat_duration_aborts;

    /*
     * Worst observed time from a frame arriving to its response going out.
     * PROTOCOL.md 9.2 requires that to stay under 200 ms, and this is the
     * evidence for the conformance checklist rather than an assertion that it
     * holds. A handler that blocks will show up here.
     */
    uint32_t stat_worst_reply_ms;
    uint32_t stat_deadline_misses;

    uint8_t  tx[ADIP_TX_SIZE];
} adip_ctx_t;

/*
 * Set up. `rx` must be at least ADIP_RX_MIN bytes and must outlive the
 * context. `user` is passed back to the handler untouched.
 */
adip_status_t adip_init(adip_ctx_t *ctx, const adip_device_t *dev,
                        uint8_t *rx, size_t rx_len, void *user);

/*
 * Hand over bytes that arrived from the transport. Partial frames are fine.
 *
 * Returns ADIP_ERR_NO_SPACE if the buffer is full, which can only happen if
 * the caller feeds without polling.
 */
adip_status_t adip_feed(adip_ctx_t *ctx, const uint8_t *bytes, size_t len);

/*
 * Process at most one message. Non-blocking.
 *
 * Returns ADIP_PENDING when there may be more work, ADIP_OK when idle. Call it
 * in a loop until it returns ADIP_OK, or once per main loop iteration; both
 * work.
 */
adip_status_t adip_poll(adip_ctx_t *ctx);

/*
 * Report the outcome of an operation that returned ADIP_ACTION_RUNNING.
 *
 * Returns ADIP_ERR_STATE if `invocation_id` is not the operation in flight,
 * which catches a firmware bug rather than silently sending a response the
 * engine cannot match.
 */
adip_status_t adip_complete(adip_ctx_t *ctx, uint32_t invocation_id,
                            adip_outcome_t outcome, adip_reason_t reason);

/*
 * Give the library somewhere to accumulate samples. Optional: without it
 * adip_publish sends each measurement on its own, which is correct but chatty.
 *
 * `buf` must outlive the context. At most ADIP_MAX_BATCH samples are used.
 */
void adip_set_signal_buffer(adip_ctx_t *ctx, adip_sample_t *buf, uint8_t count);

/*
 * Report a measurement.
 *
 * The signal index is checked against the declaration, and so is the unit: a
 * value published in the wrong quantity is the same defect as an invocation
 * carrying one, seen from the other side, and is refused rather than sent.
 *
 * Returns ADIP_ERR_ARG for an unknown signal or a unit the declaration does
 * not name for it. Never blocks. Flushes by itself when the batch is full.
 */
adip_status_t adip_publish(adip_ctx_t *ctx, uint16_t signal,
                           adip_num_t value, uint32_t t_ms);

/*
 * Send whatever samples are pending.
 *
 * Firmware calls this at whatever rate it wants frames to leave, typically
 * once per control loop. A flush with nothing pending does nothing.
 */
adip_status_t adip_flush_signals(adip_ctx_t *ctx);

/* -------------------------------------------------------------------- events */

/*
 * Raise a declared event. PROTOCOL.md 4.5.
 *
 * `severity` is 0 warning, 1 error, 2 critical. The event index is checked
 * against the declaration; the code is the firmware's own string and is not
 * interpreted here.
 *
 * Section 4.5 requires a fault event before or with entry to a safe state.
 * The library does not raise it, because only the firmware knows what failed
 * and how badly. Raise it from adip_port_safe_state, which is called at
 * exactly that moment and is the one place that knows.
 */
adip_status_t adip_emit_event(adip_ctx_t *ctx, uint16_t event,
                              const char *code, uint8_t severity);

/* Firmware tells the library what the machine's state is. */
void adip_set_interlock(adip_ctx_t *ctx, uint8_t interlock, bool open);
void adip_set_precondition(adip_ctx_t *ctx, uint8_t precondition, bool satisfied);

/*
 * Report an actual resource level, for example after a tank is refilled.
 *
 * The library decrements as tools consume, but only the firmware knows when
 * something was replenished by hand.
 */
void adip_set_resource(adip_ctx_t *ctx, uint8_t resource, adip_num_t remaining);

/* Current value of an accumulator, for firmware that wants to display it. */
adip_num_t adip_accumulated(const adip_ctx_t *ctx, uint8_t invariant);

/*
 * Check the stateful envelope for an invocation that has already passed
 * adip_validate. Returns ADIP_REASON_NONE when the sequence stays legal.
 *
 * Separate from adip_validate because it needs history, and separate from
 * commit because a rejected invocation must leave no trace.
 *
 * `now_ms` is passed in for the same reason as in adip_commit_stateful, and
 * because a check and the commit that follows it must reason about the same
 * instant. Reading the clock separately in each let a rate limit be tested
 * against one time and recorded against another. It also kept the whole state
 * module from linking without a platform.
 */
adip_reason_t adip_check_stateful(const adip_ctx_t *ctx,
                                  const adip_invocation_t *inv,
                                  uint32_t now_ms);

/*
 * Record an accepted invocation against the stateful envelope.
 *
 * Called only after every check has passed. `now_ms` is passed in rather than
 * read from the clock so a caller can replay a sequence deterministically.
 */
void adip_commit_stateful(adip_ctx_t *ctx, const adip_invocation_t *inv,
                          uint32_t now_ms);

/* True while an operation is in flight. */
bool adip_busy(const adip_ctx_t *ctx);

/* True once the failsafe has fired and before the firmware has cleared it. */
bool adip_faulted(const adip_ctx_t *ctx);

/*
 * Declare the machine fit to work again after a failsafe.
 *
 * Only the firmware can know this. Call it once the machine has been returned
 * to a known state, which may mean discarding material or waiting for a person.
 */
void adip_clear_fault(adip_ctx_t *ctx);

/*
 * The reply deadline in PROTOCOL.md 9.2. Exposed so a conformance run can
 * assert against the same number the library measures.
 */
#define ADIP_REPLY_DEADLINE_MS 200u

#ifdef __cplusplus
}
#endif

#endif /* ADIP_H */
