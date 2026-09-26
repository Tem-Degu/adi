# libadip: C reference SDK

**Design plan · M1 to M5 complete, M6 needs hardware · Targets ADI-P 0.2**

The appliance side of ADI-P, written to be linked into a manufacturer's existing
firmware. This document is the plan and the record of progress against it.

---

## 1. What it is for

A manufacturer's firmware engineer should be able to make their machine speak
ADI-P in a day, without adopting our language, our build system or our release
schedule, and without writing a parser or a validator themselves.

`HOW-TO-ADD-A-TOOL.md` currently tells that engineer to hand-write a message
handler. This SDK is what makes that step small instead of a project.

## 2. What "correct" means here

This is a safety-relevant component running on constrained hardware in a
regulated product. Five constraints follow, and every design decision below is
downstream of them.

**No dynamic allocation.** Not "avoid malloc". Zero. The library must contain no
allocation of any kind, and the caller supplies every buffer. Appliance firmware
often runs with no heap at all, and where a heap exists, allocation failure at
runtime in a safety path is unacceptable.

**Bounded and known resource use.** No recursion, so stack depth is statically
provable. Fixed upper bounds on every loop. A firmware engineer must be able to
state the worst case before shipping.

**No floating point.** Cortex-M0 and M3 have no FPU, and software floating point
is slow and, worse, makes exact range comparison awkward. Numbers are carried as
a scaled integer, described in section 5.

**Freestanding C.** C99, no `stdio`, no `stdlib`, no `printf`. `string.h` only
for `memcpy` and `memcmp`. Portable to a compiler that only offers the
freestanding subset.

**Fails closed.** Anything unrecognised, malformed, out of range or unverifiable
is rejected. There is no permissive path and no default-allow.

Secondary but real: appliance firmware is frequently written to MISRA C, so the
code should avoid the constructs MISRA forbids even where we do not formally
certify. No `goto` except for single-exit error handling, no unions for type
punning, explicit widths everywhere.

## 3. The decision that shapes everything

**The device never parses a declaration.**

This is worth stating plainly because the obvious design gets it wrong. A
declaration is a YAML or JSON document, sometimes thousands of lines. Parsing
that on a microcontroller would mean shipping a parser, allocating for the
resulting tree, and spending flash on something that never changes between
boots.

The declaration is written by the manufacturer and read by the **engine**. The
device only needs the parts that constrain its own behaviour: which tools exist,
what ranges their parameters have, which interlocks block what, and what the
stateful limits are.

So those parts are **compiled into static C tables at build time** by a
generator, and the runtime is a small interpreter over those tables.

Two consequences, both good:

- The runtime shrinks to something that fits comfortably on an MCU.
- The duplication that `HOW-TO-ADD-A-TOOL.md` currently describes as deliberate
  goes away. Today a range appears in the declaration and again in hand-written
  C, and the two can drift. Generated tables cannot drift, because there is one
  source.

## 4. The three pieces

```
  declaration.yaml
        |
        |  adi generate-c          (build time, on a host)
        v
  adip_generated.c / .h            static tables, no logic
        |
        +----> libadip             the runtime: framing, validation,
        |                          dispatch, stateful envelope, failsafe
        |
        +----> port layer          ~4 functions the manufacturer writes
        |
        +----> tool handlers       one function per tool, theirs
```

### 4.1 The generator

Lives in `adi-cli` as `adi generate-c`, reusing `adi-protocol` so the generator
and the engine agree by construction.

Emits, from a declaration:

- A tool table: id, parameter descriptors, ranges, preconditions, hazard flags,
  `reversible`, `abortable`, max duration.
- An interned string table for tool ids, so the device compares small integers
  rather than doing `strcmp` on every message.
- An interlock table, and a bitmap of which tools each blocks.
- Invariant, rate limit and resource tables from the envelope.
- Compile-time constants for the buffer sizes the runtime needs, so the caller
  can allocate statically with the right size.

All of it `const`, so it lands in flash rather than RAM.

### 4.2 The runtime

Transport-agnostic and loop-agnostic. It owns neither the wire nor the
scheduler, because appliance firmware already has both.

Responsibilities:

- Frame the byte stream into messages, and reject malformed framing.
- Decode an invocation into typed arguments.
- Validate: tool exists, arguments present and of the right type, values within
  declared range and resolution, units match, preconditions and interlocks
  satisfied, stateful envelope not breached.
- Dispatch to the handler.
- Format the reply, with the correct reason code on rejection.
- Track the stateful envelope: accumulate invariant quantities, enforce rate
  limits and minimum intervals, decrement resources, reset on the declared
  resetting tools.
- Enforce the failsafe: if the control link goes quiet past
  `control_timeout_ms` while a non-reversible operation is running, drive the
  safe state.

### 4.3 The port layer

Everything platform-specific, and deliberately tiny.

```c
uint32_t adip_port_millis(void);                       /* monotonic, wraps */
int      adip_port_write(const uint8_t *b, size_t n);  /* bytes out */
void     adip_port_safe_state(void);                   /* failsafe entry */
```

Three functions. A manufacturer who cannot implement these in an afternoon has a
larger problem than ADI-P.

## 5. Numbers without floating point

Every numeric value is carried as a **scaled integer**:

```c
typedef struct {
    int32_t mantissa;   /* the value, scaled */
    int8_t  exponent;   /* power of ten */
    uint8_t unit;       /* the closed unit vocabulary from PROTOCOL.md 4.2 */
} adip_num_t;
```

So 36.5 grams is `{ 365, -1, ADIP_UNIT_G }`.

Ranges in the generated tables use the same representation, so comparison
requires only integer arithmetic after normalising exponents. There is no
rounding ambiguity, which matters because a range check on a safety limit must
be exact and reproducible rather than nearly right.

A float accessor is provided for hosts that want it, guarded so it can be
compiled out entirely.

## 6. The API, sketched

```c
/* Setup. The caller owns all memory. */
adip_status_t adip_init(adip_ctx_t *ctx,
                        const adip_device_t *dev,     /* generated tables */
                        uint8_t *scratch, size_t scratch_len,
                        void *user);

/* Transport in. Caller passes whatever arrived; partial frames are fine. */
adip_status_t adip_feed(adip_ctx_t *ctx, const uint8_t *bytes, size_t len);

/* Called from the main loop. Non-blocking. Dispatches at most one message,
   services timeouts, and returns whether more work is pending. */
adip_status_t adip_poll(adip_ctx_t *ctx);

/* State the runtime needs from the firmware. */
void adip_set_interlock(adip_ctx_t *ctx, uint16_t interlock, bool open);
void adip_set_state(adip_ctx_t *ctx, uint16_t state);
void adip_publish(adip_ctx_t *ctx, uint16_t signal, adip_num_t v, uint32_t t_ms);

/* Completing a long-running operation. */
adip_status_t adip_complete(adip_ctx_t *ctx, uint32_t invocation,
                            adip_outcome_t outcome, adip_reason_t reason);
```

The manufacturer implements one dispatch function:

```c
adip_action_t adip_on_invoke(uint16_t tool, const adip_args_t *args, void *user);
```

Returning `ADIP_ACTION_DONE` for something instantaneous, or
`ADIP_ACTION_RUNNING` for anything that takes time, with `adip_complete` called
later. **Handlers must never block.** A six second wipe cannot stall the
appliance's main loop, and a library that encourages blocking would be wrong for
this environment.

## 7. What the runtime enforces, and what it does not

**Enforces:** everything expressible from the declaration. Ranges, resolution,
units, interlocks, preconditions on declared state, invariants, rate limits,
resource levels, response deadlines, failsafe timing.

**Does not enforce:** anything requiring physical judgement. Whether the arm
actually moved, whether the tray is really clean, whether a value is sensible
for this coffee. Those belong to the firmware and to the engine respectively.

This boundary is the point. The SDK is not a safety system. **It helps the
appliance enforce the envelope the appliance already owns**, and the spec is
explicit that enforcement must be independent of the engine. A manufacturer who
deletes this library must still be safe.

## 8. Budgets

Targets, to be measured rather than assumed:

| | Target |
|---|---|
| Flash, runtime | under 16 KB |
| RAM, excluding generated tables | under 2 KB |
| Generated tables | flash, scales with tool count |
| Stack, worst case | under 512 bytes, statically provable |
| Response latency | well inside the 200 ms the spec requires |

If the runtime does not fit an MCU comfortably, the design has failed, because
the engine already needs an applications processor and the whole point of the
device side is that it does not.

**The flash figure was 12 KB until the outbound half was built**, and is raised
here rather than met by squeezing, for two reasons worth stating.

The old number described a smaller library. It was set when this SDK could only
receive: an invocation in, an acknowledgement out, and no way to report a
measurement, raise an event or answer a heartbeat. A library that also talks is
doing about twice the work, and holding it to a figure chosen before half its
job existed would mean trimming safety-relevant code to honour a constraint we
invented.

The measurement is not the target. Every figure in section 9 is x86-64 and
labelled indicative for good reason: this is branch-heavy integer code with no
floating point, and Thumb-2 typically emits it 25 to 30 percent smaller. 12052 B
here is likely 8 to 9 KB on a Cortex-M, which was comfortable under the old
number and remains so. That stays a projection until M6 puts it on a board, and
M6 is where this budget gets confirmed or corrected against a real target rather
than a host.

16 KB keeps the claim that matters: this belongs on the cheap side of an
appliance, not on an applications processor.

## 9. Build order

**M1. Types and generated tables. DONE.** `adip_num_t`, the unit vocabulary,
table layouts, and `adi generate-c` emitting tables for both example
declarations.

Measured on an x86-64 host, so indicative rather than final, but the shape is
what matters:

| | text | data | bss |
|---|---|---|---|
| `adip_num.o` | 860 B | 0 | 0 |
| `adip_tables.o` | 408 B | 0 | 0 |
| generated tables, 6 tools | 2320 B | 0 | 0 |

**Zero bytes of `data` and `bss`.** The tables are genuinely const and live in
flash, which was the point of generating them. 49 host checks pass, and the
whole thing compiles clean under `-Wall -Wextra -pedantic -Wconversion
-Wsign-conversion -Wcast-qual -Wshadow -Wdouble-promotion`.

Three defects the compiler caught that review had not: enum value arrays were
being emitted inside an initialiser list, which is not valid C; sizing macros
and tool index enums collided when two generated headers appeared in one
translation unit; and the device symbol itself collided for the same reason. It
is now `adip_device_<basename>` with every header symbol namespaced.

**M2. Decode and validate. DONE.** Framing with CRC, a CBOR reader, decode into
a resolved invocation, and validation against the declaration. 54 host checks,
almost all of them rejection paths.

Runtime footprint after M2, x86-64 host, indicative:

| | text | data | bss |
|---|---|---|---|
| `adip_cbor.o` | 1492 B | 0 | 0 |
| `adip_decode.o` | 1324 B | 0 | 0 |
| `adip_validate.o` | 804 B | 0 | 0 |
| `adip_num.o` | 860 B | 0 | 0 |
| `adip_frame.o` | 548 B | 0 | 0 |
| `adip_tables.o` | 408 B | 0 | 0 |
| **total** | **5436 B** | **0** | **0** |

Under half the 12 KB budget with the hardest part done, and still no RAM.

**M3. Dispatch and reply. DONE.** Context, feed, poll, dispatch, the handler
contract, asynchronous completion. 57 host checks.

Footprint after M3, x86-64 host, indicative:

| | text | data | bss |
|---|---|---|---|
| all six core files | 5436 B | 0 | 0 |
| `adip_runtime.o` | 1744 B | 0 | 0 |
| **total** | **7180 B** | **0** | **0** |

RAM per instance: `adip_ctx_t` is 136 bytes, plus a caller-supplied receive
buffer of 517. About 650 bytes against a 2 KB target, and still nothing in
`bss` because the caller owns the buffer.

Two decisions worth recording.

**The handler is called before ACCEPTED is sent.** That lets firmware refuse
for a reason only it knows, such as a hardware fault, without having to abort
something it already acknowledged. It is only sound because handlers must not
block, so the non-blocking contract is what keeps the 200 ms reply deadline
reachable rather than being a separate mechanism.

**A second invocation while busy is refused with BUSY, not queued.** Queuing
would mean holding an invocation whose preconditions may no longer hold by the
time it runs, which is exactly the kind of stale-state bug the protocol exists
to prevent.

The build now separates `CORE` from `RUNTIME`. Only `adip_runtime.c` calls the
port layer, so everything else links and tests with no platform at all.

**M4. Stateful envelope. DONE.** Invariants, rate limits and resources, with
the first test group mirroring `adi-safety`'s `composition.rs` case for case
against the same fixture. 30 host checks.

**Cross-implementation testing paid for itself immediately.** It found a real
bug in `adi-safety`: `check_invariants` added the raw `f64` of a contribution
without comparing its unit to the invariant's, so grams could be summed into a
millilitre total. The C refused, the Rust accepted, and the two disagreed about
whether an action was legal. That is precisely the divergence that would show up
in the field as an engine permitting what the appliance rejects. Both sides now
check, with a regression test in `composition.rs`.

It also found a modelling error in the neutral fixture, which had one invariant
accumulating from a millilitre parameter and a gram one. The process output is
now a volume.

Footprint after M4:

| | text |
|---|---|
| core, six files | 5648 B |
| `adip_runtime.o` | 2000 B |
| `adip_state.o` | 1480 B |
| **total** | **9148 B** |

RAM: `adip_ctx_t` grew to 496 bytes, since it now carries accumulators,
resource levels and rate-limit history. With the 517 byte receive buffer that
is about 1 KB against the 2 KB target.

The compile-time maxima are enforced at init: a declaration with more
invariants, rate limits or resources than this build can track is REFUSED with
`ADIP_ERR_UNSUPPORTED` rather than partially enforced. Tracking eight of nine
invariants would mean the declaration advertises a limit the firmware silently
ignores, which is worse than not starting.

**M5. Failsafe and timing. DONE.** Link timeout, safe state entry, duration
overrun, and the reply deadline as a measurement. 42 host checks, all of them
on paths that only run when something has gone wrong.

Four decisions worth recording.

**Noise is not liveness.** `link_ms` advances only when a frame passes its CRC,
never when bytes merely arrive. A stuck transmitter spraying rubbish must not
hold the failsafe off, because rubbish is not the engine talking. A test feeds
noise throughout an operation and confirms the failsafe still fires.

**An idle machine does not fault.** The failsafe is scoped to an operation in
flight. Faulting a machine that is doing nothing because the engine went quiet
would be a nuisance rather than a safeguard.

**Reversible operations are stopped but do not fault.** PROTOCOL.md 4.6
requirement 4 scopes the MUST to non-reversible operations, and that is where
the safe state is driven. A reversible one is still stopped, since continuing
to move a machine nobody is talking to is not defensible either.

**Resuming is not automatic.** Once the failsafe fires the context refuses
further invocations with FAULT until the firmware calls `adip_clear_fault()`.
A machine interrupted mid-cycle may hold material the engine knows nothing
about, and only the firmware can say when it is fit to continue.

The 200 ms deadline is measured rather than asserted. `stat_worst_reply_ms` and
`stat_deadline_misses` give a conformance run evidence instead of a claim, and
a handler that blocks shows up there immediately.

Footprint after M5: 9516 B text, 0 data, 0 bss. `adip_ctx_t` is 496 bytes plus
the 517 byte receive buffer.

**A correction to the M4 work, found later.** `adip_check_stateful` read the
clock itself while `adip_commit_stateful` took `now_ms` from its caller. Two
consequences. A rate limit could be tested against one instant and recorded
against another, which is a real defect in safety code. And `adip_state.c`
therefore called the port layer, so the `CORE` and `RUNTIME` split above was not
true as written: `CORE` could not link without a platform. `adip_check_stateful`
now takes `now_ms` too, the runtime reads the clock once and uses it for both
calls, and `test_m1` and `test_m2` link `CORE` alone, which is what makes the
claim checkable rather than merely stated.

**M6. Port and integrate.** Reference port for one real MCU, then run the
conformance suite against a device rather than a simulator.

M1 to M5 need no hardware and are testable on a host.

## 10. Testing

**Host unit tests** for every rejection path. The happy path is the easy part;
the value is in confirming that out-of-range rejects rather than clamps, that an
open interlock blocks, that a malformed frame does not corrupt state.

**Cross-implementation agreement.** The conformance corpus in
`sdk/conformance/` drives both the Rust engine and this SDK, and the two must
produce identical accept and reject decisions on identical input. Two
independent implementations agreeing is much stronger evidence than either alone.

**Fuzzing the decoder.** The decoder is the only place untrusted bytes are
handled, so it gets fuzzed. Any input must produce a rejection rather than a
crash, an out-of-bounds read, or corrupted state.

**Static analysis** with the warnings turned all the way up, plus a MISRA-style
pass, since some customers will require it.

## 11. Deliberately out of scope for v1

- **Transport drivers.** No UART, no BLE, no USB. The caller owns the wire.
- **Declaration parsing on device.** Section 3.
- **Procedures.** Section 8.3 of the spec is an escape hatch, and the device can
  execute the steps as ordinary invocations until a customer needs otherwise.
- **The skill and knowledge extensions.** Those are read by the engine. The
  device never sees them.
- **An RTOS abstraction.** Three port functions, no task model.

## 12. Open questions

**Wire encoding. DECIDED: CBOR.** Three reasons, in order of weight.

RFC 8949 tag 4 is a decimal fraction, encoded as `[exponent, mantissa]`, which
is exactly `adip_num_t`. Numbers cross the wire with no conversion and no
rounding, which is what keeps a range check against a safety limit exact. JSON
would have meant parsing decimal text into scaled integers on an MCU.

A decoder for the subset we need is 1.5 KB and needs no allocation.

It matches the BLE binding the spec already specifies.

Binary floats are refused rather than converted, since a float cannot hold 0.1
exactly and accepting one would put back the ambiguity scaled integers remove.

The development-only JSON path was dropped. A second encoder is a second thing
to get wrong, and `adi-cli` can print a decoded frame on the host instead.

**Timer wraparound.** `adip_port_millis` wraps every 49 days on a 32-bit
counter. Unsigned subtraction handles it correctly, but every comparison must be
written that way and it must be tested at the boundary rather than assumed.

**Reporting engine defects.** The spec says an out-of-range invocation indicates
an engine defect and SHOULD be logged. Whether the SDK keeps a counter, and
whether it surfaces one, is undecided.
