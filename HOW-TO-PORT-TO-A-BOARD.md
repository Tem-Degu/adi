# How to port to a board

**A guide for appliance firmware engineers. ADI-P 0.2.**

This document covers getting `libadip` running on your hardware and talking to
an engine for the first time. It assumes no prior knowledge of ADI.

It stops at the first heartbeat. Making a mechanism callable is the next job and
is covered by [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md), which assumes
you have finished here.

**When you finish, you will have** a board that connects, identifies itself,
answers a heartbeat, reports what it is set to, and enters a safe state by
itself when the link dies. No mechanism is callable yet; that is the next
guide.

The specification is [`PROTOCOL.md`](./PROTOCOL.md). The library's own design
notes are [`sdk/c/DESIGN.md`](./sdk/c/DESIGN.md). This guide is the practical
path from an empty project to a working link.

---

## What you are building

Two boards, or two processes on one board.

```
  your appliance MCU                      the module
  ┌────────────────────┐                  ┌────────────────────┐
  │ your firmware      │                  │ the engine         │
  │   ┌──────────────┐ │   invocations    │                    │
  │   │ libadip      │◄┼──────────────────┼─ reasoning         │
  │   │ + your tables│─┼──────────────────┼─►knowledge         │
  │   └──────────────┘ │   responses,     │  models            │
  │ pins, sensors      │   signals        │                    │
  └────────────────────┘                  └────────────────────┘
```

`libadip` is freestanding C99. No allocation, no recursion, no floating point,
no library calls beyond `memcpy` and friends. It does not own the wire, it does
not own a thread, and it does not own a clock. You hand it bytes and call it
from your main loop.

Your side of the work is four things: a declaration, three port functions, a
transport, and one handler. The appliance also reports what it is set to,
which is part of the main loop and of each handler. The first three are this document. The handler is
the next one.

---

## Step 1: Write a minimal declaration

The declaration is the single source of what your machine can do and what its
limits are. The generated tables come from it, so it exists before any C.

Start with the smallest thing that can connect. One tool, one signal, one
event. You are proving the link, not describing the product.

```yaml
adip_version: "0.2"
domain: "espresso"
domain_profile: "espresso/v1"
conformance_level: advisory

device:
  vendor: "Acme"
  model: "CM-1"
  firmware_version: "0.1.0"
  declaration_id: "sha256:replace-with-your-own"

session:
  max_concurrent: 1
  heartbeat_interval_ms: 1000
  control_timeout_ms: 500

signals:
  - id: "adi.espresso.signal.mass"
    unit: g
    sample_rate_hz: 20
    accuracy: { abs: 0.1, unit: g }
    # The outcome this sensor measures, where it measures one. A correction is
    # the difference between where an outcome is and where it should be, so a
    # reading the engine cannot place leaves it with a target and no distance.
    # Leave it out on a sensor that measures nothing anyone states a target on.
    measures: "yield_mass"

tools:
  - id: "adi.espresso.grind.set_setting"
    name: "set_grind"
    parameters:
      setting:
        type: integer
        unit: step
        min: 1
        max: 40
        resolution: 1
        monotonic: profile_defined
    effects: ["grind_setting"]
    reversible: true

events:
  - id: "adi.espresso.event.fault"
    payload:
      code: { type: string }
      severity: { type: enum, values: ["warning", "error", "critical"] }
      recoverable: { type: boolean }

envelope:
  enforced_by: both
  independent_of_engine: true

  limits:
    - { parameter: "setting", unit: step, hard_max: 40 }

  interlocks:
    - { id: "service_door_closed", blocks: ["*"] }

  failsafe:
    on_control_timeout: "abort_to_safe"
    on_signal_loss: "abort_to_safe"
    on_fault: "abort_to_safe"
    safe_state: "no_heat_no_pressure_no_motion"
    max_time_to_safe_ms: 200
```

That declaration validates. The rest of this section is the four parts of it
worth understanding before you copy it.

**`max_time_to_safe_ms` is a promise about your hardware.** It is how long your
machine may take to reach a safe state once the link is lost. If your pump
takes 400 ms to stop, 200 is a lie, and it is the kind of lie nothing will
catch until something goes wrong. Measure it.

**`control_timeout_ms` bounds how long the engine waits for your reply.** Your
handler must answer within it, which is why handlers do not block.

**`conformance_level: advisory` is the right place to start,** and it is not the
same thing as the autonomy level. The conformance level is what your machine
claims to support, and the profile decides what each one requires: declaring
`parameterised` against the espresso profile obliges you to provide
`adi.espresso.brew.start` and `adi.espresso.brew.stop`, and validation says so
rather than letting you find out later. Advisory asks for nothing you have not
built. Raise it when the machine has earned it.

Autonomy is a separate setting, chosen by whoever runs the engine, with
`--level`. It is not in your declaration.

**The `envelope` block is mandatory.** There is no declaration without one,
which is deliberate: it is the part that holds whether limits are enforced by
the appliance as well as the engine, and a machine that declines to say is a
machine nobody can reason about safely.

The one above is minimal and validation will tell you so:

```
declaration: ok  (Acme CM-1)
note: no invariants or rate limits declared. D2 will refuse to run against this
      machine.
```

That note is correct and is not a problem yet. Invariants are the stateful
constraints, the ones individually valid calls can breach in sequence, and D2
refuses to initialise without them because that is exactly the case where a
model proposing values needs a backstop. Add them when you add the tools that
can accumulate. Bring-up runs at D0.

Validate before writing any C:

```sh
cd sdk
cargo run -p adi-cli -- validate path/to/your.yaml
```

This catches a great deal. Take it seriously now rather than at link-up.

---

## Step 2: Generate the tables

```sh
cd sdk
cargo run -p adi-cli -- generate-c path/to/your.yaml --out c/generated --name cm1
```

Two files appear, `adip_generated_cm1.c` and `adip_generated_cm1.h`. Add both
to your build. The header gives you counts to check against:

```c
#define ADIP_CM1_TOOL_COUNT   1
#define ADIP_CM1_SIGNAL_COUNT 1
#define ADIP_CM1_EVENT_COUNT  1
#define ADIP_CM1_MAX_PARAMS   1
```

and the source gives you one symbol, `adip_device_cm1`, which is the device
description you pass to `adip_init`.

**The tables are `const` and contain no `data` or `bss`.** They live in flash.
That is deliberate, and it is why a limit exists in exactly one place: the
declaration. You never retype a range into your firmware, and no edit to the
firmware can widen one.

Rerun this command whenever you edit the declaration. Putting it in your build
rather than running it by hand is worth the ten minutes, because a stale table
is a firmware that enforces last week's limits.

---

## Step 3: The three port functions

This is the whole platform dependency.

```c
uint32_t adip_port_millis(void);
int      adip_port_write(const uint8_t *bytes, size_t n);
void     adip_port_safe_state(void);
```

### `adip_port_millis`

A monotonic millisecond counter. SysTick, an RTOS tick, a hardware timer,
whatever you already have.

It is allowed to wrap. The library does its arithmetic so that a wrap at
2^32 ms, about 49 days, is handled, and there is a test for it. What is not
allowed is a counter that goes backwards or stops while interrupts are
disabled, because the failsafe interval is measured with it.

```c
uint32_t adip_port_millis(void)
{
    return HAL_GetTick();          /* or xTaskGetTickCount(), or your own */
}
```

### `adip_port_write`

Put `n` bytes on the wire. Return how many you wrote.

The one trap worth naming: **if your transmitter is not started, the engine
waits for a reply that is sitting in your buffer, and your appliance looks
dead rather than broken.** On a host that is a missing `fflush`. On a UART it
is forgetting to enable the transmit interrupt.

```c
int adip_port_write(const uint8_t *bytes, size_t n)
{
    /* Blocking is acceptable here and simplest to get right first.
       Move to DMA once the link works, not before. */
    if (HAL_UART_Transmit(&huart1, (uint8_t *)bytes, (uint16_t)n, 100) != HAL_OK) {
        return -1;
    }
    return (int)n;
}
```

### `adip_port_safe_state`

Drive the machine somewhere harmless, now, without asking anyone.

Cut the pump, drop the heater, vent the pressure. Within the
`max_time_to_safe_ms` your declaration promised. Do not wait for a reply, do
not wait for a scheduler, and do not try to tell anyone first.

One thing the library cannot do for you: ADI-P 4.5 requires a fault event
before or with entry to the safe state, and only your firmware knows what
failed. Raise it here.

```c
void adip_port_safe_state(void)
{
    pump_off();
    heater_off();
    vent_open();

    /* Event index 0 is adi.espresso.event.fault in the declaration above.
       The index is the event's position in `events:`, so check yours rather
       than copying this. Adding an event above it shifts it, and nothing
       will tell you. */
    (void)adip_emit_event(&ctx, 0u, "link_lost", 2u /* critical */);
}
```

This function is called when the link goes quiet, which is the case you cannot
test by hand reliably. `sdk/c/test/test_m5.c` tests the timing against a clock
the test controls. Read it before you trust your own.

---

## Step 4: The transport

ADI-P 10 requires four things of a binding: ordered and reliable delivery,
latency below `control_timeout_ms`, liveness detection, and integrity. A UART
with the framing below satisfies all four.

### The framing

From ADI-P 10.2.1, and already implemented inside the library:

```
  0xAD   len_hi   len_lo   <payload ...>   crc_hi   crc_lo
```

Sync byte, 16-bit big-endian payload length, CRC-16/CCITT-FALSE over the
payload only, polynomial 0x1021, initial value 0xFFFF, no final xor,
big-endian.

**You do not implement any of this.** `adip_feed` resynchronises on the sync
byte, checks the length before buffering, and verifies the CRC. You pass bytes
and it deals with frames. A payload never exceeds 512 bytes, which is why
`ADIP_RX_MIN` is 517 and why a fixed buffer is enough.

A binding that already frames and checks integrity, such as BLE GATT, carries
the payload alone. Do not frame twice.

### Feeding bytes

Take bytes as they arrive and hand them over. One at a time is correct and is
what a UART interrupt gives you.

Do not wait to assemble a frame yourself. A read that asks for twenty bytes
blocks until it has twenty, so your appliance waits for bytes the engine has
no reason to send while the engine waits for your reply. Both sides then wait
forever. This is the most common way a first port hangs.

```c
/* In the UART receive interrupt. Keep it to this. */
void USART1_IRQHandler(void)
{
    uint8_t byte = (uint8_t)(USART1->RDR);
    ring_push(&rx_ring, byte);     /* your own ring buffer */
}
```

### The main loop

```c
static adip_ctx_t      ctx;
static uint8_t         rx[ADIP_RX_MIN];
static adip_sample_t   batch[ADIP_MAX_BATCH];
static adip_setpoint_t held[ADIP_CM1_TOOL_COUNT * ADIP_CM1_MAX_PARAMS];

int main(void)
{
    uint8_t byte;
    uint8_t i;

    hardware_init();

    if (adip_init(&ctx, &adip_device_cm1, rx, sizeof(rx), NULL) != ADIP_OK) {
        return 1;
    }
    adip_set_signal_buffer(&ctx, batch, ADIP_MAX_BATCH);
    adip_set_setpoint_buffer(&ctx, held,
                             (uint8_t)(sizeof(held) / sizeof(held[0])));

    /* Where each mechanism stands at power-on. A product reads this back from
       its own non-volatile store; do not assume a value you did not read. */
    adip_report_setpoint(&ctx, ADIP_TOOL_CM1_ADI_ESPRESSO_GRIND_SET_SETTING,
                         0u, grind_setting_now());

    /* Both bitmaps default to the safe reading: nothing satisfied, so nothing
       is invocable until the firmware says otherwise. Set these from your
       switches and sensors as they change, not once at startup.

       Note the sense of each. A precondition is passed whether it is
       satisfied; an interlock is passed whether it is *open*, and an open
       interlock blocks. Getting this backwards gives you a machine that
       runs with the lid up. */
    for (i = 0; i < adip_device_cm1.precondition_count; i++) {
        adip_set_precondition(&ctx, i, precondition_is_satisfied(i));
    }
    for (i = 0; i < adip_device_cm1.interlock_count; i++) {
        adip_set_interlock(&ctx, i, interlock_is_open(i));
    }

    for (;;) {
        while (ring_pop(&rx_ring, &byte)) {
            (void)adip_feed(&ctx, &byte, 1);
        }


        /* Drains whatever is ready. Non-blocking. */
        while (adip_poll(&ctx) == ADIP_PENDING) {
        }

        your_other_work();
    }
}
```

That skeleton, with the declaration above and stubs for the hardware functions, compiles against `libadip` under `-Wall -Wextra -pedantic
-Wconversion -Wsign-conversion` with no warnings. If yours does not, the
difference is in your code rather than in the shape of this.

**`adip_poll` must be called whether or not bytes arrived.** It services
timeouts, and the failsafe is one of them. A loop that polls only on receipt
cannot detect a dead link, which is precisely the condition the failsafe
exists for. If your firmware is interrupt-driven and idles in a sleep
instruction, give `adip_poll` a timer tick of its own.

### Liveness

ADI-P 10.2.1 is explicit, and it is worth repeating because the wrong version
is the intuitive one:

**Arriving bytes are not evidence that the peer is alive.** Only a frame that
passes its CRC is the peer talking. Noise on a floating line looks like
traffic, and treating it as a heartbeat holds the failsafe off while the link
is in fact dead.

The library already applies this rule. Do not add a "bytes seen recently"
check of your own alongside it.

### Telling the engine what you are set to

The engine plans a correction from where a mechanism stands, and it cannot
read your dials. ADI-P 9.1 also requires that an operator can take control by
physical means at any moment, so a value the engine merely remembers having
sent is stale by mandate. The appliance reports its own settings, as it already
reports its interlocks and its faults.

Call `adip_report_setpoint` in two places:

- once at startup, for every parameter, with the value the mechanism actually
  holds, read back from your own store
- in each handler, after the command is accepted, with the value you took

```c
adip_report_setpoint(&ctx, ADIP_TOOL_CM1_ADI_ESPRESSO_GRIND_SET_SETTING,
                     0u, setting);
```

The call records the value; `adip_poll` sends it. Several changes in one pass
therefore cost one frame. A value equal to the one already held sends nothing,
and nothing is sent at all until a peer has spoken, because a machine powers
up long before an engine attaches. After a silence longer than
`control_timeout_ms` the whole set is sent again, so an engine that has just
connected is told afresh without your firmware tracking sessions.

A wrong unit, or a tool or parameter index the declaration does not have, is
refused with `ADIP_ERR_ARG` and never reaches the wire.

The startup report is the one most easily left out, and leaving it out gives
you a machine the engine can command but cannot correct: it has no position to
plan from until it has set something itself.

---

## Step 5: First contact

### Before the board exists

Build your firmware as a host executable with stdin and stdout as the wire,
exactly as `sdk/c/example/main.c` does, and let the engine start it:

```sh
cd sdk
cargo run -p adi-cli -- run --level d0 \
  --declaration path/to/your.yaml \
  --appliance path/to/your-firmware
```

This is the local IPC binding of ADI-P 10 and it is a real transport, not a
simulation. The bytes are identical to the ones a UART will carry, so your
decoding, your validation and your handler are all exercised before any
hardware is involved. Do this first. It is much easier to debug a handler on a
desktop than through a serial cable.

One Windows note, which has cost an afternoon before now: set stdin and stdout
to binary mode, or the C runtime turns every `0x0A` in a payload into
`0x0D 0x0A` and every frame containing one fails its CRC.

```c
#ifdef _WIN32
_setmode(_fileno(stdin), _O_BINARY);
_setmode(_fileno(stdout), _O_BINARY);
#endif
```

### Against the real board

Point the engine at the port instead of at a program:

```sh
cd sdk
cargo run -p adi-cli -- run --level d0 \
  --declaration path/to/your.yaml \
  --serial /dev/ttyUSB0 --baud 115200
```

On Windows the port is `COM3` or similar. `--baud` defaults to 115200.
`--serial` and `--appliance` are two different appliances, so giving both is
refused rather than resolved in favour of one.

Opening the port proves nothing by itself. A port that opens is not a machine
listening, which is what the next step is for.

If it will not open at all, the three usual causes are the port name, something
else already holding it open, which a serial monitor does, and on Linux not
being in the `dialout` group.

### Then ask it something

Type `status`. It sends a heartbeat and prints what came back.

If that answers, your framing, your CRC, your clock and your write path are all
correct. That is the whole of what first contact proves, and it is most of the
work.

### If you would rather own the transport

`Transport` is two methods, `send` and `recv` with a timeout, where `Ok(None)`
from `recv` means nothing arrived yet rather than an error. A product will want
its own implementation eventually, over whatever link it actually ships, and
`sdk/crates/adi-cli/src/serial.rs` is about a hundred lines and is the model to
copy. The reassembly is the part worth reading: a frame arrives in pieces, two
frames arrive in one read, and the line is noisy at power on.

### When it does not work

Nothing below is exotic. Each line is a mistake that produces a symptom
pointing somewhere else.

| Symptom | Where to look |
|---|---|
| Nothing at all | `adip_port_write` returning before the transmitter has sent. Check the TX line with a scope before anything else. |
| Engine reports silence, board looks busy | `adip_poll` not called, or called only when bytes arrive |
| CRC failures | Byte order on the length, or a transport translating bytes. On Windows hosts, `0x0A` becoming `0x0D 0x0A` is the classic. |
| Connects, then drops after a second | `heartbeat_interval_ms` versus a main loop that blocks longer than it |
| Invocations refused | Preconditions and interlocks still at their default, which is the safe reading, so nothing is invocable. Set them. |
| Safe state entered unexpectedly | `adip_port_millis` stopping while interrupts are disabled |
| Nothing over serial, fine over stdio | Baud mismatch, or TX and RX not crossed |

### Before you believe it

Run the library's own suite on your target, not just on a host:

```sh
cd sdk/c
make test        # on a host
make arm-test    # on an emulated Cortex-M3, if you have the toolchain
```

364 checks across seven suites. They test the arithmetic, the decoder, the
validation and the failsafe. If they pass on your target and your link answers `status`, the port
is done.

What neither emulation nor these tests establish is **real timing against real
peripherals**. The reply deadline and the failsafe interval are promises your
hardware makes, and only your hardware can keep them. Measure both.

---

## One thing to know before you write a handler

Bring-up needs no handler, because nothing is invoking anything yet. The moment
you add one, this is the part that bites.

Read arguments **by name**:

```c
adip_num_t setting;

if (adip_arg_num(inv, "setting", &setting)) {
    start_grind_move(setting);
}
```

`inv->args` is in the order the sender encoded, not the order your declaration
lists, and each argument carries the parameter it belongs to in its `index`
field. `inv->args[0]` happens to work on a tool with one parameter and silently
reads the wrong one on a tool with two: it compiles, it runs, and it drives the
mechanism with a number meant for something else.

`adip_arg_num` returns false where the parameter is absent or is an enum, and
leaves your variable untouched, so a missing value never arrives as a zero.

[`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) covers the rest of the handler.

---

## What you have, and what is next

A board that connects, identifies itself, answers a heartbeat and enters a safe
state when the link dies. No mechanism is callable yet, because you declared
one tool and wrote no handler.

That is the next document: [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md),
which takes one mechanism from a pin to something the engine can reason about.

## Checklist

- [ ] Declaration validates with `adi validate`
- [ ] `max_time_to_safe_ms` measured, not guessed
- [ ] `control_timeout_ms` longer than your slowest handler returns
- [ ] Tables generated, and regenerated by the build rather than by hand
- [ ] `adip_port_millis` monotonic, and does not stall with interrupts off
- [ ] `adip_port_write` actually starts the transmitter
- [ ] `adip_port_safe_state` reaches safety within the declared time
- [ ] `adip_port_safe_state` raises a fault event
- [ ] Bytes fed as they arrive, not assembled into frames first
- [ ] `adip_poll` called every loop, not only when bytes arrive
- [ ] Preconditions and interlocks set from real sensors
- [ ] Every sensor that measures an outcome says so, with `measures`
- [ ] `adip_set_setpoint_buffer` called before any report
- [ ] Every parameter reported at startup, read back rather than assumed
- [ ] Every handler reports the value it accepted
- [ ] `make test` passes on the target
- [ ] `status` answers from the engine
- [ ] Reply deadline and failsafe interval measured on hardware

---

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
