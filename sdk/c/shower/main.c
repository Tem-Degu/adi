/* A thermostatic shower mixer, as firmware, on a Cortex-M3 over a UART.
 *
 * Written from HOW-TO-PORT-TO-A-BOARD.md and HOW-TO-ADD-A-TOOL.md, in the
 * order those documents set out: declaration first, tables generated from it,
 * the three port functions, the transport, first contact, then a handler.
 * Pairs with examples/declarations/shower.yaml, which declares `base/v1`, so
 * nothing in the engine knows what a shower is and everything it reasons
 * about comes from the knowledge extension beside that file.
 *
 * The board is the Stellaris LM3S6965 that QEMU emulates. UART0 carries the
 * link, so the bytes cross a real peripheral and real framing.
 *
 * Two mechanisms, which is the whole of what this adds to the dehydrator in
 * the directory beside it: a request has to reach the right one of them.
 *
 * What is simulated and what is not. The library, the framing, the CRC, the
 * range and unit checks, the rate limit and the failsafe are the real ones,
 * compiled for the real core. There is no cartridge and no valve, so the
 * firmware models an outlet temperature moving towards the setting it was
 * given and a valve reaching the opening it was given. Every number that
 * reaches either crossed the same checks a product's would.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "adip.h"
#include "adip_generated_sx2.h"

/* ------------------------------------------------------------------ the board
 *
 * Stellaris peripheral registers, from the datasheet. A vendor's own header
 * supplies these; they are written out here so this file needs nothing but a
 * compiler.
 */

#define UART0_BASE      0x4000C000u
#define UART0_DR        (*(volatile uint32_t *)(UART0_BASE + 0x000u))
#define UART0_FR        (*(volatile uint32_t *)(UART0_BASE + 0x018u))
#define UART0_IBRD      (*(volatile uint32_t *)(UART0_BASE + 0x024u))
#define UART0_FBRD      (*(volatile uint32_t *)(UART0_BASE + 0x028u))
#define UART0_LCRH      (*(volatile uint32_t *)(UART0_BASE + 0x02Cu))
#define UART0_CTL       (*(volatile uint32_t *)(UART0_BASE + 0x030u))

#define FR_RXFE         (1u << 4)   /* receive FIFO empty */
#define FR_TXFF         (1u << 5)   /* transmit FIFO full */
#define LCRH_WLEN_8     (3u << 5)
#define LCRH_FEN        (1u << 4)
#define CTL_UARTEN      (1u << 0)
#define CTL_TXE         (1u << 8)
#define CTL_RXE         (1u << 9)

#define SYSTICK_CTRL      (*(volatile uint32_t *)0xE000E010u)
#define SYSTICK_LOAD      (*(volatile uint32_t *)0xE000E014u)
#define SYSTICK_VAL       (*(volatile uint32_t *)0xE000E018u)
#define SYSTICK_ENABLE    (1u << 0)
#define SYSTICK_CLKSRC    (1u << 2)
#define SYSTICK_COUNTFLAG (1u << 16)

/* The core clock this board runs at, for the millisecond counter. A real port
 * reads this from the clock tree it configured.
 *
 * Erring low is the safe direction: the counter then advances more slowly
 * than real time, so the failsafe interval is longer than declared rather
 * than shorter, and a link that is merely slow is not mistaken for a dead
 * one. The guide's warning is about the counter stopping; this is the same
 * concern one step earlier.
 */
#define CORE_HZ         12000000u

/* ----------------------------------------------------------------- the clock
 *
 * SysTick, polled rather than interrupt driven, because the startup file this
 * build shares with the library's test suite installs no SysTick vector.
 * Reading CTRL clears COUNTFLAG, so each set flag is one elapsed reload and
 * is counted once. The main loop runs far faster than the reload, which is
 * what makes polling sufficient here. A loop that could stall for longer than
 * a millisecond needs the interrupt instead, as the guide says: a counter
 * that stops is the one thing not allowed.
 */

static volatile uint32_t millis_now;

static void clock_init(void)
{
    SYSTICK_LOAD = (CORE_HZ / 1000u) - 1u;
    SYSTICK_VAL = 0u;
    SYSTICK_CTRL = SYSTICK_CLKSRC | SYSTICK_ENABLE;
}

static void clock_service(void)
{
    if ((SYSTICK_CTRL & SYSTICK_COUNTFLAG) != 0u) {
        millis_now++;
    }
}

/* ------------------------------------------------------------------- context */

static adip_ctx_t      ctx;
static uint8_t         rx[ADIP_RX_MIN];
static adip_sample_t   batch[ADIP_MAX_BATCH];

/* One entry per numeric parameter this machine reports. The engine may not
 * guess where a mechanism stands, so a machine that keeps this to itself
 * cannot be asked to correct anything. */
static adip_setpoint_t held[ADIP_SX2_TOOL_COUNT * ADIP_SX2_MAX_PARAMS];

/* ------------------------------------------------------------------ the plant
 *
 * What a product reads from a thermistor and a flow sensor. Tenths, because
 * the library carries no floating point and neither does this.
 *
 * Power-on is a shower already running at a comfortable setting, which is the
 * state someone is in when they say anything to it. A product reads these
 * back from its own non-volatile store; the guide is explicit that a value
 * you did not read is not one to assume, and these are the assumption it
 * warns about, made visible.
 */
static int32_t mixer_target_c  = 38;    /* degrees, the cartridge setting   */
static int32_t outlet_c_x10    = 380;   /* tenths, what the thermistor sees */
static int32_t valve_target_ml = 120;   /* ml per second, the valve setting */
static int32_t flow_ml_x1      = 120;   /* ml per second, what is measured  */

/* The one operation in flight, if any. ADI-P allows one at a time on this
 * machine and the declaration says so with max_concurrent: 1. */
static bool     operation_running = false;
static uint32_t pending_invocation;
static uint32_t operation_due_ms;       /* when to give up on it */

/* ------------------------------------------------------------ the three ports
 *
 * The whole platform dependency, as the port guide's step 3 sets out.
 */

uint32_t adip_port_millis(void)
{
    return millis_now;
}

int adip_port_write(const uint8_t *buf, size_t len)
{
    size_t i;

    /* Blocking, which the guide says is acceptable and simplest to get right
     * first. The trap it names is the opposite mistake: returning before the
     * transmitter has started, which leaves the engine waiting for a reply
     * sitting in a buffer and the appliance looking dead rather than broken.
     */
    for (i = 0u; i < len; i++) {
        while ((UART0_FR & FR_TXFF) != 0u) {
        }
        UART0_DR = buf[i];
    }

    return (int)len;
}

void adip_port_safe_state(void)
{
    /* Harmless for a shower is no water, not comfortable water: somebody may
     * be standing under it and the supply temperature is not ours to trust
     * once the link is gone. The declaration calls this
     * valve_closed_mixer_cold and promises it within 500 ms; these are
     * register writes on a product and are immediate.
     */
    valve_target_ml = 0;
    flow_ml_x1 = 0;
    mixer_target_c = 15;

    /* ADI-P 4.5 requires a fault event before or with entry to the safe
     * state, and only the firmware knows what failed. Event index 0 is
     * x.rainline.event.fault, which is its position in `events:`. The
     * generator emits named constants for tools and not for events, so this
     * index is checked by hand against the declaration.
     */
    (void)adip_emit_event(&ctx, 0u, "link_lost", 2u /* critical */);
}

/* ------------------------------------------------------------- the meaning
 *
 * An argument arrives as a mantissa and an exponent. `adip_num_to_double`
 * exists and is host-only, behind ADIP_ENABLE_FLOAT, so a device build needs
 * its own conversion. Every parameter here is declared as an integer with a
 * resolution of one or ten, so the value is exact and this cannot lose
 * anything the declaration permits.
 */
static int32_t num_to_int(adip_num_t n)
{
    int32_t value = n.mantissa;
    int8_t exponent = n.exponent;

    while (exponent > 0) {
        value *= 10;
        exponent--;
    }
    while (exponent < 0) {
        value /= 10;
        exponent++;
    }

    return value;
}

/* ------------------------------------------------------------------ the handler
 *
 * Called once an invocation has passed every check the library can make: the
 * tool exists, the arguments are present and of the right type, each value is
 * inside the declared range and carries the declared unit, no interlock
 * blocks it, the preconditions hold, and the rate limit is not breached. What
 * is left is what only this firmware knows.
 */
adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    adip_num_t value;
    int tool = adip_tool_index(ctx.dev, inv->tool);

    (void)user;

    /* One at a time. The library holds the reply clock, not a queue, and a
     * second operation would overwrite the first one's identifier and leave
     * the engine waiting for a result that can never be matched. */
    if (operation_running) {
        *reason = ADIP_REASON_BUSY;
        return ADIP_ACTION_REJECTED;
    }

    switch (tool) {
        case ADIP_TOOL_SX2_X_RAINLINE_MIXER_SET_TEMPERATURE:
            /* By name, never by position: the arguments arrive in the order
             * the sender encoded them. */
            if (!adip_arg_num(inv, "celsius", &value)) {
                *reason = ADIP_REASON_FAULT;
                return ADIP_ACTION_REJECTED;
            }

            mixer_target_c = num_to_int(value);

            /* Said here because this is where the setting changes. The
             * engine's belief about a value it sent is provisional until the
             * machine confirms what it did with it. */
            (void)adip_report_setpoint(
                &ctx, ADIP_TOOL_SX2_X_RAINLINE_MIXER_SET_TEMPERATURE, 0u,
                value);

            /* The cartridge takes about three seconds, which is what
             * duration_ms_typ says. A handler must not block for it, so the
             * move is started here and completed from the main loop when the
             * outlet actually arrives. A timer expiring is not evidence that
             * it did. */
            operation_running = true;
            pending_invocation = inv->invocation_id;
            operation_due_ms = adip_port_millis() + 8000u;  /* duration_ms_max */
            return ADIP_ACTION_RUNNING;

        case ADIP_TOOL_SX2_X_RAINLINE_VALVE_SET_FLOW:
            if (!adip_arg_num(inv, "ml_per_s", &value)) {
                *reason = ADIP_REASON_FAULT;
                return ADIP_ACTION_REJECTED;
            }

            valve_target_ml = num_to_int(value);

            (void)adip_report_setpoint(
                &ctx, ADIP_TOOL_SX2_X_RAINLINE_VALVE_SET_FLOW, 0u, value);

            operation_running = true;
            pending_invocation = inv->invocation_id;
            operation_due_ms = adip_port_millis() + 3000u;
            return ADIP_ACTION_RUNNING;

        default:
            /* A tool the declaration lists and this firmware has not wired
             * up. Refusing is honest; pretending it worked is not. */
            *reason = ADIP_REASON_FAULT;
            return ADIP_ACTION_REJECTED;
    }
}

/* ------------------------------------------------------------------- the plant
 *
 * Deliberately not derived from the skill file. The skill states that a
 * degree of setting moves the outlet by 0.9 of a degree, which is the
 * manufacturer's estimate and the thing a correction is supposed to test. A
 * simulation built from that number would agree with it by construction.
 */
static void plant_step(void)
{
    int32_t target_x10 = mixer_target_c * 10;

    /* The cartridge, moving about three degrees a second. */
    if (outlet_c_x10 < target_x10) {
        outlet_c_x10 += 3;
        if (outlet_c_x10 > target_x10) {
            outlet_c_x10 = target_x10;
        }
    } else if (outlet_c_x10 > target_x10) {
        outlet_c_x10 -= 3;
        if (outlet_c_x10 < target_x10) {
            outlet_c_x10 = target_x10;
        }
    }

    /* The valve, faster than the cartridge and not instant. */
    if (flow_ml_x1 < valve_target_ml) {
        flow_ml_x1 += 10;
        if (flow_ml_x1 > valve_target_ml) {
            flow_ml_x1 = valve_target_ml;
        }
    } else if (flow_ml_x1 > valve_target_ml) {
        flow_ml_x1 -= 10;
        if (flow_ml_x1 < valve_target_ml) {
            flow_ml_x1 = valve_target_ml;
        }
    }
}

/* Whether the operation in flight has arrived. Reporting the truth means
 * asking the sensor, not the clock. */
static bool operation_arrived(void)
{
    if (!operation_running) {
        return false;
    }

    return outlet_c_x10 == mixer_target_c * 10 && flow_ml_x1 == valve_target_ml;
}

/* ---------------------------------------------------------------------- init */

static void uart_init(void)
{
    uint32_t divisor;

    /* 115200 baud from CORE_HZ. The Stellaris divisor is in 6.6 fixed point,
     * so the integer part is divisor / 64 and the fraction is the remainder.
     */
    divisor = (CORE_HZ * 4u) / 115200u;
    UART0_IBRD = divisor / 64u;
    UART0_FBRD = divisor % 64u;
    UART0_LCRH = LCRH_WLEN_8 | LCRH_FEN;
    UART0_CTL = CTL_UARTEN | CTL_TXE | CTL_RXE;
}

int main(void)
{
    uint32_t last_publish = 0u;
    uint32_t last_plant = 0u;

    clock_init();
    uart_init();

    if (adip_init(&ctx, &adip_device_sx2, rx, sizeof(rx), NULL) != ADIP_OK) {
        return 1;
    }

    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);
    adip_set_setpoint_buffer(&ctx, held,
                             (uint8_t)(sizeof(held) / sizeof(held[0])));

    /* Where each mechanism stands at power-on. An engine that has just
     * connected knows nothing, and without this it has no position to plan a
     * first correction from. */
    (void)adip_report_setpoint(&ctx,
                               ADIP_TOOL_SX2_X_RAINLINE_MIXER_SET_TEMPERATURE,
                               0u, ADIP_NUM(mixer_target_c, 0, ADIP_UNIT_C));
    (void)adip_report_setpoint(&ctx,
                               ADIP_TOOL_SX2_X_RAINLINE_VALVE_SET_FLOW,
                               0u,
                               ADIP_NUM(valve_target_ml, 0, ADIP_UNIT_ML_PER_S));

    /* This machine declares no preconditions and no interlocks, so the loops
     * the guide shows here have nothing to iterate. Both bitmaps default to
     * the safe reading, which for a machine with none of either leaves
     * everything invocable.
     */

    for (;;) {
        uint32_t now;

        clock_service();
        now = millis_now;

        /* Bytes as they arrive, one at a time, which is what a UART gives
         * you. Assembling a frame here is how a first port hangs. */
        while ((UART0_FR & FR_RXFE) == 0u) {
            uint8_t byte = (uint8_t)(UART0_DR & 0xFFu);
            (void)adip_feed(&ctx, &byte, 1u);
        }

        /* Called whether or not bytes arrived, because it services timeouts
         * and the failsafe is one of them. */
        while (adip_poll(&ctx) == ADIP_PENDING) {
        }

        if (operation_running && operation_arrived()) {
            operation_running = false;
            (void)adip_complete(&ctx, pending_invocation,
                                ADIP_OUTCOME_COMPLETED, ADIP_REASON_NONE);
        } else if (operation_running && (int32_t)(now - operation_due_ms) >= 0) {
            /* It did not arrive in the time the declaration promised. Saying
             * so is the honest answer; a timer expiring is not completion. */
            operation_running = false;
            (void)adip_complete(&ctx, pending_invocation, ADIP_OUTCOME_ABORTED,
                                ADIP_REASON_FAULT);
        }

        if ((int32_t)(now - last_plant) >= 100) {
            last_plant = now;
            plant_step();
        }

        /* The declaration says two samples a second, for both signals. The
         * indices are their positions in `signals:`. */
        if ((int32_t)(now - last_publish) >= 500) {
            last_publish = now;
            (void)adip_publish(&ctx, 0u, ADIP_NUM(outlet_c_x10, -1, ADIP_UNIT_C),
                               now);
            (void)adip_publish(&ctx, 1u,
                               ADIP_NUM(flow_ml_x1, 0, ADIP_UNIT_ML_PER_S), now);
            (void)adip_flush_signals(&ctx);
        }
    }
}
