/*
 * adip_state.c - The stateful envelope.
 *
 * Constraints that exist only across a sequence. Three dispense actions of
 * 150 ml are each within range; their sum overflows a 320 ml container. No
 * per-call check can express that, which is why this file exists.
 *
 * This mirrors the logic in the Rust `adi-safety` crate deliberately, down to
 * the order of operations in commit. The two are tested against the same cases
 * and must agree, because two independent implementations agreeing is much
 * stronger evidence than either passing its own tests.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip.h"

#include <string.h>

/* Unsigned difference, correct across the 49 day wrap of a 32 bit millisecond
 * counter. Every time comparison in libadip goes through this. */
static uint32_t elapsed(uint32_t now, uint32_t then)
{
    return now - then;
}

/*
 * The value this invocation contributes to an accumulating quantity, if any.
 *
 * `entries` maps tool index to parameter index. A tool that does not appear
 * contributes nothing, which is not the same as contributing zero: it means
 * the caller should skip this invariant entirely.
 */
static bool contribution(const adip_invocation_t *inv, int tool_idx,
                         const adip_accum_t *entries, uint8_t count,
                         adip_num_t *out)
{
    uint8_t i;
    uint8_t a;

    if (entries == NULL || tool_idx < 0) {
        return false;
    }

    for (i = 0; i < count; i++) {
        if ((int)entries[i].tool != tool_idx) {
            continue;
        }
        for (a = 0; a < inv->arg_count; a++) {
            if (inv->args[a].index == entries[i].param &&
                inv->args[a].is_text == 0u) {
                *out = inv->args[a].num;
                return true;
            }
        }
        /* The tool contributes, but the invocation omitted that argument.
         * Nothing to add. */
        return false;
    }

    return false;
}

static bool rate_applies(const adip_rate_limit_t *rl, int tool_idx)
{
    uint8_t i;

    if (rl->all_tools != 0u) {
        return true;
    }
    if (rl->tools == NULL) {
        return false;
    }
    for (i = 0; i < rl->tool_count; i++) {
        if ((int)rl->tools[i] == tool_idx) {
            return true;
        }
    }
    return false;
}

void adip_set_resource(adip_ctx_t *ctx, uint8_t resource, adip_num_t remaining)
{
    if (ctx == NULL || resource >= ADIP_MAX_RESOURCES ||
        resource >= ctx->dev->resource_count) {
        return;
    }
    ctx->resource_left[resource] = remaining;
}

adip_num_t adip_accumulated(const adip_ctx_t *ctx, uint8_t invariant)
{
    if (ctx == NULL || invariant >= ADIP_MAX_INVARIANTS) {
        return ADIP_NUM_ZERO;
    }
    return ctx->accumulated[invariant];
}

adip_reason_t adip_check_stateful(const adip_ctx_t *ctx,
                                  const adip_invocation_t *inv,
                                  uint32_t now_ms)
{
    const adip_device_t *dev;
    int tool_idx;
    uint8_t i;
    const uint32_t now = now_ms;

    if (ctx == NULL || inv == NULL || inv->tool == NULL) {
        return ADIP_REASON_MALFORMED;
    }

    dev = ctx->dev;
    tool_idx = adip_tool_index(dev, inv->tool);
    if (tool_idx < 0) {
        return ADIP_REASON_UNKNOWN_TOOL;
    }

    /* ------------------------------------------------------- invariants */
    for (i = 0; i < dev->invariant_count && i < ADIP_MAX_INVARIANTS; i++) {
        const adip_invariant_t *inv_def = &dev->invariants[i];
        adip_num_t add;
        adip_num_t projected;

        if (!contribution(inv, tool_idx, inv_def->accumulates,
                          inv_def->accum_count, &add)) {
            continue;
        }

        if (!adip_num_add(ctx->accumulated[i], add, &projected)) {
            /*
             * The sum will not fit, or the units disagree. Either way the
             * limit can no longer be shown to hold, so the answer is no.
             * Saturating and carrying on would silently stop enforcing it.
             */
            return ADIP_REASON_INVARIANT_VIOLATION;
        }

        if (inv_def->has_max != 0u) {
            adip_cmp_t c = adip_num_cmp(projected, inv_def->max);
            /*
             * Incomparable is not "over the limit", and reporting it as one
             * sends a manufacturer to look at a full cup when the real fault
             * is a declaration adding grams to a millilitre total. The
             * distinct code is the whole point of having one.
             */
            if (c == ADIP_CMP_INCOMPARABLE) {
                return ADIP_REASON_UNIT_MISMATCH;
            }
            if (c == ADIP_CMP_GT) {
                return ADIP_REASON_INVARIANT_VIOLATION;
            }
        }
        if (inv_def->has_min != 0u) {
            adip_cmp_t c = adip_num_cmp(projected, inv_def->min);
            if (c == ADIP_CMP_INCOMPARABLE) {
                return ADIP_REASON_UNIT_MISMATCH;
            }
            if (c == ADIP_CMP_LT) {
                return ADIP_REASON_INVARIANT_VIOLATION;
            }
        }
    }

    /* ------------------------------------------------------ rate limits */
    for (i = 0; i < dev->rate_limit_count && i < ADIP_MAX_RATE_LIMITS; i++) {
        const adip_rate_limit_t *rl = &dev->rate_limits[i];
        uint8_t n;
        uint32_t recent = 0;

        if (!rate_applies(rl, tool_idx)) {
            continue;
        }

        /* Minimum interval since the most recent invocation. */
        if (rl->min_interval_ms > 0u && ctx->rate_used[i] > 0u) {
            uint8_t last = (uint8_t)((ctx->rate_head[i] + ADIP_MAX_RATE_HISTORY
                                      - 1u) % ADIP_MAX_RATE_HISTORY);
            if (elapsed(now, ctx->rate_history[i][last]) < rl->min_interval_ms) {
                return ADIP_REASON_RATE_LIMITED;
            }
        }

        /* Count within the window. */
        if (rl->max_invocations > 0u && rl->window_ms > 0u) {
            for (n = 0; n < ctx->rate_used[i]; n++) {
                uint8_t slot = (uint8_t)((ctx->rate_head[i] +
                                          ADIP_MAX_RATE_HISTORY - 1u - n) %
                                         ADIP_MAX_RATE_HISTORY);
                if (elapsed(now, ctx->rate_history[i][slot]) <= rl->window_ms) {
                    recent++;
                }
            }
            if (recent >= rl->max_invocations) {
                return ADIP_REASON_RATE_LIMITED;
            }
        }
    }

    /* -------------------------------------------------------- resources */
    for (i = 0; i < dev->resource_count && i < ADIP_MAX_RESOURCES; i++) {
        const adip_resource_t *res = &dev->resources[i];
        adip_num_t want;
        adip_cmp_t c;

        if (!contribution(inv, tool_idx, res->consumed_by, res->consume_count,
                          &want)) {
            continue;
        }

        c = adip_num_cmp(want, ctx->resource_left[i]);
        if (c == ADIP_CMP_INCOMPARABLE) {
            /* A reservoir in millilitres cannot be drawn down by a mass. That
             * is a declaration fault, not an empty tank. */
            return ADIP_REASON_UNIT_MISMATCH;
        }
        if (c == ADIP_CMP_GT) {
            return ADIP_REASON_RESOURCE_EXHAUSTED;
        }
    }

    return ADIP_REASON_NONE;
}

void adip_commit_stateful(adip_ctx_t *ctx, const adip_invocation_t *inv,
                          uint32_t now_ms)
{
    const adip_device_t *dev;
    int tool_idx;
    uint8_t i;
    uint8_t j;

    if (ctx == NULL || inv == NULL || inv->tool == NULL) {
        return;
    }

    dev = ctx->dev;
    tool_idx = adip_tool_index(dev, inv->tool);
    if (tool_idx < 0) {
        return;
    }

    /*
     * Accumulate first, then reset. A tool that both contributes to and resets
     * an invariant ends at zero. This ordering matches adi-safety exactly, and
     * it has to: the two implementations are tested against the same cases.
     */
    for (i = 0; i < dev->invariant_count && i < ADIP_MAX_INVARIANTS; i++) {
        const adip_invariant_t *inv_def = &dev->invariants[i];
        adip_num_t add;
        adip_num_t sum;

        if (contribution(inv, tool_idx, inv_def->accumulates,
                         inv_def->accum_count, &add)) {
            if (adip_num_add(ctx->accumulated[i], add, &sum)) {
                ctx->accumulated[i] = sum;
            }
        }

        for (j = 0; j < inv_def->reset_count; j++) {
            if ((int)inv_def->reset_on[j] == tool_idx) {
                ctx->accumulated[i] = ADIP_NUM_ZERO;
                ctx->accumulated[i].unit = inv_def->max.unit;
                break;
            }
        }
    }

    for (i = 0; i < dev->rate_limit_count && i < ADIP_MAX_RATE_LIMITS; i++) {
        if (!rate_applies(&dev->rate_limits[i], tool_idx)) {
            continue;
        }
        ctx->rate_history[i][ctx->rate_head[i]] = now_ms;
        ctx->rate_head[i] = (uint8_t)((ctx->rate_head[i] + 1u) %
                                      ADIP_MAX_RATE_HISTORY);
        if (ctx->rate_used[i] < ADIP_MAX_RATE_HISTORY) {
            ctx->rate_used[i]++;
        }
    }

    for (i = 0; i < dev->resource_count && i < ADIP_MAX_RESOURCES; i++) {
        const adip_resource_t *res = &dev->resources[i];
        adip_num_t used;
        adip_num_t left;

        if (!contribution(inv, tool_idx, res->consumed_by, res->consume_count,
                          &used)) {
            continue;
        }
        if (adip_num_add(ctx->resource_left[i], adip_num_neg(used), &left)) {
            ctx->resource_left[i] = left;
        }
    }
}
