/*
 * adip_wire.h - Framing, decoding and validation.
 *
 * M2 of sdk/c/DESIGN.md: bytes in, a validated invocation or a rejection with
 * the correct reason code.
 *
 * ENCODING: CBOR (RFC 8949), with numbers carried as tag 4 decimal fractions.
 *
 * A decimal fraction is encoded as [exponent, mantissa], which is exactly
 * adip_num_t. Numbers cross the wire with no conversion and no rounding, which
 * is what lets a range check against a safety limit stay exact. JSON would have
 * meant parsing decimal text into scaled integers on a microcontroller.
 *
 * Binary floats (CBOR major type 7) are REJECTED rather than converted. A float
 * on the wire cannot represent 0.1 exactly, so accepting one would reintroduce
 * the ambiguity the scaled-integer design exists to remove.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#ifndef ADIP_WIRE_H
#define ADIP_WIRE_H

#include "adip_tables.h"
#include "adip_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ------------------------------------------------------------------ limits */

/*
 * Largest number of arguments in one invocation. Fixed so the invocation
 * struct can live on the stack with a statically known size.
 */
#define ADIP_MAX_ARGS 8

/* Largest frame payload accepted. Anything larger is rejected at the framing
 * layer without being buffered, so an oversized frame cannot exhaust memory. */
#define ADIP_MAX_FRAME 512

/* ------------------------------------------------------------------ framing */

/*
 * Frame layout, for a stream transport such as UART:
 *
 *   0xAD  len_hi  len_lo  <payload ...>  crc_hi  crc_lo
 *
 * The sync byte lets a receiver resynchronise after corruption. The length is
 * checked before any payload is buffered. The CRC is CRC-16/CCITT-FALSE over
 * the payload only, matching PROTOCOL.md 10, which requires framing and a CRC
 * on the UART binding.
 */
#define ADIP_FRAME_SYNC   0xADu
#define ADIP_FRAME_HEADER 3u  /* sync + length */
#define ADIP_FRAME_TRAILER 2u /* crc */

uint16_t adip_crc16(const uint8_t *data, size_t len);

/*
 * Locate one complete frame in a buffer.
 *
 * Returns ADIP_OK and sets payload and payload_len when a whole frame with a
 * valid CRC is present, ADIP_PENDING when more bytes are needed, or
 * ADIP_ERR_MALFORMED when the frame is structurally wrong. `consumed` is always
 * set to how many bytes the caller may discard, so a caller can make progress
 * even when resynchronising past corruption.
 */
adip_status_t adip_frame_find(const uint8_t *buf, size_t len,
                              const uint8_t **payload, size_t *payload_len,
                              size_t *consumed);

/* Write a frame around `payload` into `out`. Returns the total length written,
 * or ADIP_ERR_NO_SPACE as a negative value if `out_len` is too small. */
int adip_frame_wrap(const uint8_t *payload, size_t payload_len,
                    uint8_t *out, size_t out_len);

/* --------------------------------------------------------------- decoding */

/* One decoded argument, resolved against the tool's parameter table. */
typedef struct {
    uint8_t     index;    /* index into tool->params */
    uint8_t     is_text;  /* enum values arrive as text */
    adip_num_t  num;      /* valid when is_text == 0 */

    /*
     * Text points into the caller's input buffer and is NOT NUL terminated.
     * It stays valid only as long as that buffer does. Nothing is copied,
     * because copying would need somewhere to copy to.
     */
    const char *text;
    size_t      text_len;
} adip_arg_t;

typedef struct {
    uint32_t            invocation_id;
    const adip_tool_t  *tool;
    adip_arg_t          args[ADIP_MAX_ARGS];
    uint8_t             arg_count;
} adip_invocation_t;

/*
 * Decode one CBOR invocation payload.
 *
 * On success the tool is resolved and every argument is matched to a declared
 * parameter. This does not validate values; call adip_validate for that.
 *
 * Returns ADIP_REASON_NONE on success. On failure the reason is what the
 * appliance should report, so an unknown tool comes back as
 * ADIP_REASON_UNKNOWN_TOOL rather than as a generic parse error.
 */
adip_reason_t adip_decode_invocation(const adip_device_t *dev,
                                     const uint8_t *payload, size_t len,
                                     adip_invocation_t *out);

/* ------------------------------------------------------------- validation */

/*
 * Check a decoded invocation against the declaration.
 *
 * `open_interlocks` and `satisfied_preconditions` are bitmaps the firmware
 * maintains, indexed the same way as the generated tables.
 *
 * Returns ADIP_REASON_NONE when the invocation may proceed.
 *
 * Out-of-range values are REJECTED, never clamped. An out-of-range value means
 * the engine is defective, and silently correcting it would hide that fault
 * permanently. See PROTOCOL.md 4.6 requirement 2.
 *
 * This covers everything expressible without history. Invariants, rate limits
 * and resources need accumulated state and arrive in M4.
 */
adip_reason_t adip_validate(const adip_device_t *dev,
                            const adip_invocation_t *inv,
                            uint32_t open_interlocks,
                            uint32_t satisfied_preconditions);

/* ------------------------------------------------------------------ replies */

/* Encode a response. Returns bytes written, or a negative adip_status_t. */
int adip_encode_response(uint32_t invocation_id, adip_outcome_t outcome,
                         adip_reason_t reason, uint8_t *out, size_t out_len);

/* Length-aware tool lookup, for ids that are not NUL terminated. */
const adip_tool_t *adip_find_tool_n(const adip_device_t *dev,
                                    const char *id, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* ADIP_WIRE_H */
