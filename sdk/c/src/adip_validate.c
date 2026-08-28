/*
 * adip_validate.c - Check a decoded invocation against the declaration.
 *
 * Everything here is expressible without history. Invariants, rate limits and
 * resources need accumulated state and arrive in M4.
 *
 * The rule that governs the whole file: out-of-range values are REJECTED, not
 * clamped. An out-of-range value means the engine is defective. Correcting it
 * quietly would hide that fault for the life of the product, and it is how a
 * manufacturer detects a misbehaving engine in the field. See PROTOCOL.md 4.6
 * requirement 2.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_wire.h"

#include <string.h>

static bool enum_value_allowed(const adip_param_t *param,
                               const char *text, size_t len)
{
    uint8_t i;

    if (param->enum_values == NULL) {
        return false;
    }

    for (i = 0; i < param->enum_count; i++) {
        const char *candidate = param->enum_values[i];
        if (strlen(candidate) == len && memcmp(candidate, text, len) == 0) {
            return true;
        }
    }

    return false;
}

/*
 * Hard limits apply to a parameter name regardless of which tool carries it,
 * so a limit on "temperature" constrains every tool with a temperature.
 */
static adip_reason_t check_hard_limits(const adip_device_t *dev,
                                       const char *name, adip_num_t value)
{
    uint8_t i;

    for (i = 0; i < dev->limit_count; i++) {
        const adip_hard_limit_t *lim = &dev->limits[i];

        if (strcmp(lim->parameter, name) != 0) {
            continue;
        }

        if (lim->has_min != 0u) {
            adip_cmp_t c = adip_num_cmp(value, lim->hard_min);
            if (c == ADIP_CMP_INCOMPARABLE) {
                return ADIP_REASON_UNIT_MISMATCH;
            }
            if (c == ADIP_CMP_LT) {
                return ADIP_REASON_ENVELOPE_VIOLATION;
            }
        }

        if (lim->has_max != 0u) {
            adip_cmp_t c = adip_num_cmp(value, lim->hard_max);
            if (c == ADIP_CMP_INCOMPARABLE) {
                return ADIP_REASON_UNIT_MISMATCH;
            }
            if (c == ADIP_CMP_GT) {
                return ADIP_REASON_ENVELOPE_VIOLATION;
            }
        }
    }

    return ADIP_REASON_NONE;
}

adip_reason_t adip_validate(const adip_device_t *dev,
                            const adip_invocation_t *inv,
                            uint32_t open_interlocks,
                            uint32_t satisfied_preconditions)
{
    const adip_tool_t *tool;
    uint8_t i;
    uint8_t j;
    int tool_idx;
    uint32_t seen = 0;

    if (dev == NULL || inv == NULL || inv->tool == NULL) {
        return ADIP_REASON_MALFORMED;
    }

    tool = inv->tool;

    /* The engine may only invoke what the declaration lets it invoke. A tool
     * reserved to the operator is not reachable from the wire. */
    if ((tool->flags & ADIP_TOOL_F_AUTH_ENGINE) == 0u) {
        return ADIP_REASON_NOT_AUTHORISED;
    }

    /* Interlocks first. An open interlock is a physical fact and outranks
     * anything about the arguments. */
    tool_idx = adip_tool_index(dev, tool);
    if (tool_idx < 0) {
        return ADIP_REASON_UNKNOWN_TOOL;
    }
    for (i = 0; i < dev->interlock_count; i++) {
        if ((open_interlocks & (1u << i)) == 0u) {
            continue;
        }
        if (dev->interlocks[i].blocks_mask == ADIP_BLOCKS_ALL ||
            (dev->interlocks[i].blocks_mask & (1u << (unsigned)tool_idx)) != 0u) {
            return ADIP_REASON_INTERLOCK;
        }
    }

    /* Every declared precondition must be satisfied. */
    if ((tool->precondition_mask & satisfied_preconditions) !=
        tool->precondition_mask) {
        return ADIP_REASON_PRECONDITION_FAILED;
    }

    /* Arguments. */
    for (i = 0; i < inv->arg_count; i++) {
        const adip_arg_t *arg = &inv->args[i];
        const adip_param_t *param;

        if (arg->index >= tool->param_count) {
            return ADIP_REASON_MALFORMED;
        }
        param = &tool->params[arg->index];

        /* A parameter supplied twice is ambiguous, so it is refused rather
         * than resolved by last-one-wins. */
        if ((seen & (1u << arg->index)) != 0u) {
            return ADIP_REASON_MALFORMED;
        }
        seen |= (1u << arg->index);

        if (param->kind == ADIP_PARAM_ENUM) {
            if (arg->is_text == 0u) {
                return ADIP_REASON_MALFORMED;
            }
            if (!enum_value_allowed(param, arg->text, arg->text_len)) {
                return ADIP_REASON_OUT_OF_RANGE;
            }
            continue;
        }

        if (arg->is_text != 0u) {
            return ADIP_REASON_MALFORMED;
        }

        /* The declaration fixes the unit. A mismatch means the engine sent
         * grams where millilitres were declared, which is a defect rather than
         * a large value, and gets its own reason code. */
        if (arg->num.unit != param->min.unit) {
            return ADIP_REASON_UNIT_MISMATCH;
        }

        if (!adip_num_in_range(arg->num, param->min, param->max)) {
            return ADIP_REASON_OUT_OF_RANGE;
        }

        /* In range is not the same as reachable. A grinder with 1-step
         * resolution cannot go to 24.5. */
        if (!adip_num_on_resolution(arg->num, param->min, param->resolution)) {
            return ADIP_REASON_OUT_OF_RANGE;
        }

        {
            adip_reason_t r = check_hard_limits(dev, param->name, arg->num);
            if (r != ADIP_REASON_NONE) {
                return r;
            }
        }
    }

    /* Every non-optional parameter must be present. A default does not excuse
     * absence on the wire: the engine decides values, and a missing required
     * argument means it did not. */
    for (j = 0; j < tool->param_count; j++) {
        if ((tool->params[j].flags & ADIP_PARAM_F_OPTIONAL) != 0u) {
            continue;
        }
        if ((seen & (1u << j)) == 0u) {
            return ADIP_REASON_PRECONDITION_FAILED;
        }
    }

    return ADIP_REASON_NONE;
}
