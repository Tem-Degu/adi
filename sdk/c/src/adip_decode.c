/*
 * adip_decode.c - Turn a CBOR payload into a resolved invocation.
 *
 * Message shape, integer keys to keep it compact:
 *
 *   {
 *     1: "adi.example.input.by_mass",   tool id
 *     2: 41,                            invocation id
 *     3: { "mass": 4([-1, 90]) }        arguments
 *   }
 *
 * Decoding resolves the tool and matches every argument to a declared
 * parameter. It does not check values; adip_validate does that. The split
 * exists so a rejection carries the right reason: an unrecognised tool is
 * UNKNOWN_TOOL, an unrecognised argument name is OUT_OF_RANGE against the
 * declaration, and a bad value is caught later with its own code.
 *
 * Nothing is copied. Text arguments point into the caller's buffer.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_cbor.h"
#include "adip_wire.h"

#include <string.h>

/* Invocation map keys. */
#define KEY_KIND 0u
#define KEY_TOOL 1u
#define KEY_ID   2u
#define KEY_ARGS 3u

/* Response map keys. Numbered independently of the invocation keys; they are
 * different messages and sharing the numbering only invites confusion. */
#define RKEY_ID      1u
#define RKEY_OUTCOME 2u
#define RKEY_REASON  3u

const adip_tool_t *adip_find_tool_n(const adip_device_t *dev,
                                    const char *id, size_t len)
{
    uint16_t i;

    if (dev == NULL || id == NULL || dev->tools == NULL) {
        return NULL;
    }

    for (i = 0; i < dev->tool_count; i++) {
        const char *candidate = dev->tools[i].id;
        if (strlen(candidate) == len && memcmp(candidate, id, len) == 0) {
            return &dev->tools[i];
        }
    }

    return NULL;
}

static int find_param_n(const adip_tool_t *tool, const char *name, size_t len)
{
    uint8_t i;

    for (i = 0; i < tool->param_count; i++) {
        const char *candidate = tool->params[i].name;
        if (strlen(candidate) == len && memcmp(candidate, name, len) == 0) {
            return (int)i;
        }
    }

    return -1;
}

static adip_reason_t decode_args(const adip_tool_t *tool, adip_cbor_t *c,
                                 adip_invocation_t *out)
{
    uint64_t count;
    uint64_t i;
    const char *name;
    size_t name_len;
    int idx;

    if (!adip_cbor_map(c, &count)) {
        return ADIP_REASON_MALFORMED;
    }
    if (count > (uint64_t)ADIP_MAX_ARGS) {
        /* More arguments than any declared tool can have. Refuse rather than
         * truncating, because a truncated argument list would silently drop a
         * value the engine believed it had sent. */
        return ADIP_REASON_MALFORMED;
    }

    for (i = 0; i < count; i++) {
        adip_arg_t *arg;
        const adip_param_t *param;

        if (!adip_cbor_text(c, &name, &name_len)) {
            return ADIP_REASON_MALFORMED;
        }

        idx = find_param_n(tool, name, name_len);
        if (idx < 0) {
            /* An argument the declaration does not describe. There is no
             * permissive path: the engine sent something this machine never
             * advertised. */
            return ADIP_REASON_OUT_OF_RANGE;
        }

        param = &tool->params[idx];
        arg = &out->args[out->arg_count];
        arg->index = (uint8_t)idx;

        if (param->kind == ADIP_PARAM_ENUM) {
            if (!adip_cbor_text(c, &arg->text, &arg->text_len)) {
                return ADIP_REASON_MALFORMED;
            }
            arg->is_text = 1u;
            arg->num = ADIP_NUM_ZERO;
        } else if (param->kind == ADIP_PARAM_NUMBER ||
                   param->kind == ADIP_PARAM_INTEGER) {
            /*
             * A quantity carries its unit, PROTOCOL.md 10.2.3, so the unit
             * checked in adip_validate is the one the engine meant rather
             * than the one the declaration assumed. Reading the magnitude
             * alone made that check compare a value against the thing it had
             * just been stamped from, so it could never fail, and an engine
             * that computed in the wrong quantity sent a number that was
             * merely a number.
             */
            if (!adip_cbor_quantity(c, &arg->num)) {
                return ADIP_REASON_MALFORMED;
            }
            arg->is_text = 0u;
            arg->text = NULL;
            arg->text_len = 0;
        } else {
            /* Object parameters are declared by schema reference and are not
             * interpreted on the device. */
            return ADIP_REASON_MALFORMED;
        }

        out->arg_count++;
    }

    return ADIP_REASON_NONE;
}

adip_reason_t adip_decode_invocation(const adip_device_t *dev,
                                     const uint8_t *payload, size_t len,
                                     adip_invocation_t *out)
{
    adip_cbor_t c;
    uint64_t count;
    uint64_t i;
    uint64_t key;
    const char *tool_id = NULL;
    size_t tool_id_len = 0;
    bool have_args = false;
    bool have_kind = false;
    adip_reason_t r;

    if (dev == NULL || payload == NULL || out == NULL) {
        return ADIP_REASON_MALFORMED;
    }

    memset(out, 0, sizeof(*out));

    c.buf = payload;
    c.len = len;
    c.pos = 0;

    if (!adip_cbor_map(&c, &count)) {
        return ADIP_REASON_MALFORMED;
    }
    /* A map larger than the payload cannot be honest. */
    if (count > (uint64_t)len) {
        return ADIP_REASON_MALFORMED;
    }

    /*
     * Two passes over the map. The tool must be known before arguments can be
     * matched to parameters, and a sender is not required to order the keys.
     * Pass one takes the tool and the invocation id; pass two takes the
     * arguments once the tool is resolved.
     */
    for (i = 0; i < count; i++) {
        if (!adip_cbor_uint(&c, &key)) {
            return ADIP_REASON_MALFORMED;
        }

        switch (key) {
            case KEY_KIND: {
                uint64_t v;
                if (!adip_cbor_uint(&c, &v)) {
                    return ADIP_REASON_MALFORMED;
                }
                /* Routing happens in adip_poll. Reaching here with any other
                 * kind means the caller routed wrongly, which is a defect in
                 * this library rather than in the message. */
                if (v != ADIP_KIND_INVOCATION) {
                    return ADIP_REASON_MALFORMED;
                }
                have_kind = true;
                break;
            }
            case KEY_TOOL:
                if (!adip_cbor_text(&c, &tool_id, &tool_id_len)) {
                    return ADIP_REASON_MALFORMED;
                }
                break;
            case KEY_ID: {
                uint64_t v;
                if (!adip_cbor_uint(&c, &v) || v > 0xffffffffu) {
                    return ADIP_REASON_MALFORMED;
                }
                out->invocation_id = (uint32_t)v;
                break;
            }
            case KEY_ARGS:
                have_args = true;
                if (!adip_cbor_skip(&c)) {
                    return ADIP_REASON_MALFORMED;
                }
                break;
            default:
                /* An unknown key is skipped, not refused. A newer engine may
                 * add optional fields, and PROTOCOL.md 11 requires tolerating
                 * unknown optional fields within a supported major version. */
                if (!adip_cbor_skip(&c)) {
                    return ADIP_REASON_MALFORMED;
                }
                break;
        }
    }

    /*
     * PROTOCOL.md 10.2.4: every control message carries its kind. Tolerating
     * its absence would leave exactly one message this library could ever
     * receive, which is where it started and why signals had nowhere to go.
     */
    if (!have_kind) {
        return ADIP_REASON_MALFORMED;
    }

    if (tool_id == NULL) {
        return ADIP_REASON_MALFORMED;
    }

    out->tool = adip_find_tool_n(dev, tool_id, tool_id_len);
    if (out->tool == NULL) {
        return ADIP_REASON_UNKNOWN_TOOL;
    }

    if (!have_args) {
        return ADIP_REASON_NONE; /* a tool with no parameters */
    }

    /* Second pass, now that the tool is known. */
    c.pos = 0;
    if (!adip_cbor_map(&c, &count)) {
        return ADIP_REASON_MALFORMED;
    }

    for (i = 0; i < count; i++) {
        if (!adip_cbor_uint(&c, &key)) {
            return ADIP_REASON_MALFORMED;
        }
        if (key == KEY_ARGS) {
            r = decode_args(out->tool, &c, out);
            if (r != ADIP_REASON_NONE) {
                return r;
            }
        } else if (!adip_cbor_skip(&c)) {
            return ADIP_REASON_MALFORMED;
        }
    }

    return ADIP_REASON_NONE;
}

bool adip_peek_kind(const uint8_t *payload, size_t len, uint8_t *kind)
{
    adip_cbor_t c;
    uint64_t count;
    uint64_t i;
    uint64_t key;

    if (payload == NULL || kind == NULL) {
        return false;
    }

    c.buf = payload;
    c.len = len;
    c.pos = 0;

    if (!adip_cbor_map(&c, &count)) {
        return false;
    }
    if (count > (uint64_t)len) {
        return false;
    }

    for (i = 0; i < count; i++) {
        if (!adip_cbor_uint(&c, &key)) {
            return false;
        }
        if (key == KEY_KIND) {
            uint64_t v;
            if (!adip_cbor_uint(&c, &v) || v > 0xffu) {
                return false;
            }
            *kind = (uint8_t)v;
            return true;
        }
        if (!adip_cbor_skip(&c)) {
            return false;
        }
    }

    return false;
}

int adip_encode_signals(const adip_sample_t *samples, uint8_t count,
                        uint8_t *out, size_t out_len)
{
    adip_cbor_w_t w;
    uint8_t i;
    const adip_sample_t *s = samples;

    if (out == NULL || samples == NULL || count == 0u) {
        return (int)ADIP_ERR_ARG;
    }

    w.buf = out;
    w.len = out_len;
    w.pos = 0;

    if (!adip_cbor_put_map(&w, 2u) ||
        !adip_cbor_put_uint(&w, KEY_KIND) ||
        !adip_cbor_put_uint(&w, ADIP_KIND_SIGNALS) ||
        !adip_cbor_put_uint(&w, 1u) ||
        !adip_cbor_put_array(&w, (uint64_t)count)) {
        return (int)ADIP_ERR_NO_SPACE;
    }

    for (i = 0; i < count; i++) {
        if (!adip_cbor_put_array(&w, 3u) ||
            !adip_cbor_put_uint(&w, (uint64_t)s[i].signal) ||
            !adip_cbor_put_quantity(&w, s[i].value) ||
            !adip_cbor_put_uint(&w, (uint64_t)s[i].t_ms)) {
            return (int)ADIP_ERR_NO_SPACE;
        }
    }

    return (int)w.pos;
}

int adip_encode_heartbeat(bool busy, bool faulted, uint8_t *out, size_t out_len)
{
    adip_cbor_w_t w;

    if (out == NULL) {
        return (int)ADIP_ERR_ARG;
    }

    w.buf = out;
    w.len = out_len;
    w.pos = 0;

    if (!adip_cbor_put_map(&w, 3u) ||
        !adip_cbor_put_uint(&w, KEY_KIND) ||
        !adip_cbor_put_uint(&w, ADIP_KIND_HEARTBEAT) ||
        !adip_cbor_put_uint(&w, 1u) ||
        !adip_cbor_put_bool(&w, busy) ||
        !adip_cbor_put_uint(&w, 2u) ||
        !adip_cbor_put_bool(&w, faulted)) {
        return (int)ADIP_ERR_NO_SPACE;
    }

    return (int)w.pos;
}

int adip_encode_response(uint32_t invocation_id, adip_outcome_t outcome,
                         adip_reason_t reason, uint8_t *out, size_t out_len)
{
    adip_cbor_w_t w;
    uint64_t entries;

    if (out == NULL) {
        return (int)ADIP_ERR_ARG;
    }

    w.buf = out;
    w.len = out_len;
    w.pos = 0;

    entries = (reason == ADIP_REASON_NONE) ? 2u : 3u;

    if (!adip_cbor_put_map(&w, entries) ||
        !adip_cbor_put_uint(&w, RKEY_ID) ||
        !adip_cbor_put_uint(&w, invocation_id) ||
        !adip_cbor_put_uint(&w, RKEY_OUTCOME) ||
        !adip_cbor_put_uint(&w, (uint64_t)outcome)) {
        return (int)ADIP_ERR_NO_SPACE;
    }

    if (reason != ADIP_REASON_NONE) {
        if (!adip_cbor_put_uint(&w, RKEY_REASON) ||
            !adip_cbor_put_uint(&w, (uint64_t)reason)) {
            return (int)ADIP_ERR_NO_SPACE;
        }
    }

    return (int)w.pos;
}
