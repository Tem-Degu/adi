/*
 * test_m4.c - The stateful envelope.
 *
 * The first group mirrors sdk/crates/adi-safety/tests/composition.rs case for
 * case, against the same neutral fixture. Two independent implementations
 * agreeing on the same inputs is much stronger evidence than either passing
 * its own tests, and it is the only way to be confident the engine and the
 * appliance will not disagree in the field about whether an action is legal.
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

#define CHECK_REASON(got, want, msg)                                           \
    do {                                                                       \
        checks++;                                                              \
        if ((got) != (want)) {                                                 \
            failures++;                                                        \
            printf("  FAIL %s:%d  %s (got %d, wanted %d)\n", __FILE__,         \
                   __LINE__, (msg), (int)(got), (int)(want));                  \
        }                                                                      \
    } while (0)

/* ------------------------------------------------------------------- port */

static uint32_t now_ms = 1000;
uint32_t adip_port_millis(void) { return now_ms; }
int adip_port_write(const uint8_t *b, size_t n) { (void)b; return (int)n; }
void adip_port_safe_state(void) { }

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    (void)inv; (void)reason; (void)user;
    return ADIP_ACTION_DONE;
}

/* --------------------------------------------------------------- fixtures */

static uint8_t rx[ADIP_RX_MIN];
static adip_ctx_t ctx;

/* Build an invocation directly, without going through the wire. These tests
 * are about the envelope, and framing is covered in M2. */
static adip_invocation_t call(const char *tool_id, const char *param,
                              adip_num_t value)
{
    adip_invocation_t inv;
    const adip_tool_t *tool;
    int idx;

    memset(&inv, 0, sizeof(inv));
    tool = adip_find_tool(&adip_device_neutral, tool_id);
    inv.tool = tool;
    inv.invocation_id = 1;

    if (tool != NULL && param != NULL) {
        idx = adip_param_index(tool, param);
        if (idx >= 0) {
            inv.args[0].index = (uint8_t)idx;
            inv.args[0].is_text = 0u;
            inv.args[0].num = value;
            inv.arg_count = 1;
        }
    }
    return inv;
}

static adip_invocation_t bare(const char *tool_id)
{
    return call(tool_id, NULL, ADIP_NUM_ZERO);
}

static void reset_ctx(void)
{
    now_ms = 1000;
    (void)adip_init(&ctx, &adip_device_neutral, rx, sizeof(rx), NULL);
}

static int invariant_index(const char *id)
{
    uint8_t i;
    for (i = 0; i < adip_device_neutral.invariant_count; i++) {
        if (strcmp(adip_device_neutral.invariants[i].id, id) == 0) {
            return (int)i;
        }
    }
    return -1;
}

/* ------------------------------------- agreement with adi-safety (Rust) */

/* composition.rs: individually_valid_calls_breach_the_container_invariant */
static void test_container_invariant(void)
{
    adip_invocation_t inv;
    int n;

    printf("invariants (mirrors composition.rs)\n");
    reset_ctx();

    /* Capacity is 320 ml. Each call is well inside the tool's own 0 to 300
     * range, so no per-call check can object to any of them. */
    inv = call("adi.example.additive.dispense", "volume",
               ADIP_NUM(150, 0, ADIP_UNIT_ML));

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "first 150 ml passes");
    adip_commit_stateful(&ctx, &inv, now_ms);

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "second 150 ml passes");
    adip_commit_stateful(&ctx, &inv, now_ms);

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms),
                 ADIP_REASON_INVARIANT_VIOLATION,
                 "450 into a 320 container is refused");

    n = invariant_index("container_capacity");
    CHECK(n >= 0, "the fixture declares container_capacity");
    if (n >= 0) {
        CHECK(adip_num_cmp(adip_accumulated(&ctx, (uint8_t)n),
                           ADIP_NUM(300, 0, ADIP_UNIT_ML)) == ADIP_CMP_EQ,
              "the accumulator holds exactly 300, not 450");
    }
}

/* composition.rs: emptying_the_container_resets_the_accumulator */
static void test_reset(void)
{
    adip_invocation_t fill;
    adip_invocation_t empty;
    int n;

    printf("accumulator reset\n");
    reset_ctx();
    n = invariant_index("container_capacity");

    fill = call("adi.example.additive.dispense", "volume",
                ADIP_NUM(300, 0, ADIP_UNIT_ML));
    CHECK_REASON(adip_check_stateful(&ctx, &fill, now_ms), ADIP_REASON_NONE,
                 "300 ml fits");
    adip_commit_stateful(&ctx, &fill, now_ms);
    CHECK(adip_num_cmp(adip_accumulated(&ctx, (uint8_t)n),
                       ADIP_NUM(300, 0, ADIP_UNIT_ML)) == ADIP_CMP_EQ,
          "accumulated 300");

    /* A second 300 would overflow. */
    CHECK_REASON(adip_check_stateful(&ctx, &fill, now_ms),
                 ADIP_REASON_INVARIANT_VIOLATION, "600 does not fit");

    empty = bare("adi.example.container.empty");
    adip_commit_stateful(&ctx, &empty, now_ms);
    CHECK(adip_accumulated(&ctx, (uint8_t)n).mantissa == 0,
          "an emptied container starts from zero");

    CHECK_REASON(adip_check_stateful(&ctx, &fill, now_ms), ADIP_REASON_NONE,
                 "there is room again");
}

/* composition.rs: running_again_too_soon_is_rate_limited */
static void test_rate_limit(void)
{
    adip_invocation_t inv;

    printf("rate limits\n");
    reset_ctx();

    inv = call("adi.example.process.start", "stop_at_volume",
               ADIP_NUM(360, -1, ADIP_UNIT_ML));

    now_ms = 0;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "first run is fine");
    adip_commit_stateful(&ctx, &inv, now_ms);

    /* The machine declares 25 s of recovery between runs. */
    now_ms = 10000;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_RATE_LIMITED,
                 "10 s is inside the declared recovery interval");

    now_ms = 24999;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_RATE_LIMITED,
                 "one millisecond short is still short");

    now_ms = 25000;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "exactly 25 s is enough");
}

/* --------------------------------------------------------------- beyond */

static void test_window_limit(void)
{
    adip_invocation_t inv;
    int i;

    printf("invocation window\n");
    reset_ctx();

    /* 12 invocations in a 600 s window, and 25 s minimum between them. Space
     * them 30 s apart so the interval never fires and only the count can. */
    inv = call("adi.example.process.start", "stop_at_volume",
               ADIP_NUM(360, -1, ADIP_UNIT_ML));

    now_ms = 0;
    {
        /* The process also fills the container, so it is emptied between runs.
         * Otherwise the container invariant fires at the ninth run and this
         * test would be measuring the wrong limit. */
        adip_invocation_t empty = bare("adi.example.container.empty");
        for (i = 0; i < 12; i++) {
            adip_reason_t r = adip_check_stateful(&ctx, &inv, now_ms);
            if (r != ADIP_REASON_NONE) {
                break;
            }
            adip_commit_stateful(&ctx, &inv, now_ms);
            adip_commit_stateful(&ctx, &empty, now_ms);
            now_ms += 30000u;
        }
    }
    CHECK(i == 12, "twelve runs fit inside the window");

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_RATE_LIMITED,
                 "the thirteenth is refused");

    /* Once the window has passed, the count is no longer against us. */
    now_ms += 600000u;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "the window slides");
}

static void test_resources(void)
{
    adip_invocation_t inv;

    printf("resources\n");
    reset_ctx();

    /* The reservoir holds 1800 ml and dispensing consumes it. */
    inv = call("adi.example.additive.dispense", "volume",
               ADIP_NUM(300, 0, ADIP_UNIT_ML));

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "the first draw is within the reservoir");

    /* Drain it, emptying the container between draws so the capacity
     * invariant is not what stops us. */
    {
        int i;
        adip_invocation_t empty = bare("adi.example.container.empty");
        for (i = 0; i < 6; i++) {
            if (adip_check_stateful(&ctx, &inv, now_ms) != ADIP_REASON_NONE) {
                break;
            }
            adip_commit_stateful(&ctx, &inv, now_ms);
            adip_commit_stateful(&ctx, &empty, now_ms);
        }
        CHECK(i == 6, "six draws of 300 empty an 1800 ml reservoir");
    }

    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms),
                 ADIP_REASON_RESOURCE_EXHAUSTED,
                 "the seventh draw finds the reservoir empty");

    /* Only the firmware knows when a tank was refilled by hand. */
    adip_set_resource(&ctx, 0, ADIP_NUM(1800, 0, ADIP_UNIT_ML));
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "refilling restores it");
}

static void test_rejection_leaves_no_trace(void)
{
    adip_invocation_t big;
    adip_invocation_t small;
    int n;

    printf("rejections leave no trace\n");
    reset_ctx();
    n = invariant_index("container_capacity");

    small = call("adi.example.additive.dispense", "volume",
                 ADIP_NUM(200, 0, ADIP_UNIT_ML));
    (void)adip_check_stateful(&ctx, &small, now_ms);
    adip_commit_stateful(&ctx, &small, now_ms);

    big = call("adi.example.additive.dispense", "volume",
               ADIP_NUM(300, 0, ADIP_UNIT_ML));
    CHECK_REASON(adip_check_stateful(&ctx, &big, now_ms),
                 ADIP_REASON_INVARIANT_VIOLATION, "500 would overflow");

    /*
     * The refused call must not have moved the accumulator. If checking and
     * committing were one operation, a rejection would poison the state and
     * every later call would be judged against a total that never happened.
     */
    CHECK(adip_num_cmp(adip_accumulated(&ctx, (uint8_t)n),
                       ADIP_NUM(200, 0, ADIP_UNIT_ML)) == ADIP_CMP_EQ,
          "a refused call left the accumulator alone");

    /* And a call that genuinely does fit still works afterwards. With 200
     * accumulated there is room for 120, so 100 fits and another 200 would
     * not. */
    {
        adip_invocation_t fits = call("adi.example.additive.dispense", "volume",
                                      ADIP_NUM(100, 0, ADIP_UNIT_ML));
        CHECK_REASON(adip_check_stateful(&ctx, &fits, now_ms), ADIP_REASON_NONE,
                     "a call that fits in the remaining 120 still works");
    }
}

static void test_timer_wraparound(void)
{
    adip_invocation_t inv;

    printf("timer wraparound\n");
    reset_ctx();

    /* A 32 bit millisecond counter wraps every 49 days. Unsigned subtraction
     * handles it, but only if every comparison is written that way, so it is
     * tested at the boundary rather than assumed. */
    inv = call("adi.example.process.start", "stop_at_volume",
               ADIP_NUM(360, -1, ADIP_UNIT_ML));

    now_ms = 0xFFFFF000u;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "a run just before the wrap");
    adip_commit_stateful(&ctx, &inv, now_ms);

    /* 8 seconds later, having wrapped through zero. Still inside 25 s. */
    now_ms = 0x00000E60u;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_RATE_LIMITED,
                 "the interval is still enforced across the wrap");

    /* 30 seconds after the first, again across the wrap. */
    now_ms = 0xFFFFF000u + 30000u;
    CHECK_REASON(adip_check_stateful(&ctx, &inv, now_ms), ADIP_REASON_NONE,
                 "and released correctly across the wrap");
}

static void test_init_refuses_what_it_cannot_enforce(void)
{
    adip_device_t oversized = adip_device_neutral;
    adip_ctx_t local;

    printf("init refuses what it cannot enforce\n");

    /*
     * A machine declaring more invariants than this build can track must not
     * start. Enforcing eight of nine would mean the declaration advertises a
     * limit the firmware silently ignores.
     */
    oversized.invariant_count = ADIP_MAX_INVARIANTS + 1;
    CHECK(adip_init(&local, &oversized, rx, sizeof(rx), NULL) ==
              ADIP_ERR_UNSUPPORTED,
          "too many invariants is refused at init");

    oversized = adip_device_neutral;
    oversized.resource_count = ADIP_MAX_RESOURCES + 1;
    CHECK(adip_init(&local, &oversized, rx, sizeof(rx), NULL) ==
              ADIP_ERR_UNSUPPORTED,
          "too many resources is refused at init");

    CHECK(adip_init(&local, &adip_device_neutral, rx, sizeof(rx), NULL) ==
              ADIP_OK, "the real fixture is accepted");
}

int main(void)
{
    printf("libadip M4 tests\n\n");
    test_container_invariant();
    test_reset();
    test_rate_limit();
    test_window_limit();
    test_resources();
    test_rejection_leaves_no_trace();
    test_timer_wraparound();
    test_init_refuses_what_it_cannot_enforce();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
