/*
 * example/main.c - A working appliance, in about three hundred lines.
 *
 * This is the thing a firmware engineer copies. It is a complete integration:
 * the three port functions, one handler, and a main loop. Everything below is
 * what a manufacturer writes; everything it calls is libadip.
 *
 * The transport is stdin and stdout, which makes it the local IPC binding of
 * PROTOCOL.md 10 and lets the engine drive it as a child process on any
 * platform. A real machine puts the same bytes on a UART or a BLE
 * characteristic; nothing above the port layer changes.
 *
 * Two honest limitations, so nobody mistakes this for a reference port.
 *
 * The read is blocking, so time driven work only happens when bytes arrive.
 * A real appliance calls adip_poll from a scheduler that runs regardless, and
 * the failsafe depends on that. The failsafe timing is tested properly in
 * test_m5.c, against a clock the test controls, which is the only way to test
 * it without flakiness.
 *
 * The extraction curve is generated as fast as the loop runs rather than over
 * thirty real seconds, because a test that takes thirty seconds per shot is a
 * test nobody runs. The shape and the units are real; only the pace is not.
 *
 * Build:  mingw32-make example
 * Run:    the engine spawns it; see sdk/crates/adi-wire/tests/loopback.rs
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip.h"
#include "adip_generated_ex900.h"

#include <stdio.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

/* ------------------------------------------------------------- port layer */

/*
 * The three functions a manufacturer implements. That is the whole platform
 * dependency.
 */

uint32_t adip_port_millis(void)
{
    /* clock() is portable and monotonic enough for a host example. Firmware
     * uses whatever tick its RTOS or SysTick already provides. */
    return (uint32_t)((clock() * 1000) / CLOCKS_PER_SEC);
}

int adip_port_write(const uint8_t *buf, size_t len)
{
    size_t written = fwrite(buf, 1, len, stdout);

    /* Without the flush the engine waits for a reply sitting in a buffer, and
     * the appliance looks unresponsive. On a UART this is the equivalent of
     * forgetting to start the transmitter. */
    fflush(stdout);

    return (int)written;
}

static int safe_state_entries = 0;

void adip_port_safe_state(void)
{
    safe_state_entries++;

    /* Here a real machine cuts the pump, drops the heater and vents pressure,
     * within the max_time_to_safe_ms its declaration promises, without
     * waiting for anyone's permission.
     *
     * PROTOCOL.md 4.5 requires a fault event before or with entry to the safe
     * state. The library cannot raise it: only the firmware knows what failed
     * and how badly. This is where it belongs.
     *
     * Event 1 of the EX-900 declaration is adi.espresso.event.fault. */
    extern adip_ctx_t ctx;
    (void)adip_emit_event(&ctx, 1u, "link_lost", 2u /* critical */);
}

/* ------------------------------------------------------------ the machine */

/*
 * The state a real appliance would read from sensors and switches. Held in
 * plain variables here because the point is the protocol, not the plumbing.
 */
/* Braced rather than ADIP_NUM, which is a compound literal and so is not a
 * constant expression for an object with static storage. */
static adip_num_t grind_setting = { 24,   0, ADIP_UNIT_STEP };
static adip_num_t brew_temp     = { 920, -1, ADIP_UNIT_C };
static adip_num_t dose_mass     = { 180, -1, ADIP_UNIT_G };
static adip_num_t target_mass   = { 360, -1, ADIP_UNIT_G };

/* Set when a brew is accepted, so the main loop knows to produce a shot. */
static bool   brewing      = false;
static uint32_t brew_id    = 0;

adip_ctx_t ctx;
static uint8_t rx[ADIP_RX_MIN];
static adip_sample_t batch[ADIP_MAX_BATCH];

/* Tool indices, in declaration order. The generated header names the count;
 * a firmware that wants names can compare against the ids in the table. */
enum {
    TOOL_GRIND_SET = 0,
    TOOL_GRIND_PURGE,
    TOOL_DOSE,
    TOOL_SET_TEMP,
    TOOL_BREW_START,
    TOOL_DISPENSE,
    TOOL_EJECT,
    TOOL_SWEETENER
};

/*
 * Read one numeric argument by name.
 *
 * Note what does not happen here: no conversion to floating point. Firmware
 * keeps values as adip_num_t, which is an integer mantissa and exponent, so a
 * comparison against a limit stays exact and a Cortex-M0 with no FPU pays
 * nothing. The doubles further down are in the shot simulation, which stands
 * in for physics rather than for your machine.
 */
static bool arg_num(const adip_invocation_t *inv, const char *name,
                    adip_num_t *out)
{
    uint8_t i;

    for (i = 0; i < inv->arg_count; i++) {
        const adip_param_t *p = &inv->tool->params[inv->args[i].index];
        if (strcmp(p->name, name) == 0 && inv->args[i].is_text == 0u) {
            *out = inv->args[i].num;
            return true;
        }
    }
    return false;
}

/* ------------------------------------------------------------ the handler */

/*
 * Called once an invocation has passed every check libadip can make: the tool
 * exists, the arguments are in range and in the right units, the interlocks
 * are closed, the preconditions hold, and the stateful envelope is satisfied.
 *
 * What is left is what only the firmware knows.
 */
adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    int tool = adip_tool_index(ctx.dev, inv->tool);
    adip_num_t value;

    (void)user;

    switch (tool) {
        case TOOL_GRIND_SET:
            if (arg_num(inv, "setting", &value)) {
                grind_setting = value;
            }
            /* A real collar takes about two seconds; nothing here does, so the
             * result is known immediately. */
            return ADIP_ACTION_DONE;

        case TOOL_SET_TEMP:
            if (arg_num(inv, "temperature", &value)) {
                brew_temp = value;
            }
            return ADIP_ACTION_DONE;

        case TOOL_DOSE:
            if (arg_num(inv, "mass", &value)) {
                dose_mass = value;
            }
            return ADIP_ACTION_DONE;

        case TOOL_BREW_START:
            if (!arg_num(inv, "stop_at_mass", &target_mass)) {
                /* The declaration marks it required, so libadip would have
                 * refused already. Belt and braces, and it costs nothing. */
                *reason = ADIP_REASON_PRECONDITION_FAILED;
                return ADIP_ACTION_REJECTED;
            }

            /*
             * Anything that takes time returns RUNNING and completes later.
             * Blocking here for thirty seconds would stall the whole appliance
             * and blow the 200 ms reply deadline, which is why the API is
             * shaped to make this the natural thing to write.
             */
            brewing = true;
            brew_id = inv->invocation_id;
            return ADIP_ACTION_RUNNING;

        case TOOL_GRIND_PURGE:
        case TOOL_DISPENSE:
        case TOOL_EJECT:
        case TOOL_SWEETENER:
            return ADIP_ACTION_DONE;

        default:
            /* A tool the declaration lists and this firmware has not wired up
             * yet. Refusing is honest; pretending it worked is not. */
            *reason = ADIP_REASON_FAULT;
            return ADIP_ACTION_REJECTED;
    }
}

/* ------------------------------------------------------------ the shot */

static adip_num_t grams(double v)
{
    adip_num_t n;
    n.mantissa = (int32_t)(v * 100.0 + (v < 0 ? -0.5 : 0.5));
    n.exponent = -2;
    n.unit = (uint8_t)ADIP_UNIT_G;
    return n;
}

static adip_num_t seconds(double v)
{
    adip_num_t n;
    n.mantissa = (int32_t)(v * 100.0 + (v < 0 ? -0.5 : 0.5));
    n.exponent = -2;
    n.unit = (uint8_t)ADIP_UNIT_S;
    return n;
}

/*
 * Produce an extraction and report it.
 *
 * The curve is the usual shape: nothing for the first few seconds while the
 * puck saturates, then a rising flow that tails off. First drip is what
 * PROTOCOL.md 7.3 measures shot_time from, and it is reported as an event
 * because the engine cannot see it in the mass trace reliably.
 *
 * Signals go out batched, which is what 10.1 asks for.
 */
static void run_extraction(void)
{
    const double first_drip_s = 4.0;
    const double step_s = 0.5;
    double t = 0.0;
    double mass = 0.0;
    double flow;

    /*
     * From here down is the simulation, and it is the only place this file
     * uses floating point. Stand-in physics, not protocol: a real machine has
     * a pump and a puck instead.
     *
     * Grind and temperature move the shot roughly as the EX-900 skill says
     * they do. Finer, meaning a lower setting, runs slower.
     */
    double setting = adip_num_to_double(grind_setting);
    double temp = adip_num_to_double(brew_temp);
    double stop_at = adip_num_to_double(target_mass);
    double rate = 1.9 - (24.0 - setting) * 0.06 + (temp - 92.0) * 0.02;

    if (rate < 0.4) {
        rate = 0.4;
    }

    (void)adip_emit_event(&ctx, 0u, "first_drip", 0u);

    while (mass < stop_at && t < 90.0) {
        t += step_s;

        if (t >= first_drip_s) {
            /* Rises for a few seconds, then settles. */
            double ramp = (t - first_drip_s) / 3.0;
            flow = rate * (ramp < 1.0 ? ramp : 1.0);
            mass += flow * step_s;
            if (mass > stop_at) {
                mass = stop_at;
            }
        }

        (void)adip_publish(&ctx, 0u, grams(mass), (uint32_t)(t * 1000.0));
        (void)adip_publish(&ctx, 1u, seconds(t), (uint32_t)(t * 1000.0));
    }

    (void)adip_flush_signals(&ctx);

    brewing = false;
    (void)adip_complete(&ctx, brew_id, ADIP_OUTCOME_COMPLETED,
                        ADIP_REASON_NONE);
}

/* -------------------------------------------------------------- main loop */

int main(void)
{
    uint8_t byte;
    uint8_t i;

#ifdef _WIN32
    /* Without this the CRT translates 0x0A into 0x0D 0x0A and corrupts every
     * frame containing it. The first thing to check when a Windows port
     * mysteriously fails its CRC. */
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
#endif

    if (adip_init(&ctx, &adip_device_ex900, rx, sizeof(rx), NULL) != ADIP_OK) {
        return 1;
    }

    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);

    /*
     * Both bitmaps default to the safe reading: nothing satisfied, so nothing
     * is invocable until the firmware says otherwise. A real machine sets
     * these from switches and sensors as they change.
     */
    for (i = 0; i < adip_device_ex900.precondition_count; i++) {
        adip_set_precondition(&ctx, i, true);
    }
    for (i = 0; i < adip_device_ex900.interlock_count; i++) {
        adip_set_interlock(&ctx, i, false);   /* closed, meaning not blocking */
    }

    /*
     * One byte at a time.
     *
     * Not an optimisation to make later: fread asks for the full count and
     * blocks until it has it, so a twenty byte frame would leave the appliance
     * waiting for bytes the engine has no reason to send, with the engine
     * waiting for the reply. Both sides then wait forever.
     *
     * Taking a byte as it arrives is also what firmware does. A UART
     * interrupt hands over one byte, adip_feed takes it, and adip_poll acts
     * once a whole frame is present. The buffering belongs to the library.
     */
    for (;;) {
        int c = fgetc(stdin);

        if (c == EOF) {
            break;      /* the engine closed the link */
        }

        byte = (uint8_t)c;
        if (adip_feed(&ctx, &byte, 1) != ADIP_OK) {
            /* Only reachable by feeding without polling. */
            continue;
        }

        while (adip_poll(&ctx) == ADIP_PENDING) {
            /* Keep going until there is nothing left to dispatch. */
        }

        if (brewing) {
            run_extraction();
        }
    }

    return 0;
}
