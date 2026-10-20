/*
 * adip_types.h - Core types for libadip, the ADI-P appliance-side SDK.
 *
 * Targets ADI-P 0.2. See PROTOCOL.md.
 *
 * Constraints this file is written under, from sdk/c/DESIGN.md section 2:
 *   - Freestanding C99. No stdio, no stdlib, no printf.
 *   - No dynamic allocation anywhere.
 *   - No floating point. See adip_num_t below.
 *   - Fails closed. Every enum reserves 0 for the invalid or unknown case
 *     where that is meaningful, so a zeroed struct is never accidentally valid.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#ifndef ADIP_TYPES_H
#define ADIP_TYPES_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The protocol version this build implements. */
#define ADIP_VERSION_MAJOR 0
#define ADIP_VERSION_MINOR 2

/* ------------------------------------------------------------------ units */

/*
 * The closed unit vocabulary from PROTOCOL.md 4.2. A declaration may not use
 * any other unit, and a value without a unit cannot cross the protocol
 * boundary except as an index or a count.
 */
typedef enum {
    ADIP_UNIT_NONE     = 0,  /* dimensionless ratio, permitted only where the
                                registry declares it */
    ADIP_UNIT_G        = 1,  /* mass, grams */
    ADIP_UNIT_G_PER_S  = 2,  /* mass flow */
    ADIP_UNIT_ML       = 3,  /* volume */
    ADIP_UNIT_ML_PER_S = 4,  /* volume flow */
    ADIP_UNIT_C        = 5,  /* temperature, degrees Celsius */
    ADIP_UNIT_BAR      = 6,  /* pressure */
    ADIP_UNIT_S        = 7,  /* time, seconds */
    ADIP_UNIT_MS       = 8,  /* time, milliseconds */
    ADIP_UNIT_STEP     = 9,  /* discrete mechanism position */
    ADIP_UNIT_KG_F     = 10, /* force */
    ADIP_UNIT_PCT      = 11, /* percentage, 0 to 100 */
    ADIP_UNIT__COUNT   = 12
} adip_unit_t;

/* ---------------------------------------------------------------- numbers */

/*
 * A number as a scaled integer: value = mantissa * 10^exponent.
 *
 * 36.5 grams is { 365, -1, ADIP_UNIT_G }.
 *
 * There is no floating point in libadip. Cortex-M0 and M3 have no FPU, and
 * more importantly a range check against a safety limit has to be exact and
 * reproducible. Integer comparison after exponent normalisation gives that;
 * binary floating point does not.
 */
typedef struct {
    int32_t mantissa;
    int8_t  exponent;
    uint8_t unit;      /* adip_unit_t */
} adip_num_t;

#define ADIP_NUM(m, e, u) ((adip_num_t){ (int32_t)(m), (int8_t)(e), (uint8_t)(u) })
#define ADIP_NUM_ZERO     ADIP_NUM(0, 0, ADIP_UNIT_NONE)

/* Largest exponent shift attempted when normalising two numbers for
 * comparison. Beyond this the mantissa would overflow int64. */
#define ADIP_NUM_MAX_SHIFT 18

/* Result of comparing two numbers. */
typedef enum {
    ADIP_CMP_LT        = -1,
    ADIP_CMP_EQ        = 0,
    ADIP_CMP_GT        = 1,
    ADIP_CMP_INCOMPARABLE = 2  /* different units, or the shift overflows */
} adip_cmp_t;

/*
 * Compare a and b. Returns ADIP_CMP_INCOMPARABLE if the units differ, which
 * callers must treat as a rejection rather than as "not less than". A unit
 * mismatch is an engine defect, not a value that happens to be large.
 */
adip_cmp_t adip_num_cmp(adip_num_t a, adip_num_t b);

/* True when lo <= v <= hi. False if any pair is incomparable, so an unmatched
 * unit fails closed. */
bool adip_num_in_range(adip_num_t v, adip_num_t lo, adip_num_t hi);

/*
 * True when v lies on the grid defined by resolution, measuring from lo.
 * A resolution of zero means unconstrained. Used to reject values that are in
 * range but not on a step the mechanism can actually reach.
 */
bool adip_num_on_resolution(adip_num_t v, adip_num_t lo, adip_num_t resolution);

/* Normalise a and b to a common exponent. Returns false on unit mismatch or
 * on an overflow that would make the comparison unsound. */
bool adip_num_align(adip_num_t a, adip_num_t b, int64_t *out_a, int64_t *out_b);

/*
 * Add two quantities exactly, at the finer of the two scales.
 *
 * Returns false on a unit mismatch or if the result will not fit, rather than
 * saturating. An accumulator that silently saturated would stop enforcing the
 * limit it exists to enforce, which is the opposite of failing closed.
 */
bool adip_num_add(adip_num_t a, adip_num_t b, adip_num_t *out);

/* Negate. Used to subtract without a second code path. */
adip_num_t adip_num_neg(adip_num_t a);

#ifdef ADIP_ENABLE_FLOAT
/* Host and development only. Never compiled into a device build. */
double adip_num_to_double(adip_num_t n);
#endif

/*
 * One parameter's setpoint: what a mechanism is commanded to.
 *
 * Not a sample. A sample is something the machine measured; this is what it
 * was told to reach. A heater commanded to 62 C may be measuring 45 C on the
 * way there, and an engine planning the next correction adjusts the command
 * rather than the thermometer.
 *
 * Addressed by position, as signals and events already are: the declaration
 * gives both sides the same ordered list of tools and each tool the same
 * ordered list of parameters, so no names cross the wire.
 */
typedef struct {
    uint16_t   tool;
    uint16_t   parameter;
    adip_num_t value;
} adip_setpoint_t;

/* ---------------------------------------------------------------- outcomes */

/* Invocation lifecycle from PROTOCOL.md 9.2. */
typedef enum {
    ADIP_OUTCOME_INVALID   = 0,
    ADIP_OUTCOME_ACCEPTED  = 1,
    ADIP_OUTCOME_RUNNING   = 2,
    ADIP_OUTCOME_COMPLETED = 3,
    ADIP_OUTCOME_ABORTED   = 4,
    ADIP_OUTCOME_REJECTED  = 5
} adip_outcome_t;

/*
 * Rejection reason codes from PROTOCOL.md 9.2.
 *
 * ADIP_REASON_OUT_OF_RANGE indicates an engine defect: the constraint layer
 * should have clamped before sending. The appliance rejects rather than
 * clamping so that the fault stays visible. See PROTOCOL.md 4.6 requirement 2.
 */
typedef enum {
    ADIP_REASON_NONE                = 0,
    ADIP_REASON_OUT_OF_RANGE        = 1,
    ADIP_REASON_ENVELOPE_VIOLATION  = 2,
    ADIP_REASON_PRECONDITION_FAILED = 3,
    ADIP_REASON_INTERLOCK           = 4,
    ADIP_REASON_BUSY                = 5,
    ADIP_REASON_NOT_AUTHORISED      = 6,
    ADIP_REASON_FAULT               = 7,
    ADIP_REASON_INVARIANT_VIOLATION = 8,
    ADIP_REASON_RATE_LIMITED        = 9,
    ADIP_REASON_RESOURCE_EXHAUSTED  = 10,
    ADIP_REASON_UNKNOWN_TOOL        = 11,
    ADIP_REASON_MALFORMED           = 12,
    ADIP_REASON_UNIT_MISMATCH       = 13
} adip_reason_t;

/* Library return codes. Negative is failure, zero is success. */
typedef enum {
    ADIP_OK              = 0,
    ADIP_PENDING         = 1,   /* work remains, call again */
    ADIP_ERR_ARG         = -1,  /* a caller passed something invalid */
    ADIP_ERR_NO_SPACE    = -2,  /* a caller-supplied buffer is too small */
    ADIP_ERR_MALFORMED   = -3,  /* bytes off the wire did not decode */
    ADIP_ERR_UNSUPPORTED = -4,  /* a declared feature this build cannot honour */
    ADIP_ERR_STATE       = -5   /* called in the wrong order */
} adip_status_t;

/* What a tool handler tells the runtime it did. */
typedef enum {
    ADIP_ACTION_DONE     = 0,  /* finished synchronously */
    ADIP_ACTION_RUNNING  = 1,  /* started; adip_complete() will follow */
    ADIP_ACTION_REJECTED = 2   /* the firmware refuses, with a reason */
} adip_action_t;

/* ------------------------------------------------------------- parameters */

typedef enum {
    ADIP_PARAM_INVALID = 0,
    ADIP_PARAM_NUMBER  = 1,
    ADIP_PARAM_INTEGER = 2,
    ADIP_PARAM_ENUM    = 3,
    ADIP_PARAM_OBJECT  = 4
} adip_param_kind_t;

/*
 * Direction of a mechanism where direction carries domain meaning. The engine
 * must never infer this, so it is declared. See PROTOCOL.md 4.2.
 */
typedef enum {
    ADIP_MONOTONIC_UNSET          = 0,
    ADIP_MONOTONIC_HIGHER_IS_MORE = 1,
    ADIP_MONOTONIC_HIGHER_IS_LESS = 2,
    ADIP_MONOTONIC_PROFILE        = 3  /* resolved by the domain profile */
} adip_monotonic_t;

/* Parameter flags. */
#define ADIP_PARAM_F_OPTIONAL    (1u << 0)
#define ADIP_PARAM_F_HAS_DEFAULT (1u << 1)

/* Tool flags. */
#define ADIP_TOOL_F_REVERSIBLE   (1u << 0)
#define ADIP_TOOL_F_ABORTABLE    (1u << 1)
#define ADIP_TOOL_F_IMPLICIT     (1u << 2)
#define ADIP_TOOL_F_AUTH_ENGINE  (1u << 3)
#define ADIP_TOOL_F_AUTH_OPERATOR (1u << 4)

/* Conformance level from PROTOCOL.md 3.1. Distinct from autonomy level, which
 * is an engine-side configuration and never appears on the device. */
typedef enum {
    ADIP_CONFORMANCE_ADVISORY      = 0,
    ADIP_CONFORMANCE_PARAMETERISED = 1,
    ADIP_CONFORMANCE_AUTONOMOUS    = 2
} adip_conformance_t;

/* ------------------------------------------------------------------ signals */

/*
 * One measurement, waiting to be sent.
 *
 * `t_ms` is the appliance's own clock. The engine is told to treat these as
 * relative, because two devices share no epoch and pretending otherwise would
 * put a made up absolute time into a decision record.
 */
typedef struct {
    uint16_t   signal;
    adip_num_t value;
    uint32_t   t_ms;
} adip_sample_t;

#ifdef __cplusplus
}
#endif

#endif /* ADIP_TYPES_H */
