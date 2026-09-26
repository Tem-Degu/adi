/*
 * adip_cbor.c - A minimal CBOR reader.
 *
 * Only the subset ADI-P uses: unsigned and negative integers, text strings,
 * arrays, maps, and tag 4 decimal fractions. Everything else is refused.
 *
 * Binary floats are refused deliberately rather than for want of effort. A
 * float cannot hold 0.1 exactly, so accepting one would put back the rounding
 * ambiguity the scaled-integer design removes.
 *
 * No allocation, no recursion. The one nested structure we need, a tag
 * containing a two element array, is read inline.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_cbor.h"

/* CBOR major types, RFC 8949 section 3.1. */
#define MT_UINT   0u
#define MT_NINT   1u
#define MT_BYTES  2u
#define MT_TEXT   3u
#define MT_ARRAY  4u
#define MT_MAP    5u
#define MT_TAG    6u
#define MT_SIMPLE 7u

/* The only tag we accept: a decimal fraction, [exponent, mantissa]. */
#define TAG_DECIMAL_FRACTION 4u

static bool take(adip_cbor_t *c, size_t n, const uint8_t **out)
{
    if (c == NULL || c->pos + n < c->pos || c->pos + n > c->len) {
        return false;
    }
    *out = c->buf + c->pos;
    c->pos += n;
    return true;
}

bool adip_cbor_head(adip_cbor_t *c, uint8_t *mt, uint64_t *value)
{
    const uint8_t *p;
    uint8_t ib;
    uint8_t ai;
    uint64_t v = 0;
    size_t i;
    size_t n;

    if (c == NULL || mt == NULL || value == NULL) {
        return false;
    }
    if (!take(c, 1, &p)) {
        return false;
    }

    ib = p[0];
    *mt = (uint8_t)(ib >> 5);
    ai = (uint8_t)(ib & 0x1fu);

    if (ai < 24u) {
        *value = ai;
        return true;
    }

    switch (ai) {
        case 24u: n = 1; break;
        case 25u: n = 2; break;
        case 26u: n = 4; break;
        case 27u: n = 8; break;
        default:
            /* 28 to 30 are reserved, 31 is indefinite length. Indefinite
             * length items have no statically bounded size, so they are
             * refused rather than supported. */
            return false;
    }

    if (!take(c, n, &p)) {
        return false;
    }
    for (i = 0; i < n; i++) {
        v = (v << 8) | (uint64_t)p[i];
    }
    *value = v;
    return true;
}

bool adip_cbor_uint(adip_cbor_t *c, uint64_t *out)
{
    uint8_t mt;
    uint64_t v;

    if (!adip_cbor_head(c, &mt, &v) || mt != MT_UINT) {
        return false;
    }
    *out = v;
    return true;
}

bool adip_cbor_int(adip_cbor_t *c, int64_t *out)
{
    uint8_t mt;
    uint64_t v;

    if (!adip_cbor_head(c, &mt, &v)) {
        return false;
    }

    if (mt == MT_UINT) {
        if (v > (uint64_t)INT64_MAX) {
            return false;
        }
        *out = (int64_t)v;
        return true;
    }

    if (mt == MT_NINT) {
        /* A negative integer encodes -1 - n. */
        if (v > (uint64_t)INT64_MAX) {
            return false;
        }
        *out = -1 - (int64_t)v;
        return true;
    }

    return false;
}

bool adip_cbor_text(adip_cbor_t *c, const char **out, size_t *out_len)
{
    uint8_t mt;
    uint64_t v;
    const uint8_t *p;

    if (!adip_cbor_head(c, &mt, &v) || mt != MT_TEXT) {
        return false;
    }
    if (v > (uint64_t)c->len) {
        return false;
    }
    if (!take(c, (size_t)v, &p)) {
        return false;
    }

    *out = (const char *)p;
    *out_len = (size_t)v;
    return true;
}

bool adip_cbor_map(adip_cbor_t *c, uint64_t *count)
{
    uint8_t mt;
    uint64_t v;

    if (!adip_cbor_head(c, &mt, &v) || mt != MT_MAP) {
        return false;
    }
    *count = v;
    return true;
}

/*
 * Read a number.
 *
 * Accepts a plain integer, or tag 4 wrapping [exponent, mantissa]. The tag
 * form is the whole reason CBOR was chosen: it is bit for bit the shape of
 * adip_num_t, so nothing is converted and nothing is rounded.
 *
 * The unit is not on the wire. It comes from the declaration, because the
 * parameter's unit is fixed by the declaration and an engine cannot change it.
 */
bool adip_cbor_num(adip_cbor_t *c, uint8_t unit, adip_num_t *out)
{
    uint8_t mt;
    uint64_t v;
    int64_t exponent;
    int64_t mantissa;
    size_t save;

    if (c == NULL || out == NULL) {
        return false;
    }

    save = c->pos;
    if (!adip_cbor_head(c, &mt, &v)) {
        return false;
    }

    if (mt == MT_UINT || mt == MT_NINT) {
        c->pos = save;
        if (!adip_cbor_int(c, &mantissa)) {
            return false;
        }
        if (mantissa > INT32_MAX || mantissa < INT32_MIN) {
            return false;
        }
        out->mantissa = (int32_t)mantissa;
        out->exponent = 0;
        out->unit = unit;
        return true;
    }

    if (mt != MT_TAG || v != TAG_DECIMAL_FRACTION) {
        /* Includes major type 7, which is where binary floats live. A float
         * cannot represent 0.1 exactly and is refused on purpose. */
        return false;
    }

    /* The tag must wrap a two element array. */
    if (!adip_cbor_head(c, &mt, &v) || mt != MT_ARRAY || v != 2u) {
        return false;
    }
    if (!adip_cbor_int(c, &exponent) || !adip_cbor_int(c, &mantissa)) {
        return false;
    }
    if (exponent > INT8_MAX || exponent < INT8_MIN) {
        return false;
    }
    if (mantissa > INT32_MAX || mantissa < INT32_MIN) {
        return false;
    }

    out->mantissa = (int32_t)mantissa;
    out->exponent = (int8_t)exponent;
    out->unit = unit;
    return true;
}

/*
 * Skip one item, including nested containers.
 *
 * Iterative with an explicit pending count rather than recursive, so stack
 * depth stays constant no matter what arrives. A hostile input cannot drive
 * this into the stack.
 */
bool adip_cbor_skip(adip_cbor_t *c)
{
    uint64_t pending = 1;
    uint8_t mt;
    uint64_t v;
    const uint8_t *p;
    unsigned guard = 0;

    while (pending > 0u) {
        /* Bounded work per call. A frame is at most ADIP_MAX_FRAME bytes, so
         * this can never legitimately be reached. */
        if (++guard > 4096u) {
            return false;
        }
        if (!adip_cbor_head(c, &mt, &v)) {
            return false;
        }
        pending--;

        switch (mt) {
            case MT_UINT:
            case MT_NINT:
                break;
            case MT_BYTES:
            case MT_TEXT:
                if (v > (uint64_t)c->len || !take(c, (size_t)v, &p)) {
                    return false;
                }
                break;
            case MT_ARRAY:
                if (v > (uint64_t)c->len) {
                    return false;
                }
                pending += v;
                break;
            case MT_MAP:
                if (v > (uint64_t)c->len) {
                    return false;
                }
                pending += v * 2u;
                break;
            case MT_TAG:
                pending += 1u;
                break;
            case MT_SIMPLE:
            default:
                /* Floats and simple values are not part of the accepted
                 * subset, so an item containing one cannot be skipped past
                 * either. Refuse the whole message. */
                return false;
        }
    }

    return true;
}

/* ------------------------------------------------------------------ writing */

bool adip_cbor_put_bool(adip_cbor_w_t *w, bool v)
{
    return adip_cbor_put_head(w, 7u, v ? 21u : 20u);
}

bool adip_cbor_put_head(adip_cbor_w_t *w, uint8_t mt, uint64_t value)
{
    uint8_t ai;
    size_t n;
    size_t i;

    if (w == NULL) {
        return false;
    }

    if (value < 24u) {
        ai = (uint8_t)value;
        n = 0;
    } else if (value <= 0xffu) {
        ai = 24u;
        n = 1;
    } else if (value <= 0xffffu) {
        ai = 25u;
        n = 2;
    } else if (value <= 0xffffffffu) {
        ai = 26u;
        n = 4;
    } else {
        ai = 27u;
        n = 8;
    }

    if (w->pos + 1u + n > w->len) {
        return false;
    }

    w->buf[w->pos++] = (uint8_t)((mt << 5) | ai);
    for (i = 0; i < n; i++) {
        w->buf[w->pos++] = (uint8_t)((value >> (8u * (n - 1u - i))) & 0xffu);
    }
    return true;
}

bool adip_cbor_put_uint(adip_cbor_w_t *w, uint64_t v)
{
    return adip_cbor_put_head(w, MT_UINT, v);
}

bool adip_cbor_put_map(adip_cbor_w_t *w, uint64_t count)
{
    return adip_cbor_put_head(w, MT_MAP, count);
}
