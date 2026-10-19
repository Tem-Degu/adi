/* A food dehydrator, as firmware, on a Cortex-M3 over a UART.
 *
 * Written by following HOW-TO-PORT-TO-A-BOARD.md as a manufacturer would:
 * declaration first, tables generated from it, the three port functions, the
 * transport, then first contact. Pairs with
 * examples/declarations/dehydrator.yaml and declares `base/v1`, so nothing in
 * the engine knows what drying is and everything it reasons about comes from
 * the knowledge extension beside that file.
 *
 * The board is the Stellaris LM3S6965 that QEMU emulates, chosen because the
 * library's own test suite already runs on it. UART0 carries the link, so the
 * bytes cross a real peripheral and real framing rather than a pipe.
 *
 * One tool and one outcome, deliberately. The question this answers is whether
 * the engine can reason about a machine nobody wrote a profile for, and a
 * second mechanism would not make the answer clearer.
 *
 * What is simulated and what is not. The library, the framing, the CRC, the
 * range and unit checks, the interlock and precondition gates and the failsafe
 * are the real ones, compiled for the real core. The heating element and the
 * moisture probe are not: there is no element, so the firmware models one
 * settling towards the setting it was given. Every number that reaches the
 * element crossed the same checks a product's would.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "adip.h"
#include "adip_generated_dh20.h"

/* ------------------------------------------------------------------ the board
 *
 * Stellaris peripheral registers. A vendor's own header supplies these; they
 * are written out here so that this file needs nothing but a compiler.
 */

#define SYSCTL_RCGC1    (*(volatile uint32_t *)0x400FE104u)
#define RCGC1_UART0     (1u << 0)

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

#define SYSTICK_CTRL    (*(volatile uint32_t *)0xE000E010u)
#define SYSTICK_LOAD    (*(volatile uint32_t *)0xE000E014u)
#define SYSTICK_VAL     (*(volatile uint32_t *)0xE000E018u)
#define SYSTICK_ENABLE   (1u << 0)
#define SYSTICK_CLKSRC   (1u << 2)
#define SYSTICK_COUNTFLAG (1u << 16)

/* The core clock this board is assumed to run at, for the millisecond
 * counter below. A real port reads this from the clock tree it configured.
 *
 * Erring low is the safe direction. The counter then advances more slowly than
 * real time, so the failsafe interval is longer than declared rather than
 * shorter, and a link that is merely slow is not mistaken for a dead one. The
 * reverse would have the appliance drop to its safe state while the engine is
 * still talking to it.
 */
#define CORE_HZ         12000000u

/* ----------------------------------------------------------------- the clock
 *
 * SysTick, polled rather than interrupt driven, because the startup file this
 * build shares with the test suite installs no SysTick vector. Reading CTRL
 * clears COUNTFLAG, so each set flag is one elapsed reload and is counted
 * once. The main loop below runs far faster than the reload, which is what
 * makes polling sufficient; a loop that could stall for longer than a
 * millisecond needs the interrupt instead.
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

static adip_ctx_t    ctx;
static uint8_t       rx[ADIP_RX_MIN];
static adip_sample_t batch[ADIP_MAX_BATCH];

/* ------------------------------------------------------------ the three ports
 *
 * The whole platform dependency, as HOW-TO-PORT-TO-A-BOARD.md step 3 sets out.
 */

uint32_t adip_port_millis(void)
{
    return millis_now;
}

int adip_port_write(const uint8_t *buf, size_t len)
{
    size_t i;

    /* Blocking, which the guide recommends getting right before moving to
     * interrupts or DMA. The trap it names is the one that matters here: a
     * transmitter that was never enabled leaves the engine waiting for a reply
     * that is sitting in a FIFO, and the appliance looks dead rather than
     * broken. UART0_CTL below enables TXE for that reason.
     */
    for (i = 0u; i < len; i++) {
        while ((UART0_FR & FR_TXFF) != 0u) {
        }
        UART0_DR = buf[i];
    }
    return (int)len;
}

/* Set by the handler, read by the main loop. */
static bool     element_moving;
static uint32_t element_due_ms;
static uint32_t pending_invocation;
static int32_t  element_target_c = 40;   /* where the element was last told to go */
static int32_t  element_now_c = 20;      /* where it actually is */

void adip_port_safe_state(void)
{
    /* Now, without asking anyone, and inside the max_time_to_safe_ms the
     * declaration promises. The element is the hazard, so it goes first; the
     * fan is what carries the residual heat out, so it stays on, which is what
     * `safe_state: element_off_fan_running` means.
     */
    element_target_c = 0;
    element_moving = false;

    /* ADI-P 4.5 requires a fault event before or with entry to the safe state,
     * and only the firmware knows what failed. Event index 0 is
     * x.dryline.event.fault: the index is the event's position in `events:`,
     * and the generator emits named constants for tools, preconditions and
     * interlocks but not for events, so this one is tracked by hand.
     */
    (void)adip_emit_event(&ctx, 0u, "link_lost", 2u);
}

/* ------------------------------------------------------------------ the tool */

/* An ADI-P number as a whole integer.
 *
 * Written here because the library offers no integer conversion.
 * `adip_num_to_double` exists and is declared inside `#ifdef
 * ADIP_ENABLE_FLOAT`, described in adip_types.h as host and development only
 * and never compiled into a device build, which is correct: there is no FPU on
 * an M0 or an M3 and a range check against a safety limit has to be exact.
 *
 * The consequence is that the handler in the example appliance and in the
 * README, which reads its argument with `adip_num_to_double`, does not compile
 * for a device. That example is a host build and defines the macro. A
 * manufacturer following it onto a board arrives here.
 *
 * `celsius` is declared as an integer with a resolution of 1, so in practice
 * the exponent is zero and this is a copy. It is written out anyway, because a
 * handler that assumes the exponent it was sent is a handler that silently
 * misreads a declaration someone later edits.
 */
static int32_t num_to_int(adip_num_t n)
{
    int32_t v = n.mantissa;
    int8_t  e = n.exponent;

    while (e > 0 && v > -214748364 && v < 214748364) {
        v *= 10;
        e--;
    }
    while (e < 0) {
        v /= 10;
        e++;
    }
    return v;
}

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason,
                             void *user)
{
    adip_num_t celsius;

    (void)user;
    (void)reason;

    if (adip_tool_index(ctx.dev, inv->tool)
            == ADIP_TOOL_DH20_X_DRYLINE_HEATER_SET_TEMPERATURE
        && adip_arg_num(inv, "celsius", &celsius)) {

        /* By the time this runs the library has confirmed the tool exists, the
         * argument is present, in range and in the declared unit, the
         * precondition holds and no interlock blocks. None of that is repeated
         * here.
         */
        element_target_c = num_to_int(celsius);

        /* The element takes about two minutes to settle, which is what
         * duration_ms_typ says. A handler must not block for it, so the move
         * is started and completed from the main loop. Shortened here because
         * nothing is actually warming up.
         */
        element_moving = true;
        element_due_ms = adip_port_millis() + 1500u;
        pending_invocation = inv->invocation_id;

        /* While the element moves the machine is no longer idle, so the
         * precondition stops being satisfied and a second call is refused by
         * the library rather than by this handler.
         */
        adip_set_precondition(&ctx, ADIP_PRECOND_DH20_STATE____IDLE, false);

        return ADIP_ACTION_RUNNING;
    }

    /* Nothing else is declared, so nothing else can arrive. */
    return ADIP_ACTION_DONE;
}

/* ------------------------------------------------------------------ the loop */

static void uart_init(void)
{
    uint32_t divisor;

    SYSCTL_RCGC1 |= RCGC1_UART0;

    UART0_CTL = 0u;

    /* 115200 8N1. The divisor is the core clock over sixteen times the baud,
     * in 6.6 fixed point.
     */
    divisor = (CORE_HZ * 4u) / 115200u;
    UART0_IBRD = divisor / 64u;
    UART0_FBRD = divisor % 64u;
    UART0_LCRH = LCRH_WLEN_8 | LCRH_FEN;
    UART0_CTL = CTL_UARTEN | CTL_TXE | CTL_RXE;
}

/* The element creeping towards where it was told to go, and the load drying as
 * it does. Stands in for a thermistor and a moisture probe.
 *
 * Deliberately not derived from the skill file. The skill states that a degree
 * of air temperature moves moisture by 0.4 of a percent, which is the
 * manufacturer's estimate and the thing a cycle is supposed to test. A
 * simulation built from that number would agree with it by construction, and
 * every correction would appear to work for a reason that proves nothing.
 */
static int32_t moisture_pct_x10 = 420;    /* 42.0 %, a wet load */

static void plant_step(void)
{
    if (element_now_c < element_target_c) {
        element_now_c++;
    } else if (element_now_c > element_target_c) {
        element_now_c--;
    }

    /* Warmer air dries faster, and nothing dries below the ambient floor. The
     * shape is a decay towards a floor set by temperature, not a line.
     */
    if (element_now_c > 25) {
        int32_t floor_x10 = 180 - (element_now_c - 25) * 2;
        if (floor_x10 < 30) {
            floor_x10 = 30;
        }
        if (moisture_pct_x10 > floor_x10) {
            moisture_pct_x10 -= 1 + (element_now_c - 25) / 20;
        }
    }
}

int main(void)
{
    uint8_t i;
    uint32_t last_publish = 0u;
    uint32_t last_plant = 0u;

    clock_init();
    uart_init();

    if (adip_init(&ctx, &adip_device_dh20, rx, sizeof(rx), NULL) != ADIP_OK) {
        return 1;
    }
    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);

    /* Both bitmaps default to the safe reading, so nothing is invocable until
     * the firmware says otherwise. Note the sense of each: a precondition is
     * passed whether it is satisfied, an interlock whether it is open, and an
     * open interlock blocks. The tray is latched, so the interlock is not
     * open.
     */
    for (i = 0u; i < adip_device_dh20.precondition_count; i++) {
        adip_set_precondition(&ctx, i, true);
    }
    for (i = 0u; i < adip_device_dh20.interlock_count; i++) {
        adip_set_interlock(&ctx, i, false);
    }

    for (;;) {
        uint32_t now;

        clock_service();
        now = adip_port_millis();

        /* One byte at a time, as it arrives. Assembling a frame here would
         * block on bytes the engine has no reason to send while the engine
         * waits for a reply, and both sides would wait forever.
         */
        while ((UART0_FR & FR_RXFE) == 0u) {
            uint8_t byte = (uint8_t)(UART0_DR & 0xFFu);
            (void)adip_feed(&ctx, &byte, 1u);
        }

        /* Called whether or not bytes arrived, because it services timeouts
         * and the failsafe is one of them.
         */
        while (adip_poll(&ctx) == ADIP_PENDING) {
        }

        /* The element finished moving. Reporting completion is the firmware's
         * job and an expired timer is not evidence of it; here the move is the
         * simulation, so its own clock is the evidence there is.
         */
        if (element_moving && (int32_t)(now - element_due_ms) >= 0) {
            element_moving = false;
            adip_set_precondition(&ctx, ADIP_PRECOND_DH20_STATE____IDLE, true);
            (void)adip_complete(&ctx, pending_invocation,
                                ADIP_OUTCOME_COMPLETED, ADIP_REASON_NONE);
        }

        if ((int32_t)(now - last_plant) >= 100) {
            last_plant = now;
            plant_step();
        }

        /* The declaration says two samples a second. */
        if ((int32_t)(now - last_publish) >= 500) {
            last_publish = now;
            (void)adip_publish(&ctx, 0u,
                               ADIP_NUM(moisture_pct_x10, -1, ADIP_UNIT_PCT),
                               now);
            (void)adip_flush_signals(&ctx);
        }
    }
}
