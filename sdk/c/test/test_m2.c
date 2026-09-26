/*
 * test_m2.c - Host tests for framing, decoding and validation.
 *
 * The happy path is the cheap part. Nearly all of these test a rejection,
 * because the value of this layer is refusing things correctly, with the right
 * reason code, rather than accepting things.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdio.h>
#include <string.h>

#include "adip_cbor.h"
#include "adip_generated_neutral.h"
#include "adip_wire.h"

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
            printf("  FAIL %s:%d  %s (got reason %d, wanted %d)\n",            \
                   __FILE__, __LINE__, (msg), (int)(got), (int)(want));        \
        }                                                                      \
    } while (0)

/* ------------------------------------------------------------ CBOR builder */

/* A tiny encoder used only to build test inputs. */
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

static void b_nint(builder_t *b, int64_t v)
{
    /* CBOR negative integers encode -1 - n. */
    b_head(b, 1u, (uint64_t)(-1 - v));
}

static void b_int(builder_t *b, int64_t v)
{
    if (v < 0) {
        b_nint(b, v);
    } else {
        b_uint(b, (uint64_t)v);
    }
}

static void b_text(builder_t *b, const char *s)
{
    size_t n = strlen(s);
    b_head(b, 3u, n);
    memcpy(b->buf + b->pos, s, n);
    b->pos += n;
}

static void b_map(builder_t *b, uint64_t n) { b_head(b, 5u, n); }

/* Tag 4: a decimal fraction, [exponent, mantissa]. This is the shape that
 * matches adip_num_t exactly, which is why CBOR was chosen. */
static void b_decimal(builder_t *b, int32_t mantissa, int8_t exponent)
{
    b_head(b, 6u, 4u);
    b_head(b, 4u, 2u);
    b_int(b, exponent);
    b_int(b, mantissa);
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


/* Build: { 1: tool, 2: id, 3: { name: <number> } } */
static void build_num_call(builder_t *b, const char *tool, uint32_t id,
                           const char *arg, int32_t mant, int8_t exp)
{
    b->pos = 0;
    b_map(b, 4u);
    b_uint(b, 0u); b_uint(b, ADIP_KIND_INVOCATION);
    b_uint(b, 1u); b_text(b, tool);
    b_uint(b, 2u); b_uint(b, id);
    b_uint(b, 3u);
    b_map(b, 1u);
    b_text(b, arg);
    b_quantity(b, unit_for(tool, arg), mant, exp);
}

/* ------------------------------------------------------------------ framing */

static void test_framing(void)
{
    uint8_t frame[64];
    const uint8_t payload[] = { 0xa1, 0x01, 0x02 };
    const uint8_t *found;
    size_t found_len;
    size_t consumed;
    int n;

    printf("framing\n");

    n = adip_frame_wrap(payload, sizeof(payload), frame, sizeof(frame));
    CHECK(n == (int)(sizeof(payload) + 5u), "wrapped length");
    CHECK(frame[0] == ADIP_FRAME_SYNC, "sync byte");

    CHECK(adip_frame_find(frame, (size_t)n, &found, &found_len, &consumed) ==
              ADIP_OK,
          "a well formed frame is found");
    CHECK(found_len == sizeof(payload), "payload length");
    CHECK(memcmp(found, payload, sizeof(payload)) == 0, "payload contents");
    CHECK(consumed == (size_t)n, "whole frame consumed");

    /* Truncation must be pending, never an error. A stream arrives in pieces. */
    CHECK(adip_frame_find(frame, 2, &found, &found_len, &consumed) ==
              ADIP_PENDING,
          "a partial header is pending");
    CHECK(adip_frame_find(frame, (size_t)n - 1u, &found, &found_len,
                          &consumed) == ADIP_PENDING,
          "a partial frame is pending");

    /* A corrupted payload must fail the CRC rather than be delivered. */
    frame[4] ^= 0xffu;
    CHECK(adip_frame_find(frame, (size_t)n, &found, &found_len, &consumed) ==
              ADIP_ERR_MALFORMED,
          "a corrupted payload fails the CRC");
    CHECK(consumed == 1u, "a bad frame still lets the caller make progress");
    frame[4] ^= 0xffu;

    /* Leading noise: the caller is told how much to discard so it can
     * resynchronise rather than stall. */
    {
        uint8_t noisy[80];
        noisy[0] = 0x00u;
        noisy[1] = 0x11u;
        memcpy(noisy + 2, frame, (size_t)n);
        CHECK(adip_frame_find(noisy, (size_t)n + 2u, &found, &found_len,
                              &consumed) == ADIP_PENDING,
              "noise before the sync byte is skipped");
        CHECK(consumed == 2u, "exactly the noise is consumed");
        CHECK(adip_frame_find(noisy + 2, (size_t)n, &found, &found_len,
                              &consumed) == ADIP_OK,
              "the frame is found after resynchronising");
    }

    /* An absurd length is refused before anything is buffered. */
    {
        uint8_t bad[8];
        bad[0] = ADIP_FRAME_SYNC;
        bad[1] = 0xffu;
        bad[2] = 0xffu;
        CHECK(adip_frame_find(bad, sizeof(bad), &found, &found_len,
                              &consumed) == ADIP_ERR_MALFORMED,
              "an oversized length is refused without buffering");
    }

    /* Known-answer test, so an independent implementation can be checked
     * against this one. CRC-16/CCITT-FALSE of "123456789" is 0x29B1. */
    CHECK(adip_crc16((const uint8_t *)"123456789", 9) == 0x29B1u,
          "CRC-16/CCITT-FALSE known answer");
}

/* ----------------------------------------------------------------- decoding */

static void test_decoding(void)
{
    const adip_device_t *dev = &adip_device_neutral;
    builder_t b;
    adip_invocation_t inv;
    adip_reason_t r;

    printf("decoding\n");

    /* 9.0 g as a decimal fraction. */
    build_num_call(&b, "adi.example.input.by_mass", 41u, "mass", 90, -1);
    r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(r, ADIP_REASON_NONE, "a well formed call decodes");
    CHECK(inv.invocation_id == 41u, "invocation id");
    CHECK(inv.tool != NULL, "tool resolved");
    CHECK(inv.arg_count == 1, "one argument");
    CHECK(inv.args[0].is_text == 0u, "numeric argument");
    CHECK(adip_num_cmp(inv.args[0].num, ADIP_NUM(90, -1, ADIP_UNIT_G)) ==
              ADIP_CMP_EQ,
          "9.0 g survived the wire exactly");

    /* The unit comes from the declaration, never from the wire. */
    CHECK(inv.args[0].num.unit == ADIP_UNIT_G, "unit taken from the tables");

    /* An undeclared tool gets its own reason, not a parse error. */
    build_num_call(&b, "x.nobody.invented.this", 1u, "mass", 90, -1);
    r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(r, ADIP_REASON_UNKNOWN_TOOL, "an undeclared tool is named");

    /* An argument the declaration does not describe. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "nonsense", 1, 0);
    r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(r, ADIP_REASON_OUT_OF_RANGE, "an undeclared argument");

    /* Truncation at every length must be refused, never accepted or crash. */
    {
        size_t i;
        int accepted_short = 0;
        build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 90, -1);
        for (i = 1; i < b.pos; i++) {
            if (adip_decode_invocation(dev, b.buf, i, &inv) ==
                ADIP_REASON_NONE) {
                accepted_short = 1;
            }
        }
        CHECK(accepted_short == 0, "no truncated prefix is accepted");
    }

    /* A binary float is refused. It cannot represent 0.1 exactly, and
     * accepting one would reintroduce the ambiguity scaled integers remove. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_head(&b, 4u, 2u);                 /* a well formed quantity ... */
        b_uint(&b, unit_for("adi.example.input.by_mass", "mass"));
        b.buf[b.pos++] = 0xfbu;             /* ... carrying a float64 */
        memset(b.buf + b.pos, 0, 8);
        b.pos += 8;
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "a binary float is refused");
    }

    /* Unknown keys are tolerated, per PROTOCOL.md 11. */
    {
        b.pos = 0;
        b_map(&b, 5u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 7u);
        b_uint(&b, 99u); b_text(&b, "from a newer engine");
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_NONE, "an unknown key is skipped");
        CHECK(inv.invocation_id == 7u, "the rest still decodes");
    }

    /* A control message without a kind is not a control message.
     * PROTOCOL.md 10.2.4. */
    {
        b.pos = 0;
        b_map(&b, 3u);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "a message with no kind is refused");
    }

    /* A kind this decoder does not route here is refused rather than assumed
     * to be an invocation. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_SIGNALS);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "the wrong kind is refused");
    }

    /* A unit this version does not define is refused rather than carried. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, 200u, 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "an undefined unit is refused");
    }

    /* A bare magnitude with no unit is no longer a quantity. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_decimal(&b, 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "a unitless number is refused");
    }

    /* A message with no kind is not a control message. PROTOCOL.md 10.2.4. */
    {
        b.pos = 0;
        b_map(&b, 3u);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "a message with no kind is refused");
    }

    /* A kind this decoder does not implement is refused rather than assumed
     * to be an invocation. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_SIGNALS);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_MALFORMED, "an outbound kind is refused inbound");
    }

    /* Key order must not matter. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        b_uint(&b, 2u); b_uint(&b, 5u);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_NONE, "arguments before the tool id");
        CHECK(inv.arg_count == 1, "argument still matched");
    }
}

/* --------------------------------------------------------------- validation */

static void test_validation(void)
{
    const adip_device_t *dev = &adip_device_neutral;
    builder_t b;
    adip_invocation_t inv;
    adip_reason_t r;
    uint32_t all_preconditions = 0xffffffffu;

    printf("validation\n");

    /* In range and on the 0.1 g grid. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 90, -1);
    CHECK(adip_decode_invocation(dev, b.buf, b.pos, &inv) == ADIP_REASON_NONE,
          "decodes");
    r = adip_validate(dev, &inv, 0u, all_preconditions);
    CHECK_REASON(r, ADIP_REASON_NONE, "9.0 g is accepted");

    /* Out of range: 40 g against a 6 to 14 g parameter. Refused, never
     * clamped to 14, because clamping would hide an engine defect forever. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 400, -1);
    CHECK(adip_decode_invocation(dev, b.buf, b.pos, &inv) == ADIP_REASON_NONE,
          "decodes");
    r = adip_validate(dev, &inv, 0u, all_preconditions);
    CHECK_REASON(r, ADIP_REASON_OUT_OF_RANGE, "40 g is refused, not clamped");

    /* In range but off the resolution grid. 9.05 is unreachable on a 0.1 g
     * mechanism, so it is not a value the machine can honestly accept. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 905, -2);
    CHECK(adip_decode_invocation(dev, b.buf, b.pos, &inv) == ADIP_REASON_NONE,
          "decodes");
    r = adip_validate(dev, &inv, 0u, all_preconditions);
    CHECK_REASON(r, ADIP_REASON_OUT_OF_RANGE, "9.05 g is off the 0.1 grid");

    /* Boundaries are inclusive. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 60, -1);
    (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                 ADIP_REASON_NONE, "6.0 g, the declared minimum, is accepted");

    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 140, -1);
    (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                 ADIP_REASON_NONE, "14.0 g, the declared maximum, is accepted");

    /* A missing required argument. */
    {
        b.pos = 0;
        b_map(&b, 3u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
        r = adip_validate(dev, &inv, 0u, all_preconditions);
        CHECK_REASON(r, ADIP_REASON_PRECONDITION_FAILED,
                     "a missing required argument is refused");
    }

    /*
     * An unsatisfied precondition.
     *
     * This uses process.start rather than input.by_mass, because the latter
     * declares no preconditions and an empty mask is correctly satisfied by
     * anything. Picking a tool that declares none would test nothing.
     */
    build_num_call(&b, "adi.example.process.start", 1u, "stop_at_volume", 360, -1);
    CHECK(adip_decode_invocation(dev, b.buf, b.pos, &inv) == ADIP_REASON_NONE,
          "process call decodes");
    CHECK(inv.tool->precondition_mask != 0u,
          "the tool under test actually declares preconditions");

    r = adip_validate(dev, &inv, 0u, all_preconditions);
    CHECK_REASON(r, ADIP_REASON_NONE, "36.0 g is accepted when ready");

    r = adip_validate(dev, &inv, 0u, 0u);
    CHECK_REASON(r, ADIP_REASON_PRECONDITION_FAILED,
                 "an unsatisfied precondition is refused");

    /* Partially satisfied is still unsatisfied. All declared preconditions
     * must hold, not merely some of them. */
    r = adip_validate(dev, &inv, 0u, 1u);
    CHECK_REASON(r, ADIP_REASON_PRECONDITION_FAILED,
                 "one of two preconditions is not enough");

    /* Interlocks outrank arguments: a perfectly valid call is still refused
     * while the service door is open. */
    {
        uint8_t i;
        uint32_t door = 0;
        for (i = 0; i < dev->interlock_count; i++) {
            if (dev->interlocks[i].blocks_mask == ADIP_BLOCKS_ALL) {
                door = (uint32_t)(1u << i);
            }
        }
        CHECK(door != 0u, "the fixture has a wildcard interlock");

        build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 90, -1);
        (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
        r = adip_validate(dev, &inv, door, all_preconditions);
        CHECK_REASON(r, ADIP_REASON_INTERLOCK,
                     "a wildcard interlock blocks a valid call");
    }

    /* Enum values are checked against the declared set. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.additive.dispense");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 2u);
        b_text(&b, "substance"); b_text(&b, "a");
        b_text(&b, "volume");    b_quantity(&b, unit_for("adi.example.additive.dispense", "volume"), 50, 0);
        CHECK(adip_decode_invocation(dev, b.buf, b.pos, &inv) ==
                  ADIP_REASON_NONE, "enum call decodes");
        CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                     ADIP_REASON_NONE, "a declared enum value is accepted");

        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.additive.dispense");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 2u);
        b_text(&b, "substance"); b_text(&b, "z");
        b_text(&b, "volume");    b_quantity(&b, unit_for("adi.example.additive.dispense", "volume"), 50, 0);
        (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                     ADIP_REASON_OUT_OF_RANGE,
                     "an undeclared enum value is refused");
    }

    /*
     * A value in range, in the wrong unit.
     *
     * 9.0 is inside the 6.0 to 14.0 the declaration allows for `mass`, so no
     * range check can object. Only the unit says the engine computed a volume
     * and sent it to a mass parameter. PROTOCOL.md 10.2.3.
     */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 1u);
        b_text(&b, "mass");
        b_quantity(&b, (uint8_t)ADIP_UNIT_ML, 90, -1);
        r = adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(r, ADIP_REASON_NONE, "a wrong unit still decodes");
        CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                     ADIP_REASON_UNIT_MISMATCH,
                     "millilitres into a mass parameter are refused");
    }

    /* A duplicated parameter is ambiguous and is refused rather than resolved
     * by last-one-wins. */
    {
        b.pos = 0;
        b_map(&b, 4u);
        b_uint(&b, 0u); b_uint(&b, ADIP_KIND_INVOCATION);
        b_uint(&b, 1u); b_text(&b, "adi.example.input.by_mass");
        b_uint(&b, 2u); b_uint(&b, 1u);
        b_uint(&b, 3u);
        b_map(&b, 2u);
        b_text(&b, "mass"); b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 90, -1);
        b_text(&b, "mass"); b_quantity(&b, unit_for("adi.example.input.by_mass", "mass"), 120, -1);
        (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
        CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                     ADIP_REASON_MALFORMED, "a duplicated argument is refused");
    }

    /* The hard limit on "mass" is 16 g and applies whatever the tool. The
     * parameter range is tighter here, so this confirms the ordering rather
     * than the limit firing. */
    build_num_call(&b, "adi.example.input.by_mass", 1u, "mass", 150, -1);
    (void)adip_decode_invocation(dev, b.buf, b.pos, &inv);
    CHECK_REASON(adip_validate(dev, &inv, 0u, all_preconditions),
                 ADIP_REASON_OUT_OF_RANGE,
                 "15 g fails the parameter range before the hard limit");
}

/* ------------------------------------------------------------------ replies */

static void test_replies(void)
{
    uint8_t out[32];
    int n;

    printf("replies\n");

    n = adip_encode_response(41u, ADIP_OUTCOME_ACCEPTED, ADIP_REASON_NONE, out,
                             sizeof(out));
    CHECK(n > 0, "an acceptance encodes");

    n = adip_encode_response(41u, ADIP_OUTCOME_REJECTED,
                             ADIP_REASON_INTERLOCK, out, sizeof(out));
    CHECK(n > 0, "a rejection encodes");

    /* A buffer too small must report, never overrun. */
    n = adip_encode_response(41u, ADIP_OUTCOME_REJECTED,
                             ADIP_REASON_INTERLOCK, out, 2);
    CHECK(n < 0, "a short buffer is reported rather than overrun");
}

int main(void)
{
    printf("libadip M2 tests\n\n");
    test_framing();
    test_decoding();
    test_validation();
    test_replies();
    printf("\n%d checks, %d failures\n", checks, failures);
    return failures == 0 ? 0 : 1;
}
