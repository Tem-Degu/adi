# How to add a tool

**A guide for appliance firmware engineers. ADI-P 0.2.**

This document explains how to make a mechanism on your machine callable by an
Artificial Domain Intelligence engine. It is written for the engineer who will
do the work, and assumes no prior knowledge of ADI.

It assumes one thing about your setup: that the library is already on your
board, or on your desk as a host build, and that an engine can reach it. If it
cannot yet, start with
[`HOW-TO-PORT-TO-A-BOARD.md`](./HOW-TO-PORT-TO-A-BOARD.md), which ends where
this begins.

**When you finish, you will have** one mechanism the engine can call, with its
range and its units enforced in two places, and, if you write the optional
skill entry, one the engine will choose by itself to reach a result somebody
asked for.

The specification is [`PROTOCOL.md`](./PROTOCOL.md). This guide is the
practical path through it.

---

## The short version

A **tool** is one action your machine can perform. Setting the grind, dispensing
milk, ejecting the puck, wiping a tray. Each tool is a function in your firmware
whose body happens to move metal.

Making one callable takes three things:

1. The mechanism, wired and driven from a pin, as you already do today.
2. A handler in your firmware that runs it when a named instruction arrives.
3. One entry in a data file declaring the name, its parameters and its limits.

Two further steps, both optional and both data. One entry in the skill lets the
engine *choose* your tool while solving a problem rather than only call it when
told. And notes let the machine answer questions about your machine in words.

Steps 1 and 2 are ordinary appliance engineering. **Step 3 is the only new work
the protocol asks of you**, and it is not code.

Nothing here requires a release from this project. Publish a new declaration
and the engine picks the tool up on its next connection.

---

## Where software meets the machine

One place only: a pin going high.

```
   supply  ------------------+
                             |
                        [ actuator ]
                             |
   pin  --->  [ driver ]  ---+
                             |
                           ground
```

The pin carries almost no current. It is a light switch, and the driver is what
lets the real power through. Above the pin everything is bytes. Below it
everything is voltage.

Sensing is the same chain in reverse: a physical quantity changes a sensor
electrically, an amplifier and ADC turn that into a number, and calibration
turns the number into grams or degrees.

---

## Phase 1: Mechanical

### 1. Define the action as a state change

Not "a wiper". Rather: *the tray goes from soiled to clean, starting and ending
at the home position.*

Every tool needs a defined start state and end state. If you cannot write that
sentence, the tool is not yet one action and probably needs splitting.

### 2. Build the mechanism

Actuator, linkage, end stops.

### 3. Decide how you know it finished

This step gets skipped, and it matters more than it looks.

If you run a timer and then report success, that report is a guess. It becomes
untrue the moment the mechanism jams. A limit switch at the end of travel makes
completion a fact rather than an assumption.

The engine trusts what your firmware reports. Make it truthful.

---

## Phase 2: Electrical

### 4. Establish what the actuator needs

Voltage and current. A small gear motor might be 12 V at 0.5 A.

### 5. Choose the switch

| Switch | Use it for |
|---|---|
| **MOSFET / transistor** | DC loads, one direction, silent, fast |
| **H-bridge** | Anything that must reverse |
| **Relay** | Mains voltage, large loads. Audible click, slower |
| **Optocoupler** | When the two sides must share no electrical connection at all |

Use an optocoupler wherever the machine side is dangerous or electrically noisy.
It passes the signal as light across a sealed gap, so a fault on one side cannot
travel to the other.

### 6. Wire it

Pin to driver, driver to actuator, actuator on its own supply. Never through the
pin.

### 7. Add the sensing you need

The end-of-travel switch from step 3, plus any interlock the action depends on,
such as a switch proving the tray is present.

---

## Phase 3: Firmware

### 8. The pin layer

```c
void motor_forward(void) { gpio_set(PIN_A, HIGH); gpio_set(PIN_B, LOW);  }
void motor_reverse(void) { gpio_set(PIN_A, LOW);  gpio_set(PIN_B, HIGH); }
void motor_stop(void)    { gpio_set(PIN_A, LOW);  gpio_set(PIN_B, LOW);  }
```

### 9. The meaning layer

One function expressing what the machine does, with its own protection built in.

```c
int wipe_tray(int passes) {
    for (int i = 0; i < passes; i++) {
        motor_forward();
        if (!wait_for(limit_far, 5000))  { motor_stop(); return ERR_TIMEOUT; }
        motor_reverse();
        if (!wait_for(limit_home, 5000)) { motor_stop(); return ERR_TIMEOUT; }
    }
    motor_stop();
    return OK;
}
```

Note the timeouts. Anything that moves must be able to give up.

### 10. Register the name

You do not write a parser, a validator or a state machine. `libadip` does all
of it from tables compiled out of your declaration, and hands you one function
to fill in.

```c
#include "adip.h"
#include "adip_generated_ex900.h"   /* generated from your declaration */

adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    adip_num_t passes;

    if (adip_tool_index(ctx.dev, inv->tool) != TOOL_TRAY_WIPE) {
        return ADIP_ACTION_DONE;
    }
    if (!adip_arg_num(inv, "passes", &passes)) {
        return ADIP_ACTION_DONE;
    }

    /* Start the movement and return. Do not wait for it. */
    start_wipe(passes);
    return ADIP_ACTION_RUNNING;
}
```

**Read arguments by name, never by position.** `inv->args` is in the order the
sender encoded, not the order your declaration lists, and each argument carries
the parameter it belongs to in its `index` field. `inv->args[0]` happens to work
on a tool with one parameter and silently reads the wrong one on a tool with
two, which compiles, runs, and drives the mechanism with a number meant for
something else. `adip_arg_num` does the lookup and returns false where the
parameter is absent or is an enum, so a missing value never arrives as a zero.

By the time this is called, the library has already confirmed that the tool
exists, that every argument is present and of the right type, that each value
is inside the declared range and carries the declared unit, that no interlock
blocks it, that your preconditions hold, and that the stateful envelope is not
breached. It has also started the 200 ms reply clock and will answer for you.

What is left is the part only your firmware knows: whether the hardware is
actually able to do it right now.

Three rules that are easy to get wrong, and that the library cannot enforce
for you.

**Handlers must not block.** A six second tray wipe cannot stall the appliance's
main loop. Start the movement, return `ADIP_ACTION_RUNNING`, and call
`adip_complete()` when it finishes or fails. The whole API is shaped to make
that the natural thing to write, and the 200 ms reply deadline is only
reachable because of it.

**Report the truth.** `ADIP_ACTION_DONE` means it happened. If you know only
that a timer expired, you do not know that it happened. See step 3.

**Publish what you measure.** `adip_publish()` sends a reading to the engine,
batched for you. A machine that cannot report is a machine the engine cannot
reason about: Conformance Level 0 is signals and no tools at all.

The three port functions are the whole platform dependency:

```c
uint32_t adip_port_millis(void);                       /* a monotonic tick   */
int      adip_port_write(const uint8_t *b, size_t n);  /* bytes out          */
void     adip_port_safe_state(void);                   /* the failsafe       */
```

**A complete working example is in [`sdk/c/example/`](./sdk/c/example/)**: the
three port functions, one handler, a main loop, and a machine that accepts
commands, reports a measurement curve and raises events. Copy it.

### 11. Handle the failure paths

Decide what happens if the link to the engine drops mid-movement. The answer
must be that the machine stops itself, within the time your declaration states,
without waiting for anyone's permission.

Your machine is the final authority. It enforces its own limits regardless of
what arrives.

---

## Phase 4: The declaration

This is data. No programming.

### 12. Choose the identifier

| Form | When |
|---|---|
| `adi.espresso.grind.set_setting` | It matches a meaning already in the registry (PROTOCOL.md §7) |
| `x.yourcompany.tray.wipe` | It is your own invention |

Use the registry ID whenever one fits, because the engine already knows what it
means. Use your own namespace for anything else. You are never required to force
a new idea into an existing ID.

### 13. Write the entry

```yaml
tools:
  - id: "x.acme.tray.wipe"
    name: "wipe_tray"
    parameters:
      passes:
        type: integer
        unit: step
        min: 1
        max: 3
        resolution: 1
        default: 1
    preconditions: ["state == idle", "tray_present == true"]
    effects: ["tray_state"]
    duration_ms_typ: 6000
    duration_ms_max: 15000
    reversible: false
    abortable: true
    authority: ["engine", "operator"]
    hazard: ["moving_parts"]
```

Field notes:

- **Every numeric parameter needs a unit, a range and a resolution.** Unitless
  numbers are not accepted.
- **`monotonic`** is required on any mechanism where direction carries meaning,
  such as a grinder collar. The engine must never guess which way is finer.
- **`effects`** names what the tool changes. The engine uses this to avoid
  changing two things at once, which is a hard rule in process tuning.
- **`reversible: false`** for anything that consumes material, produces heat or
  pressure, or cannot be undone by another call.
- **`preconditions`** must list every state you require. The engine will not
  invoke a tool whose preconditions it cannot verify.

You will notice the range does not appear in your C at all. `adi generate-c`
compiles this declaration into const tables that live in flash, so there is one
source for every limit and nothing to drift.

The checking is still duplicated, and deliberately: the engine checks so it
never sends something wrong, and your firmware checks because it must never
trust the engine. What is no longer duplicated is the number itself.

### 13b. Describe it

Required for your own tools, and unnecessary for ours.

A registry identifier such as `adi.espresso.grind.set_setting` means the same on
every machine that declares it, so the profile supplies the description and you
do not retype it. Your own identifier means nothing to anyone else: a model
offered `x.acme.aux.actuate_3` with a magnitude and a confidence cannot tell what
it does. `adi validate` refuses a vendor tool that the skill makes selectable and
that carries no description.

Write it anyway for one of ours where your mechanism differs from the usual,
since what is true of your machine is yours to state and takes precedence.

```yaml
    description: >
      Stepper-driven burr collar, 60 detents. Settles within two seconds, and
      retains roughly 0.4 g from the previous setting, so a purge is advisable
      after a change of more than four steps.
```

Two rules. Do not put the magnitude of the effect here: `per_unit` in the skill
is what sizes a correction, and writing it twice creates two numbers that will
disagree. And do not rely on it for safety, because nothing does: the engine
clamps and checks whatever is chosen, so a description that is wrong produces a
poor choice among safe options rather than an unsafe one.

### 14. Add envelope entries if the tool can accumulate

Some limits only exist across a sequence rather than within one call. Three
dispense actions can each be in range while their sum overflows a cup.

A wiper needs none of this. A dispenser does:

```yaml
envelope:
  invariants:
    - id: "cup_capacity"
      quantity: "cup_contents"
      max: 320.0
      unit: ml
      accumulates:
        "adi.espresso.additive.dispense": "volume"
      reset_on: ["adi.espresso.puck.eject"]

  rate_limits:
    - id: "brew_thermal_recovery"
      tools: ["adi.espresso.brew.start"]
      min_interval_s: 25.0
```

### 15. Validate before connecting anything

```sh
adi validate declaration.yaml --skill skill.yaml
```

This catches malformed identifiers, missing units, and ranges that contradict
the envelope, before a single motor turns.

**At this point the tool exists.** The engine reads your declaration on its next
connection and can call it. A user can ask for it by name, and a saved recipe
can reference it. Everything below is optional.

---

## Phase 5: Making it reasoned about (optional)

The engine can call your tool now. For it to *choose* your tool on its own while
solving a problem, it has to know what the tool does to the outcome. That is
physics, and no description field contains it.

### 16. Skill response

One entry saying which outcome your tool moves and by how much. A second
grinder, for instance:

```yaml
responses:
  - tool: "x.acme.grind.fine_adjust"
    parameter: "offset"
    variable: "shot_time"
    per_unit: -0.4          # signed: +1 offset shortens the shot by 0.4 s
    unit: s
    valid_range: { min: -10, max: 10 }
    confidence: 0.7
```

`per_unit` is a signed number rather than a word, so direction cannot be
misread. `confidence` between 0 and 1 tells the engine how firmly you hold the
figure, and it sizes its correction steps accordingly.

### 16b. Declare your own outcome, if none of ours fits

A response has to name an outcome the engine models, and the espresso profile
models nine of them. A tool doing something the profile never considered,
wiping a drip tray, flushing a line, retracting a cover, moves none of them.

Declare your own, in your namespace, in your knowledge extension:

```yaml
variables:
  x.acme.tray_state:
    unit: none
    description: >
      How clean the drip tray is, from 0 for soiled to 1 for clean.
```

Then name it in the skill exactly as you would one of ours:

```yaml
responses:
  - tool: "x.acme.tray.wipe"
    parameter: "passes"
    variable: "x.acme.tray_state"
    per_unit: 0.4
    unit: none
    confidence: 0.8
```

The engine can now select your tool, because it has an outcome to pursue and a
number telling it how far one pass gets there. This is what makes a capability
nobody anticipated reachable, and it needs no change from us.

The `description` matters more here than for a base outcome. A model has
intuitions about what makes coffee bitter and none at all about
`x.acme.tray_state`, so what you write is the only thing connecting a person's
words to your mechanism.

Two constraints, both checked at validation: the identifier must sit in your
namespace, and you may not redefine one of ours.

### 17. Dynamics

If the effect is not immediate, say so. Retention is the common case: after a
grind change, some of the previous setting is still in the chamber, so the next
result is a blend. An engine unaware of this misreads that result and corrects
in the wrong direction.

```yaml
dynamics:
  - tool: "x.acme.grind.fine_adjust"
    retention:
      mass: { value: 0.8, unit: g }
      remedy_tool: "adi.espresso.grind.purge"
    settling:
      time: { value: 15, unit: s }
```

### 18. Procedure, sparingly

Only when your hardware requires a fixed order that the engine cannot derive.
Housekeeping sequences qualify. Drink definitions do not, and belong in a
knowledge extension instead.

```yaml
procedures:
  - id: "x.acme.sequence.espresso_with_cleanup"
    effects: ["yield_mass", "puck_state", "tray_state"]
    abortable_until: "brew"
    max_duration_s: 120
    steps:
      - id: "brew"
        tool: "adi.espresso.brew.start"
        arguments:
          stop_at_mass: { binding: engine, domain_variable: "yield_mass" }
      - id: "wipe"
        tool: "x.acme.tray.wipe"
        arguments:
          passes: { binding: fixed, value: 1 }
        on_error: "continue"
```

**The binding rule.** `passes` is fixed because it is a property of your
hardware. `stop_at_mass` must be engine-bound because it determines the coffee.
An argument corresponding to a modelled domain variable MUST be engine-bound,
and a procedure that fixes one will be rejected.

You own the order of operations. The engine owns the numbers.

---

## Phase 6: Telling it what you know (optional)

Everything above makes the machine *do* things. This makes it *answer* things.

### 19. Notes

Free text the machine may answer from, in your knowledge extension:

```yaml
notes:
  - id: "x.acme.note.basket"
    text: >
      A 58 mm basket, taking 18 to 20 g. Third-party baskets of the same
      diameter fit, and a stepped basket will not seat correctly.

  - id: "x.acme.note.descaling"
    text: >
      Descale every 300 cycles, or sooner in hard water. The machine does not
      detect scale; the counter is on the display.
```

What goes in them is your judgement. We specify the mechanism and prescribe no
content. If you believe an owner might ask it, write it down.

Four things to know.

**Every note is sent to the model on every utterance.** There is no relevance
filter, because filtering is a guess about what a question needs and a wrong
guess withholds the answer without telling anyone.

**You are therefore responsible for the total size.** A model has a finite
context. Keep the notes within what the smallest model you ship can hold,
alongside everything else the engine sends. Exceed it and the engine truncates
them and prints a warning, which is a degraded answer rather than a failure, but
it is still your fault and not the engine's.

**A note may inform an answer and may never determine an action.** Supplying
them changes nothing about how a request is validated: a closed schema, outcomes
only from the declared set, every value clamped and checked. A note cannot make
the machine do something it could not already do.

**Do not put numbers in them.** "Aim for 27 seconds" belongs in a style and
"shortens the shot by 0.4 s per step" belongs in the skill, where both are
validated. A figure written in a note as well will eventually disagree with the
one that is used.

The machine also answers from what it already knows without any notes at all:
its own model and firmware, what each declared tool is for, and what each
outcome means. You get that for free.

### 20. What an answer is not

An answer is text going to a person, and nothing verifies it.

An action is safe because of the shape of the system: you declared it, the
engine clamped it, your firmware validated it again. An answer has none of that
behind it. The engine instructs the model to answer only from what it was given
and to say it does not know otherwise, which narrows the risk and does not
remove it.

So treat the notes as published material, in the same way a printed manual is,
because that is what they become.

---

## Phase 7: Test, in this order

Each stage works before you add the next. Do not wire everything and then debug.

**21. The function alone.** On the bench, no engine. Does the mechanism complete
and return home.

**22. Against the reference engine, on a laptop.** The messages are CBOR, so
they are not something you can type by hand. Build your firmware as a host
binary and let the engine drive it:

```sh
cd sdk/c && make example
cd .. && cargo run -p adi-cli -- run --level d0 --appliance c/build/example                                      --declaration path/to/your-machine.yaml
```

`adi run` starts your binary, opens a session against it, and gives you a
prompt. You are testing your handler against the same engine that will ship,
before any board exists.

The automated equivalent, if you want this in your own test suite:

```sh
cargo test -p adi-cli --test loopback
```

**23. On the machine, with the engine connected.**

**24. The failure paths.** These matter more than the happy path:

- Open the interlock. Confirm `REJECTED` with `INTERLOCK`.
- Send an out-of-range value. Confirm rejection, not clamping.
- Disconnect the link mid-movement. Confirm the machine stops itself within the
  declared time.
- Jam the mechanism. Confirm the timeout fires and it reports `ABORTED`.

---

## Checklist

**All tools**

- [ ] Start state and end state are defined
- [ ] Completion is detected, not assumed
- [ ] The actuator is driven through a switch, never from a pin
- [ ] The firmware function has a timeout
- [ ] `ACCEPTED` or `REJECTED` is returned within 200 ms
      (`stat_worst_reply_ms` measures it rather than asserting it)
- [ ] Handlers return rather than blocking, so the deadline stays reachable
- [ ] Measurements are published, or the engine has nothing to reason about
- [ ] Out-of-range values are rejected, never silently clamped
- [ ] The declared range is wide enough to be useful. The engine clamps to it and
      reports the correction as partial, so a range narrower than the mechanism
      allows makes the machine look stuck
- [ ] Every numeric parameter declares unit, range and resolution
- [ ] `monotonic` is set on any direction-bearing mechanism
- [ ] Preconditions list every required state
- [ ] The declaration validates
- [ ] Link loss during a non-reversible action reaches a safe state in time

**If the tool accumulates toward a limit**

- [ ] An invariant or rate limit is declared in the envelope
- [ ] Something resets the accumulator

**If the engine should select the tool by itself**

- [ ] A skill response names an outcome the engine models
- [ ] Or, where none fits, you declared your own in your namespace with a unit
      and a description
- [ ] `per_unit` is signed, and `confidence` reflects measured certainty
- [ ] Retention, settling or hysteresis are declared where they exist

**If the machine should be able to answer questions about it**

- [ ] A `description` on the tool, if another tool could serve the same purpose
- [ ] Notes for anything an owner might ask that the structured data does not say
- [ ] The notes fit the context of the smallest model you ship
- [ ] No targets and no effect magnitudes in any note

---

## What you are not doing

No reasoning. No models. No adoption of another build system, language or
release schedule.

No domain knowledge is required of you either. The profile supplies what makes
espresso good, and your declaration describes your machine. Where you know
something about *your* machine that we could not, notes and your own outcomes
are there for it, and both are optional.

No parser, no validator, no framing, no CRC, no range checking and no envelope
bookkeeping either. `libadip` is freestanding C99 with no allocation and no
floating point, it compiles your declaration into const tables, and it asks you
for three port functions and one handler.

You describe your machine and drive its mechanisms. The engine does the
thinking, and the library does the protocol.
