/*
 * adip_cbor.h - The CBOR subset ADI-P uses.
 *
 * Internal to libadip. Firmware talks to adip_wire.h, not to this.
 *
 * Accepted: unsigned integers, negative integers, text strings, arrays, maps,
 * and tag 4 decimal fractions. Everything else, including binary floats and
 * indefinite-length items, is refused.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#ifndef ADIP_CBOR_H
#define ADIP_CBOR_H

#include "adip_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/* A read cursor over a caller-owned buffer. Nothing is copied. */
typedef struct {
    const uint8_t *buf;
    size_t         len;
    size_t         pos;
} adip_cbor_t;

/* A write cursor over a caller-owned buffer. */
typedef struct {
    uint8_t *buf;
    size_t   len;
    size_t   pos;
} adip_cbor_w_t;

/* Read the next item head. Returns the major type and its argument. */
bool adip_cbor_head(adip_cbor_t *c, uint8_t *mt, uint64_t *value);

bool adip_cbor_uint(adip_cbor_t *c, uint64_t *out);
bool adip_cbor_int(adip_cbor_t *c, int64_t *out);
bool adip_cbor_text(adip_cbor_t *c, const char **out, size_t *out_len);
bool adip_cbor_map(adip_cbor_t *c, uint64_t *count);

/*
 * Read a number as adip_num_t. Accepts a plain integer or a tag 4 decimal
 * fraction. `unit` is stamped onto the result, so this reads a magnitude and
 * nothing more.
 */
bool adip_cbor_num(adip_cbor_t *c, uint8_t unit, adip_num_t *out);

/*
 * Read a quantity: the two element array [unit, number] of PROTOCOL.md 10.2.3.
 *
 * Distinct from adip_cbor_num because the unit arrives rather than being
 * assumed. A magnitude whose unit came from the declaration can only ever
 * agree with the declaration, so the check against it could never fail and an
 * engine that computed in the wrong quantity went unseen.
 */
bool adip_cbor_quantity(adip_cbor_t *c, adip_num_t *out);

/* Skip one item, including nested containers. Iterative, so a hostile input
 * cannot drive it into the stack. */
bool adip_cbor_skip(adip_cbor_t *c);

bool adip_cbor_put_head(adip_cbor_w_t *w, uint8_t mt, uint64_t value);
bool adip_cbor_put_uint(adip_cbor_w_t *w, uint64_t v);
bool adip_cbor_put_map(adip_cbor_w_t *w, uint64_t count);
/* CBOR simple values: major type 7, 20 is false and 21 is true. */
bool adip_cbor_put_bool(adip_cbor_w_t *w, bool v);
bool adip_cbor_put_int(adip_cbor_w_t *w, int64_t v);
bool adip_cbor_put_text(adip_cbor_w_t *w, const char *s);
bool adip_cbor_put_array(adip_cbor_w_t *w, uint64_t count);

/* Write a tag 4 decimal fraction: the magnitude alone, without its unit. */
bool adip_cbor_put_num(adip_cbor_w_t *w, adip_num_t v);

/* Write [unit, number], the quantity form of PROTOCOL.md 10.2.3. */
bool adip_cbor_put_quantity(adip_cbor_w_t *w, adip_num_t v);

#ifdef __cplusplus
}
#endif

#endif /* ADIP_CBOR_H */
