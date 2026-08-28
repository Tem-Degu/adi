/*
 * adip_frame.c - Framing and CRC for stream transports.
 *
 * PROTOCOL.md section 10 requires framing and a CRC on the UART binding. This
 * is that.
 *
 * The length is checked before any payload is examined, so an oversized or
 * absurd length is refused without being buffered.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_wire.h"

/* CRC-16/CCITT-FALSE: polynomial 0x1021, initial value 0xFFFF, no reflection,
 * no final xor. Chosen because it is ubiquitous in embedded work and every
 * firmware team already has an implementation to check against. */
uint16_t adip_crc16(const uint8_t *data, size_t len)
{
    uint16_t crc = 0xFFFFu;
    size_t i;
    unsigned bit;

    if (data == NULL) {
        return crc;
    }

    for (i = 0; i < len; i++) {
        crc ^= (uint16_t)((uint16_t)data[i] << 8);
        for (bit = 0; bit < 8u; bit++) {
            if ((crc & 0x8000u) != 0u) {
                crc = (uint16_t)((uint16_t)(crc << 1) ^ 0x1021u);
            } else {
                crc = (uint16_t)(crc << 1);
            }
        }
    }

    return crc;
}

adip_status_t adip_frame_find(const uint8_t *buf, size_t len,
                              const uint8_t **payload, size_t *payload_len,
                              size_t *consumed)
{
    size_t start = 0;
    size_t plen;
    size_t total;
    uint16_t got;
    uint16_t want;

    if (buf == NULL || payload == NULL || payload_len == NULL ||
        consumed == NULL) {
        return ADIP_ERR_ARG;
    }

    *consumed = 0;

    /* Resynchronise. Anything before a sync byte is noise the caller may
     * discard, which is how a receiver recovers after corruption. */
    while (start < len && buf[start] != ADIP_FRAME_SYNC) {
        start++;
    }
    if (start > 0) {
        *consumed = start;
        return ADIP_PENDING;
    }

    if (len < ADIP_FRAME_HEADER) {
        return ADIP_PENDING;
    }

    plen = ((size_t)buf[1] << 8) | (size_t)buf[2];

    /* Refuse before buffering, not after. */
    if (plen == 0 || plen > ADIP_MAX_FRAME) {
        *consumed = 1; /* skip this sync byte and look for the next */
        return ADIP_ERR_MALFORMED;
    }

    total = ADIP_FRAME_HEADER + plen + ADIP_FRAME_TRAILER;
    if (len < total) {
        return ADIP_PENDING;
    }

    got = (uint16_t)(((uint16_t)buf[ADIP_FRAME_HEADER + plen] << 8) |
                     (uint16_t)buf[ADIP_FRAME_HEADER + plen + 1u]);
    want = adip_crc16(buf + ADIP_FRAME_HEADER, plen);

    if (got != want) {
        *consumed = 1;
        return ADIP_ERR_MALFORMED;
    }

    *payload = buf + ADIP_FRAME_HEADER;
    *payload_len = plen;
    *consumed = total;
    return ADIP_OK;
}

int adip_frame_wrap(const uint8_t *payload, size_t payload_len,
                    uint8_t *out, size_t out_len)
{
    size_t total;
    size_t i;
    uint16_t crc;

    if (payload == NULL || out == NULL) {
        return (int)ADIP_ERR_ARG;
    }
    if (payload_len == 0 || payload_len > ADIP_MAX_FRAME) {
        return (int)ADIP_ERR_ARG;
    }

    total = ADIP_FRAME_HEADER + payload_len + ADIP_FRAME_TRAILER;
    if (out_len < total) {
        return (int)ADIP_ERR_NO_SPACE;
    }

    out[0] = ADIP_FRAME_SYNC;
    out[1] = (uint8_t)((payload_len >> 8) & 0xffu);
    out[2] = (uint8_t)(payload_len & 0xffu);
    for (i = 0; i < payload_len; i++) {
        out[ADIP_FRAME_HEADER + i] = payload[i];
    }

    crc = adip_crc16(payload, payload_len);
    out[ADIP_FRAME_HEADER + payload_len] = (uint8_t)((crc >> 8) & 0xffu);
    out[ADIP_FRAME_HEADER + payload_len + 1u] = (uint8_t)(crc & 0xffu);

    return (int)total;
}
