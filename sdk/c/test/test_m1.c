/*
 * test_m1.c - Host tests for M1: scaled integers and generated tables.
 *
 * Deliberately freestanding-adjacent: the library under test uses no stdio and
 * no allocation. This harness uses printf only to report, and never passes
 * anything from stdio into the library.
 *
 * Build and run: see sdk/c/Makefile, target `test`.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdio.h>
#include <string.h>

#include "adip_generated_ex900.h"
#include "adip_generated_neutral.h"
#include "adip_tables.h"
#include "adip_types.h"

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

/* ------------------------------------------------------------------ numbers */

static void test_numbers(void)
{
    printf("scaled integers\n");

    /* Same value, different scales, must compare equal. This is the whole
     * reason for normalising rather than comparing mantissas. */
    CHECK(adip_num_cmp(ADIP_NUM(365, -1, ADIP_UNIT_G),
                       ADIP_NUM(3650, -2, ADIP_UNIT_G)) == ADIP_CMP_EQ,
          "36.5 == 36.50");

    CHECK(adip_num_cmp(ADIP_NUM(1, 0, ADIP_UNIT_G),
                       ADIP_NUM(2, 0, ADIP_UNIT_G)) == ADIP_CMP_LT,
          "1 < 2");

    CHECK(adip_num_cmp(ADIP_NUM(-5, 0, ADIP_UNIT_C),
                       ADIP_NUM(-50, -1, ADIP_UNIT_C)) == ADIP_CMP_EQ,
          "-5 == -5.0");

    /* A unit mismatch is an engine defect, not a large value. It must be
     * reported as incomparable so the caller rejects. */
    CHECK(adip_num_cmp(ADIP_NUM(10, 0, ADIP_UNIT_G),
                       ADIP_NUM(10, 0, ADIP_UNIT_ML)) == ADIP_CMP_INCOMPARABLE,
          "grams and millilitres do not compare");

    /* Fails closed: an unmatched unit is out of range, never in it. */
    CHECK(!adip_num_in_range(ADIP_NUM(10, 0, ADIP_UNIT_ML),
                             ADIP_NUM(0, 0, ADIP_UNIT_G),
                             ADIP_NUM(100, 0, ADIP_UNIT_G)),
          "wrong unit is not in range");

    CHECK(adip_num_in_range(ADIP_NUM(60, -1, ADIP_UNIT_G),
                            ADIP_NUM(60, -1, ADIP_UNIT_G),
                            ADIP_NUM(140, -1, ADIP_UNIT_G)),
          "range is inclusive at the bottom");

    CHECK(adip_num_in_range(ADIP_NUM(140, -1, ADIP_UNIT_G),
                            ADIP_NUM(60, -1, ADIP_UNIT_G),
                            ADIP_NUM(140, -1, ADIP_UNIT_G)),
          "range is inclusive at the top");

    CHECK(!adip_num_in_range(ADIP_NUM(141, -1, ADIP_UNIT_G),
                             ADIP_NUM(60, -1, ADIP_UNIT_G),
                             ADIP_NUM(140, -1, ADIP_UNIT_G)),
          "14.1 is outside 6.0 to 14.0");

    /* Resolution: a value can be in range and still unreachable by the
     * mechanism. 6.05 is not on a 0.1 grid starting at 6.0. */
    CHECK(adip_num_on_resolution(ADIP_NUM(61, -1, ADIP_UNIT_G),
                                 ADIP_NUM(60, -1, ADIP_UNIT_G),
                                 ADIP_NUM(1, -1, ADIP_UNIT_G)),
          "6.1 is on a 0.1 grid from 6.0");

    CHECK(!adip_num_on_resolution(ADIP_NUM(605, -2, ADIP_UNIT_G),
                                  ADIP_NUM(60, -1, ADIP_UNIT_G),
                                  ADIP_NUM(1, -1, ADIP_UNIT_G)),
          "6.05 is not on a 0.1 grid from 6.0");

    CHECK(adip_num_on_resolution(ADIP_NUM(12345, -3, ADIP_UNIT_G),
                                 ADIP_NUM(0, 0, ADIP_UNIT_G),
                                 ADIP_NUM(0, 0, ADIP_UNIT_G)),
          "zero resolution means unconstrained");

    /* Overflow must be reported, never wrapped. */
    {
        int64_t a;
        int64_t b;
        CHECK(!adip_num_align(ADIP_NUM(2000000000, 10, ADIP_UNIT_G),
                              ADIP_NUM(1, -8, ADIP_UNIT_G), &a, &b),
              "an overflowing alignment is refused");
    }
}

/* ------------------------------------------------------------------- tables */

static void test_tables(void)
{
    const adip_device_t *dev = &adip_device_neutral;
    const adip_tool_t *tool;
    int idx;

    printf("generated tables (neutral fixture)\n");

    CHECK(dev->tool_count == ADIP_NEUTRAL_TOOL_COUNT, "tool count matches the header");
    CHECK(dev->conformance_level == ADIP_CONFORMANCE_AUTONOMOUS,
          "conformance level survived generation");
    CHECK(strcmp(dev->domain_profile, "example/v1") == 0, "domain profile");
    CHECK(dev->has_stateful_envelope == 1, "stateful envelope detected");
    CHECK(dev->control_timeout_ms == 2000, "control timeout");
    CHECK(dev->failsafe.max_time_to_safe_ms == 500, "failsafe timing");

    tool = adip_find_tool(dev, "adi.example.input.by_mass");
    CHECK(tool != NULL, "a declared tool is found");

    if (tool != NULL) {
        CHECK(tool->param_count == 1, "one parameter");
        idx = adip_param_index(tool, "mass");
        CHECK(idx == 0, "parameter located by name");

        if (idx >= 0) {
            const adip_param_t *p = &tool->params[idx];

            /* 6.0 to 14.0 g in the declaration, exactly. */
            CHECK(p->kind == ADIP_PARAM_NUMBER, "kind");
            CHECK(p->min.unit == ADIP_UNIT_G, "unit survived");
            CHECK(adip_num_cmp(p->min, ADIP_NUM(60, -1, ADIP_UNIT_G)) == ADIP_CMP_EQ,
                  "min is exactly 6.0");
            CHECK(adip_num_cmp(p->max, ADIP_NUM(140, -1, ADIP_UNIT_G)) == ADIP_CMP_EQ,
                  "max is exactly 14.0");
            CHECK(adip_num_cmp(p->resolution, ADIP_NUM(1, -1, ADIP_UNIT_G)) == ADIP_CMP_EQ,
                  "resolution is exactly 0.1");

            /* The range check the runtime will perform in M2. */
            CHECK(adip_num_in_range(ADIP_NUM(90, -1, ADIP_UNIT_G), p->min, p->max),
                  "9.0 g is accepted");
            CHECK(!adip_num_in_range(ADIP_NUM(400, -1, ADIP_UNIT_G), p->min, p->max),
                  "40.0 g is refused");
        }
    }

    /* An undeclared tool must not resolve. There is no fallback. */
    CHECK(adip_find_tool(dev, "x.nobody.invented.this") == NULL,
          "an undeclared tool is not found");

    /* Direction that carries domain meaning must survive as declared. */
    tool = adip_find_tool(dev, "adi.example.actuator.set_position");
    CHECK(tool != NULL, "actuator tool present");
    if (tool != NULL && tool->param_count > 0) {
        CHECK(tool->params[0].monotonic == ADIP_MONOTONIC_PROFILE,
              "monotonic survived generation");
        CHECK((tool->flags & ADIP_TOOL_F_REVERSIBLE) != 0, "reversible flag");
    }

    /* Non-reversible and hazardous must not be silently dropped. */
    tool = adip_find_tool(dev, "adi.example.process.start");
    CHECK(tool != NULL, "process tool present");
    if (tool != NULL) {
        CHECK((tool->flags & ADIP_TOOL_F_REVERSIBLE) == 0, "not reversible");
        CHECK((tool->flags & ADIP_TOOL_F_ABORTABLE) != 0, "abortable");
        CHECK(tool->precondition_mask != 0, "preconditions were compiled");
        CHECK(tool->duration_ms_max == 60000, "max duration");
    }

    /* Stateful envelope. */
    CHECK(dev->invariant_count == 1, "one invariant");
    if (dev->invariant_count > 0) {
        const adip_invariant_t *inv = &dev->invariants[0];
        CHECK(strcmp(inv->id, "container_capacity") == 0, "invariant id");
        CHECK(inv->has_max == 1, "invariant has a maximum");
        CHECK(adip_num_cmp(inv->max, ADIP_NUM(320, 0, ADIP_UNIT_ML)) == ADIP_CMP_EQ,
              "capacity is exactly 320 ml");
        CHECK(inv->accum_count == 2, "two tools accumulate into it");
        CHECK(inv->reset_count == 1, "one tool resets it");

        /* The accumulator must point at a real tool and a real parameter. */
        if (inv->accum_count > 0) {
            uint16_t t = inv->accumulates[0].tool;
            uint8_t p = inv->accumulates[0].param;
            CHECK(t < dev->tool_count, "accumulator tool index in range");
            CHECK(p < dev->tools[t].param_count, "accumulator param index in range");
        }
    }

    CHECK(dev->rate_limit_count == 1, "one rate limit");
    if (dev->rate_limit_count > 0) {
        CHECK(dev->rate_limits[0].min_interval_ms == 25000,
              "25 s became 25000 ms");
        CHECK(dev->rate_limits[0].window_ms == 600000, "600 s window");
    }

    /* An interlock declared as ["*"] must block everything. */
    {
        uint8_t i;
        int found_wildcard = 0;
        for (i = 0; i < dev->interlock_count; i++) {
            if (dev->interlocks[i].blocks_mask == ADIP_BLOCKS_ALL) {
                found_wildcard = 1;
            }
        }
        CHECK(found_wildcard == 1, "the service door interlock blocks all tools");
    }
}

int main(void)
{
    printf("libadip M1 tests\n\n");
    test_numbers();
    test_tables();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
