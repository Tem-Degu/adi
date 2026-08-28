/*
 * adip_tables.h - Static table layouts describing one appliance.
 *
 * These are the compiled form of a capability declaration. A device never
 * parses YAML or JSON: `adi generate-c` reads the declaration on a host and
 * emits const tables in this shape, which land in flash.
 *
 * See sdk/c/DESIGN.md section 3 for why.
 *
 * Everything here is const and shared. Nothing in this file holds runtime
 * state; that lives in the context struct, which the caller owns.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#ifndef ADIP_TABLES_H
#define ADIP_TABLES_H

#include "adip_types.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Preconditions and interlocks are addressed by bit index, so a device can
 * hold the whole state of both in two words. A declaration needing more than
 * 32 of either is rejected by the generator rather than silently truncated.
 */
#define ADIP_MAX_PRECONDITIONS 32
#define ADIP_MAX_INTERLOCKS    32

/* Sentinel meaning "this interlock blocks every tool", the `["*"]` form. */
#define ADIP_BLOCKS_ALL 0xFFFFFFFFu

/* --------------------------------------------------------------- parameter */

typedef struct {
    const char        *name;        /* parameter name as declared */
    uint8_t            kind;        /* adip_param_kind_t */
    uint8_t            flags;       /* ADIP_PARAM_F_* */
    uint8_t            monotonic;   /* adip_monotonic_t */
    uint8_t            enum_count;

    adip_num_t         min;
    adip_num_t         max;
    adip_num_t         resolution;  /* mantissa 0 means unconstrained */
    adip_num_t         dflt;        /* valid only with ADIP_PARAM_F_HAS_DEFAULT */

    /* Non-NULL only when kind == ADIP_PARAM_ENUM. */
    const char *const *enum_values;
} adip_param_t;

/* -------------------------------------------------------------------- tool */

typedef struct {
    const char         *id;          /* semantic id, e.g. "x.acme.tray.wipe" */
    const char         *name;        /* short name, may be NULL */

    const adip_param_t *params;
    uint8_t             param_count;

    uint16_t            flags;       /* ADIP_TOOL_F_* */

    /* Every precondition this tool requires, as bits into the precondition
     * table. All listed bits must be satisfied for the tool to be invoked. */
    uint32_t            precondition_mask;

    /* Domain variables this tool changes, as indices into the variable table.
     * Used by the engine; carried here so the device can report them. */
    const uint16_t     *effects;
    uint8_t             effect_count;

    uint32_t            duration_ms_typ;
    uint32_t            duration_ms_max;
} adip_tool_t;

/* ------------------------------------------------------------------ signal */

typedef struct {
    const char *id;
    uint8_t     unit;            /* adip_unit_t */
    uint8_t     filtered;        /* already smoothed by the appliance */
    uint16_t    sample_rate_mhz; /* millihertz, so 20 Hz is 20000 */
    adip_num_t  accuracy_abs;    /* mandatory: PROTOCOL.md 4.3 */
    adip_num_t  range_min;
    adip_num_t  range_max;
} adip_signal_t;

/* ------------------------------------------------------- envelope: static */

/* A hard limit on a named parameter, enforced regardless of the tool. */
typedef struct {
    const char *parameter;
    adip_num_t  hard_min;
    adip_num_t  hard_max;
    uint8_t     has_min;
    uint8_t     has_max;
} adip_hard_limit_t;

typedef struct {
    const char *id;
    /* Bitmask of tool indices this blocks, or ADIP_BLOCKS_ALL. */
    uint32_t    blocks_mask;
} adip_interlock_t;

typedef struct {
    const char *expr;  /* the declared text, for diagnostics only */
} adip_precondition_t;

/* ----------------------------------------------------- envelope: stateful */

/*
 * These are the constraints that only exist across a sequence. Individually
 * valid actions can compose into an invalid one, and no per-call check can
 * express that. See PROTOCOL.md 1.1 point 3 and 4.6.
 */

/* One tool's contribution to an accumulating quantity. */
typedef struct {
    uint16_t tool;       /* index into the tool table */
    uint8_t  param;      /* index into that tool's parameter list */
} adip_accum_t;

typedef struct {
    const char         *id;
    const char         *quantity;
    adip_num_t          min;
    adip_num_t          max;
    uint8_t             has_min;
    uint8_t             has_max;

    const adip_accum_t *accumulates;
    uint8_t             accum_count;

    /* Tools that zero the accumulator, by index. */
    const uint16_t     *reset_on;
    uint8_t             reset_count;
} adip_invariant_t;

typedef struct {
    const char     *id;
    const uint16_t *tools;        /* tool indices, or NULL with all_tools set */
    uint8_t         tool_count;
    uint8_t         all_tools;    /* the ["*"] form */

    uint32_t        max_invocations;  /* 0 means unset */
    uint32_t        window_ms;        /* 0 means unset */
    uint32_t        min_interval_ms;  /* 0 means unset */
    uint16_t        max_duty_centipct;/* 0 means unset; 10000 is 100% */
} adip_rate_limit_t;

typedef struct {
    const char         *id;
    uint8_t             unit;
    adip_num_t          capacity;
    uint8_t             has_capacity;
    const adip_accum_t *consumed_by;
    uint8_t             consume_count;
} adip_resource_t;

/* ---------------------------------------------------------------- failsafe */

typedef struct {
    uint32_t max_time_to_safe_ms;
    uint8_t  on_control_timeout;  /* adip_failsafe_action_t */
    uint8_t  on_signal_loss;
    uint8_t  on_fault;
} adip_failsafe_t;

typedef enum {
    ADIP_FAILSAFE_ABORT_TO_SAFE = 0,
    ADIP_FAILSAFE_HOLD          = 1
} adip_failsafe_action_t;

/* ------------------------------------------------------------------ device */

/*
 * The whole compiled declaration. One const instance per firmware image,
 * emitted by the generator as `adip_device_<basename>`.
 */
typedef struct {
    const char *adip_version;
    const char *domain;
    const char *domain_profile;
    uint8_t     conformance_level;   /* adip_conformance_t */

    const char *vendor;
    const char *model;
    const char *hardware_revision;   /* may be NULL */
    const char *firmware_version;
    const char *declaration_id;

    uint32_t    heartbeat_interval_ms;
    uint32_t    control_timeout_ms;
    uint8_t     max_concurrent;

    const adip_tool_t         *tools;
    uint16_t                   tool_count;
    /* Domain variable names, indexed by the `effects` arrays on each tool. */
    const char *const         *variables;
    uint16_t                   variable_count;
    const adip_signal_t       *signals;
    uint16_t                   signal_count;
    const adip_precondition_t *preconditions;
    uint8_t                    precondition_count;
    const adip_interlock_t    *interlocks;
    uint8_t                    interlock_count;
    const adip_hard_limit_t   *limits;
    uint8_t                    limit_count;
    const adip_invariant_t    *invariants;
    uint8_t                    invariant_count;
    const adip_rate_limit_t   *rate_limits;
    uint8_t                    rate_limit_count;
    const adip_resource_t     *resources;
    uint8_t                    resource_count;

    adip_failsafe_t            failsafe;

    /*
     * True when the declaration carries invariants or rate limits.
     *
     * The engine refuses autonomy level D2 against a machine without these
     * (PROTOCOL.md 3.3.2). The device does not enforce that, since autonomy is
     * an engine-side setting, but it reports the fact so the engine can check
     * it against a device that never sent a full declaration.
     */
    uint8_t                    has_stateful_envelope;
} adip_device_t;

/* Look up a tool by semantic id. Returns NULL when absent, which the caller
 * must treat as ADIP_REASON_UNKNOWN_TOOL. */
const adip_tool_t *adip_find_tool(const adip_device_t *dev, const char *id);

/* Index of a tool, or -1. Indices are what the stateful tables reference. */
int adip_tool_index(const adip_device_t *dev, const adip_tool_t *tool);

/* Index of a named parameter within a tool, or -1. */
int adip_param_index(const adip_tool_t *tool, const char *name);

#ifdef __cplusplus
}
#endif

#endif /* ADIP_TABLES_H */
