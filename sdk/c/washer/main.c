/* A washing machine, as firmware, on a Cortex-M3 over a UART.
 *
 * Written from HOW-TO-PORT-TO-A-BOARD.md and HOW-TO-ADD-A-TOOL.md in the order
 * they set out. Pairs with examples/declarations/washer.yaml, which declares
 * `base/v1`, so nothing in the engine knows what washing is and everything it
 * reasons about comes from the knowledge extension beside that file.
 *
 * What this adds to the machines beside it is an interlock. The door must be
 * latched, nothing is invocable while it is not, and the refusal comes from
 * the appliance rather than from the engine being careful. Opening the door
 * during an operation aborts it and raises an event, which is what ADI-P 9.1
 * requires of a physical override.
 *
 * Two serial ports. UART0 carries the protocol. UART1 stands in for the door
 * latch switch, so a person testing this can open and shut the door without a
 * machine in front of them: send `o` to open it and `c` to shut it. On a
 * product that line is a switch on a pin, and the only thing this arrangement
 * changes is where the firmware reads it from.
 *
 * What is simulated and what is not. The library, the framing, the CRC, the
 * range and unit checks, the interlock gate, the rate limit and the failsafe
 * are the real ones, compiled for the real core. There is no heater, no drum
 * and no load cell, so the firmware models water warming towards the setting
 * it was given and a load giving up water as the drum spins.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "adip.h"
#include "adip_generated_nw8.h"

/* ------------------------------------------------------------------ the board
 *
 * Stellaris peripheral registers, from the datasheet. A vendor's own header
 * supplies these; they are written out here so this file needs nothing but a
 * compiler.
 */

#define SYSCTL_RCGC1    (*(volatile uint32_t *)0x400FE104u)
#define RCGC1_UART0     (1u << 0)
#define RCGC1_UART1     (1u << 1)

#define UART0_BASE      0x4000C000u
#define UART1_BASE      0x4000D000u

#define UART_DR(base)   (*(volatile uint32_t *)((base) + 0x000u))
#define UART_FR(base)   (*(volatile uint32_t *)((base) + 0x018u))
#define UART_IBRD(base) (*(volatile uint32_t *)((base) + 0x024u))
#define UART_FBRD(base) (*(volatile uint32_t *)((base) + 0x028u))
#define UART_LCRH(base) (*(volatile uint32_t *)((base) + 0x02Cu))
#define UART_CTL(base)  (*(volatile uint32_t *)((base) + 0x030u))

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

/* The core clock this board runs at. Erring low is the safe direction: the
 * counter then advances more slowly than real time, so the failsafe interval
 * is longer than declared rather than shorter.
 */
#define CORE_HZ         12000000u

/* ----------------------------------------------------------------- the clock
 *
 * SysTick, polled, because the startup file this build shares with the
 * library's test suite installs no SysTick vector. Reading CTRL clears
 * COUNTFLAG, so each set flag is one elapsed reload and is counted once.
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
static adip_setpoint_t held[ADIP_NW8_TOOL_COUNT * ADIP_NW8_MAX_PARAMS];

/* ------------------------------------------------------------------ the plant
 *
 * Tenths where a tenth matters, because the library carries no floating point
 * and neither does this. A product reads these from a thermistor, a load cell
 * and a tachometer, and reads the settings back from its own store.
 */
static int32_t wash_target_c   = 40;    /* the setting the element holds     */
static int32_t drum_c_x10      = 200;   /* what the thermistor sees, tenths  */
static int32_t spin_target_pct = 0;     /* the setting the drum holds        */
static int32_t spin_now_pct    = 0;     /* what the tachometer sees          */
static int32_t moisture_pct    = 90;    /* a load straight out of the wash   */
static int32_t load_g          = 4200;  /* telemetry, a four kilo load       */

/* The door latch. Shut at power on, because a machine someone is talking to
 * is a machine with the door shut. */
static bool door_open = false;

/* The one operation in flight, if any. max_concurrent is 1. */
static bool     operation_running = false;
static uint32_t pending_invocation;
static uint32_t operation_due_ms;

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

    for (i = 0u; i < len; i++) {
        while ((UART_FR(UART0_BASE) & FR_TXFF) != 0u) {
        }
        UART_DR(UART0_BASE) = buf[i];
    }

    return (int)len;
}

void adip_port_safe_state(void)
{
    /* Harmless for a washing machine is a cold element, a still drum and shut
     * valves, which is what the declaration calls
     * heater_off_drum_stopped_valves_shut and promises within a second. These
     * are register writes on a product.
     */
    wash_target_c = 20;
    spin_target_pct = 0;
    spin_now_pct = 0;

    /* ADI-P 4.5 requires a fault event before or with entry to the safe
     * state, and only the firmware knows what failed. Event index 0 is
     * x.northwash.event.fault, which is its position in `events:`. The
     * generator emits named constants for tools and interlocks and not for
     * events, so this index is checked by hand against the declaration.
     */
    (void)adip_emit_event(&ctx, 0u, "link_lost", 2u /* critical */);
}

/* ------------------------------------------------------------------- the door
 *
 * UART1 stands in for the latch switch. On a product this function reads a
 * pin; nothing else about the firmware changes.
 */
static void door_service(void)
{
    while ((UART_FR(UART1_BASE) & FR_RXFE) == 0u) {
        uint8_t key = (uint8_t)(UART_DR(UART1_BASE) & 0xFFu);
        bool was_open = door_open;

        /* Echoed so that somebody driving this by hand can see the panel
         * received the keypress, as a latch switch would be visible on a
         * machine. */
        while ((UART_FR(UART1_BASE) & FR_TXFF) != 0u) {
        }
        UART_DR(UART1_BASE) = key;

        if (key == (uint8_t)'o') {
            door_open = true;
        } else if (key == (uint8_t)'c') {
            door_open = false;
        } else {
            continue;
        }

        if (door_open == was_open) {
            continue;
        }

        /* An interlock is passed whether it is open, and an open interlock
         * blocks. Getting this backwards gives a machine that runs with the
         * door up, which is why the guide says it twice. */
        adip_set_interlock(&ctx, ADIP_INTERLOCK_NW8_DOOR_CLOSED, door_open);

        if (!door_open) {
            continue;
        }

        /* ADI-P 9.1: an operator may take control by physical means at any
         * moment, that MUST abort an engine-initiated operation, and it MUST
         * emit an event. Opening the door is that moment. */
        spin_target_pct = 0;
        spin_now_pct = 0;

        (void)adip_emit_event(&ctx, 0u, "door_opened", 1u /* error */);

        if (operation_running) {
            operation_running = false;
            (void)adip_complete(&ctx, pending_invocation, ADIP_OUTCOME_ABORTED,
                                ADIP_REASON_INTERLOCK);
        }
    }
}

/* ------------------------------------------------------------- the meaning
 *
 * An argument arrives as a mantissa and an exponent. `adip_num_to_double` is
 * host-only, behind ADIP_ENABLE_FLOAT, so a device build needs its own
 * conversion. Every parameter here is declared as an integer, so the value is
 * exact and this cannot lose anything the declaration permits.
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
 * in the declared range and carries the declared unit, no interlock blocks
 * it, and the rate limit is not breached. The door is one of those checks and
 * is not repeated here.
 */
adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    adip_num_t value;
    int tool = adip_tool_index(ctx.dev, inv->tool);

    (void)user;

    if (operation_running) {
        *reason = ADIP_REASON_BUSY;
        return ADIP_ACTION_REJECTED;
    }

    switch (tool) {
        case ADIP_TOOL_NW8_X_NORTHWASH_WASH_SET_TEMPERATURE:
            /* By name, never by position. */
            if (!adip_arg_num(inv, "celsius", &value)) {
                *reason = ADIP_REASON_FAULT;
                return ADIP_ACTION_REJECTED;
            }

            wash_target_c = num_to_int(value);

            /* Said here because this is where the setting changes. The
             * engine's belief about a value it sent is provisional until the
             * machine confirms what it did with it. */
            (void)adip_report_setpoint(
                &ctx, ADIP_TOOL_NW8_X_NORTHWASH_WASH_SET_TEMPERATURE, 0u,
                value);

            /* Water takes minutes to heat, which is what duration_ms_typ
             * says. A handler must not block for it, so the move is started
             * here and completed from the main loop when the thermistor
             * actually arrives. A timer expiring is not evidence that it did.
             */
            operation_running = true;
            pending_invocation = inv->invocation_id;
            operation_due_ms = adip_port_millis() + 600000u;
            return ADIP_ACTION_RUNNING;

        case ADIP_TOOL_NW8_X_NORTHWASH_SPIN_SET_SPEED:
            if (!adip_arg_num(inv, "pct", &value)) {
                *reason = ADIP_REASON_FAULT;
                return ADIP_ACTION_REJECTED;
            }

            spin_target_pct = num_to_int(value);

            (void)adip_report_setpoint(
                &ctx, ADIP_TOOL_NW8_X_NORTHWASH_SPIN_SET_SPEED, 0u, value);

            operation_running = true;
            pending_invocation = inv->invocation_id;
            operation_due_ms = adip_port_millis() + 30000u;
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
 * percent of spin speed moves residual moisture by 0.45 of a percent, which
 * is the manufacturer's estimate and the thing a correction is supposed to
 * test. A simulation built from that number would agree with it by
 * construction.
 */
static void plant_step(void)
{
    int32_t target_x10 = wash_target_c * 10;

    /* The element, held off while the door is unlatched. */
    if (!door_open) {
        if (drum_c_x10 < target_x10) {
            drum_c_x10 += 2;
            if (drum_c_x10 > target_x10) {
                drum_c_x10 = target_x10;
            }
        } else if (drum_c_x10 > target_x10) {
            drum_c_x10 -= 1;
            if (drum_c_x10 < target_x10) {
                drum_c_x10 = target_x10;
            }
        }
    }

    /* The drum reaching the speed it was asked for. */
    if (spin_now_pct < spin_target_pct) {
        spin_now_pct += 10;
        if (spin_now_pct > spin_target_pct) {
            spin_now_pct = spin_target_pct;
        }
    } else if (spin_now_pct > spin_target_pct) {
        spin_now_pct -= 10;
        if (spin_now_pct < spin_target_pct) {
            spin_now_pct = spin_target_pct;
        }
    }

    /* Water leaves the load while the drum turns, down to a floor set by how
     * fast it is turning. Nothing dries below that floor however long it
     * spins, and nothing gets wetter on its own.
     */
    if (spin_now_pct > 0) {
        int32_t floor_pct = 90 - (spin_now_pct / 2);

        if (moisture_pct > floor_pct) {
            moisture_pct -= 1;
        }
    }
}

/* Whether the operation in flight has arrived. Reporting the truth means
 * asking the sensor, not the clock. */
static bool operation_arrived(void)
{
    return drum_c_x10 == wash_target_c * 10 && spin_now_pct == spin_target_pct;
}

/* ---------------------------------------------------------------------- init */

static void uart_init(uint32_t base)
{
    uint32_t divisor;

    /* 115200 baud from CORE_HZ. The Stellaris divisor is in 6.6 fixed point,
     * so the integer part is divisor / 64 and the fraction is the remainder.
     */
    divisor = (CORE_HZ * 4u) / 115200u;
    UART_IBRD(base) = divisor / 64u;
    UART_FBRD(base) = divisor % 64u;
    UART_LCRH(base) = LCRH_WLEN_8 | LCRH_FEN;
    UART_CTL(base) = CTL_UARTEN | CTL_TXE | CTL_RXE;
}

int main(void)
{
    uint8_t i;
    uint32_t last_publish = 0u;
    uint32_t last_plant = 0u;

    clock_init();
    SYSCTL_RCGC1 |= RCGC1_UART0 | RCGC1_UART1;
    uart_init(UART0_BASE);
    uart_init(UART1_BASE);

    if (adip_init(&ctx, &adip_device_nw8, rx, sizeof(rx), NULL) != ADIP_OK) {
        return 1;
    }

    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);
    adip_set_setpoint_buffer(&ctx, held,
                             (uint8_t)(sizeof(held) / sizeof(held[0])));

    /* Where each mechanism stands at power-on. An engine that has just
     * connected knows nothing, and without this it has no position to plan a
     * first correction from. */
    (void)adip_report_setpoint(&ctx,
                               ADIP_TOOL_NW8_X_NORTHWASH_WASH_SET_TEMPERATURE,
                               0u, ADIP_NUM(wash_target_c, 0, ADIP_UNIT_C));
    (void)adip_report_setpoint(&ctx,
                               ADIP_TOOL_NW8_X_NORTHWASH_SPIN_SET_SPEED,
                               0u, ADIP_NUM(spin_target_pct, 0, ADIP_UNIT_PCT));

    /* Both bitmaps default to the safe reading, so nothing is invocable until
     * the firmware says otherwise. This machine has one interlock and no
     * preconditions, and the door is shut, so it is set from the latch here
     * and from UART1 as it changes.
     */
    for (i = 0; i < adip_device_nw8.interlock_count; i++) {
        adip_set_interlock(&ctx, i, door_open);
    }

    for (;;) {
        uint32_t now;

        clock_service();
        now = millis_now;

        door_service();

        /* Bytes as they arrive, one at a time, which is what a UART gives
         * you. Assembling a frame here is how a first port hangs. */
        while ((UART_FR(UART0_BASE) & FR_RXFE) == 0u) {
            uint8_t byte = (uint8_t)(UART_DR(UART0_BASE) & 0xFFu);
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

        /* The declaration says two samples a second, for all three signals.
         * The indices are their positions in `signals:`. */
        if ((int32_t)(now - last_publish) >= 500) {
            last_publish = now;
            (void)adip_publish(&ctx, 0u, ADIP_NUM(drum_c_x10, -1, ADIP_UNIT_C),
                               now);
            (void)adip_publish(&ctx, 1u,
                               ADIP_NUM(moisture_pct, 0, ADIP_UNIT_PCT), now);
            (void)adip_publish(&ctx, 2u, ADIP_NUM(load_g, 0, ADIP_UNIT_G), now);
            (void)adip_flush_signals(&ctx);
        }
    }
}
