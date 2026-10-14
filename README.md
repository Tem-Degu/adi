# Artificial Domain Intelligence

Artificial Domain Intelligence is an approach to making physical machines
intelligent and capable within the thing they are built to do (their domain).
A machine measures what it just produced, determines what was wrong with it,
corrects itself, and accepts spoken instruction from the person using it. It
acts on the machine directly instead of offering advice for the person to
carry out.

It also holds the knowledge of its domain, so a person can ask it questions and
receive instruction without consulting anything else. What model it is, why a
result came out as it did, what a particular setting changes. It answers from
what the manufacturer supplied and from the declaration it already reasons over.

Everything runs on the device. No internet connection, no account, no
companion application.

The intelligence is bounded to one domain, meaning the scope of what the
machine exists to do. An espresso machine reasons about extraction while a
greenhouse controller reasons about growth. That boundary is what keeps the
system small enough to run on appliance hardware, specific enough to be useful,
and narrow enough that its behaviour can be tested and certified.

This repository contains the protocol that defines the integration, and a
working implementation of both sides of it. The first domain is espresso.

## Try it

Requires a Rust toolchain and a C compiler. Nothing else, and no network.

```sh
cd sdk
cargo build --release
cd c && make example && cd ..

cargo run -p adi-cli -- run --level d0
```

Type `cycle`:

```
> cycle
  observing    an 18 s shot, described as sour
  diagnosed    shot_time
  chose        adi.espresso.grind.set_setting
  sent         setting = 20 Step
  accepted     by the appliance
  explains     Diagnosed shot_time and set setting to 20.
```

The appliance is a real build of the C library running as a separate process.
That value crossed the same framing, checksum, decoding, range validation and
safety envelope that would run on a machine.

[Running it](#running-it) covers the other levels, speech, and the tests.

---

## How it works

An appliance declares what it can physically do. For an espresso machine that
might be: set the grind, dose by mass, set the brew temperature, start
extraction and stop at a target weight, dispense an additive, eject the puck.
Each declared action states its range, its unit, its resolution, and the
conditions under which it may be used.

The intelligence acts only by invoking actions from that declaration. It
cannot perform an action the manufacturer never built, cannot send a value
outside a declared range, and cannot breach a limit the appliance enforces in
its own firmware. This is the Safe Action Protocol, and its consequence is
that safety is a property of the declared action surface, not of a model
behaving correctly.

A complete cycle:

1. The machine reports what it measured during the operation.
2. The engine derives meaning from those measurements and compares the result
   against the target for what was asked for.
3. Where the result is outside target, it identifies the likely cause.
4. It plans a correction, sized from the manufacturer's own statement of how
   each mechanism affects the result.
5. It checks the whole plan against a safety envelope, including constraints
   that only apply to sequences, such as a container that three individually
   valid additions would overflow.
6. It sends the invocation. The appliance validates it again, independently.
7. It records what it did, including what it wanted where a mechanism was
   already at its limit, so a partial correction is reported as partial rather
   than as a success.
8. It explains the decision in words, and remembers the cycle for the next one.

A language model turns speech into a structured request and phrases the
explanation afterwards. Three autonomy levels determine how much of the
decision it makes; see [Autonomy](#autonomy).

A request is not the only thing a person can say. Asked a question, the machine
answers from the declaration it already reasons over and from whatever the
manufacturer wrote down.

## Integrating a machine

A **tool** is one action the machine can perform: setting the grind,
dispensing milk, ejecting the puck.

Three things make one available: the mechanism, a handler that runs it, and one
entry in a data file. The first two are ordinary appliance engineering and the
third is data rather than code:

**1. Build the mechanism.** A motor, a valve, a heater, driven from a pin.

**2. Decide how the firmware knows it finished.** A limit switch, an encoder
count, a measured mass. An expired timer is not evidence of completion, and
the engine treats a reported completion as fact, so an assumed one leaves it
reasoning about a state the machine is not in.

**3. Write a handler.** One function, called when a named instruction arrives.
It starts the mechanism and returns; it does not block. Long operations report
completion later.

```c
adip_action_t adip_on_invoke(const adip_invocation_t *inv,
                             adip_reason_t *reason, void *user)
{
    if (adip_tool_index(ctx.dev, inv->tool) == TOOL_GRIND_SET) {
        start_grind_move(setting_from(inv));
        return ADIP_ACTION_RUNNING;
    }
    return ADIP_ACTION_DONE;
}
```

By the time this is called the library has confirmed the tool exists, every
argument is present, in range and in the declared unit, no interlock blocks
it, the preconditions hold, and the sequence constraints are satisfied.

**4. Declare it.** One entry in a data file, stating the name, the parameters,
their limits, the description, and the conditions under which the tool may run.

```yaml
tools:
  - id: "adi.espresso.grind.set_setting"
    parameters:
      setting:
        type: integer
        unit: step
        min: 0
        max: 60
        resolution: 1
    preconditions: ["state == idle"]
    duration_ms_max: 6000
    reversible: true
    # What the mechanism is, for a model choosing between two tools that
    # serve the same outcome. Read by no engine logic.
    description: >
      Stepper-driven burr collar. Settles within two seconds, and retains
      roughly 0.4 g of grounds from the previous setting.
```

`adi generate-c` compiles that into constant tables, so a limit exists in one
place and nothing can drift from it.

A tool from the registry needs no `description`, because the profile supplies
one: `adi.espresso.grind.set_setting` means the same on every machine that
declares it. Write your own only where your mechanism differs from the usual, as
above, and it takes precedence.

**A tool of your own must carry one**, and `adi validate` refuses a declaration
that omits it. Your identifier means nothing to anyone else, so a model offered
`x.acme.aux.actuate_3` with a magnitude and a confidence cannot tell what it does.

A description may influence which tool a model picks and never what value is
sent, so one that is wrong or out of date cannot reach the actuator. Keep the
numbers out of it: how far the mechanism moves an outcome belongs in step 5.

**5. Optionally, describe its effect.** A separate file states how much the
mechanism moves a result, which is what lets the engine size a correction
instead of guessing at one. A tool with no such entry remains callable; it is
not selected automatically.

```yaml
responses:
  - tool: "adi.espresso.grind.set_setting"
    parameter: "setting"
    variable: "shot_time"
    per_unit: -1.4          # seconds of shot time per step, signed
    unit: s
    confidence: 0.75
```

**6. Optionally, declare an outcome of your own.** A response has to name an
outcome the engine models, and a tool doing something the profile never
considered moves none of them. Declare one in your own namespace and the engine
can select your tool for a goal on it.

```yaml
variables:
  x.acme.tray_state:
    unit: none
    description: >
      How clean the drip tray is, from 0 for soiled to 1 for clean.
```

This is what makes a new capability reachable.

**7. Optionally, say what you know.** Free text the machine may answer from.

```yaml
notes:
  - id: "x.acme.note.basket"
    text: >
      A 58 mm basket, taking 18 to 20 g. A stepped basket will not seat
      correctly.
```

A note may inform an answer and may never determine an action. Every note
reaches the model on every utterance, so keeping the total within the context of
the smallest model you ship is the author's responsibility.

The manufacturer implements no reasoning and no models. A tool they implement
works the day they declare it, without a release from this project.

Three platform functions complete the integration: a millisecond clock, a byte
write, and a function that drives the machine to a safe state.

See [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) for the full path,
including the electrical steps and the failure cases.

## Autonomy

Once the engine has a request, or has diagnosed something wrong with a cycle,
two decisions remain. Which of the declared actions to use, and what value to
send it. A grind setting can be changed, or a dose, or a brew temperature, and
each takes a number.

The autonomy level states who makes each of those two decisions: the engine,
working deterministically from the manufacturer's declared effects, or the
model. It is configuration, not a separate code path, and it appears in every
decision record.

| Level | Which action | What value |
|---|---|---|
| D0 | engine | engine |
| D1 | model, from the declared actions | engine |
| **D2** (default) | model | model |

The model turns speech into a structured request at every level, including D0,
and explains the outcome afterwards at every level. The level governs only
what happens between those two points.

D2 is the default because an engine choosing on its own can only act on
situations someone expressed as a rule beforehand. A model can act on a
request, or on a cycle, that no rule describes, which is what allows a person
to ask for what they want in their own words instead of selecting from a fixed
set of options.

The safety of a model-chosen value does not depend on the model being right: the
engine clamps it to the declared range and snaps it to the declared resolution,
the safety envelope checks the whole plan, and the appliance validates it
independently.

D2 refuses to initialise against a declaration that states no sequence
constraints, since that is precisely the case where individually valid actions
combine into an invalid one.

## Architecture

Two halves, one protocol.

**On the appliance**, a microcontroller runs `libadip`: framing, decoding,
validation, the stateful envelope and the failsafe, driven by tables compiled
from the manufacturer's declaration.

**On the module**, an applications processor runs the engine:

```
adi-protocol   Wire types. Declaration, skill, knowledge, intent, invocation.
adi-wire       ADI-P 10.2 encoding, verified against libadip.
adi-safety     Safe Action Protocol. Envelope, invariants, rate limits.
adi-core       The reasoning. Cycle, targets, diagnosis, correction.
adi-decision   Autonomy levels D0, D1 and D2 behind one selector trait.
adi-language   The four model seams: transcribe, interpret, explain, speak.
adi-models     Model configuration, provenance, and external backends.
adi-session    The loop. Link, heartbeat, cycle, envelope, record, memory.
adi-domain-*   One domain profile. Registry, knowledge, diagnosis rules.
adi-cli        Validator, code generator and development driver.
```

Every crate except the domain profile and the command line tool is
domain-neutral and must remain so. `sdk/check-neutrality.py` enforces this,
and `sdk/conformance/greenhouse/` tests the abstraction against a domain with
a very different shape: cycles measured in weeks, not seconds, feedback
arriving long after the action, and operations that overlap.

## Hardware

Two sides, with very different requirements.

### The appliance side

Whatever microcontroller the machine already has. `libadip` is freestanding C99
with no allocation, no recursion and no floating point, so an M0 is enough.

| | |
|---|---|
| Flash | 7198 B, measured on Cortex-M4, plus your generated tables |
| RAM | no static or heap allocation. One context of 496 bytes, plus the frame buffers you size |
| Integration | three platform functions and one handler |

The full appliance test suite, 322 checks, passes on an emulated Cortex-M3.
What emulation does not establish is real timing, which matters for the reply
deadline and the failsafe interval, or the platform functions against real
peripherals.

### The module side

The engine is Rust on an applications processor. What governs the choice is not
the reasoning, which is small, but **how long the model takes to interpret a
request**.

| Tier | Board class | Interpreter | Suits |
|---|---|---|---|
| Minimal | Raspberry Pi 5, 8 GB | Qwen 3 1.7B Q4 | Evaluation, and a product where a short pause is acceptable |
| Standard | RK3588 class | Qwen 3 4B Q4 | A premium appliance, where the voice interaction should feel immediate |
| Custom | Whatever you measure | Any | Measured with `adi models bench` |

Published latency figures are measured on other hardware with other prompts, so
they do not transfer. `adi models bench` runs the evaluation corpus against your
configured interpreter on the board and reports how often the answer was correct
and what each request cost. Run it before committing to a board.

Storage depends on which models are installed. A speech recogniser, an
interpreter and a synthesiser together are roughly 1 to 3 GB at the quantisations
above.

The named tiers are measured on general-purpose cores, so an accelerator is an
optimisation and not a prerequisite. A dedicated NPU shortens the interpreter's
response time where a board has one.

A microphone and a speaker are the integrator's to drive.
[`sdk/README.md`](./sdk/README.md) sets out what the SDK handles and what they
supply.

## Running it

Requires a Rust toolchain and a C compiler.

```sh
cd sdk
cargo build --release
cd c && make example && cd ..

cargo run -p adi-cli -- run --level d0
```

This starts the example appliance as a separate process, opens a session
against it, and presents a prompt. Typing `cycle` observes a finished cycle:

```
> cycle
  observing    an 18 s shot, described as sour
  diagnosed    shot_time
  chose        adi.espresso.grind.set_setting
  sent         setting = 20 Step
  accepted     by the appliance
```

The appliance is a real build of the C library running as a separate process.
Every value crosses the same framing, checksum, decoding, range validation and
safety envelope that would run on a machine.

### Running at each level

| | Command | Requires |
|---|---|---|
| D0 | `adi run --level d0` | nothing |
| D1 | `adi run --level d1 --config <file>` | a model backend |
| D2 | `adi run --level d2 --config <file>` | a model backend, and a declaration stating sequence constraints |

D2 is the default, so `adi run` with no level attempts it and refuses if no
model backend is configured. It does not fall back to a lower level, because a
decision record naming a level that did not run would be false.

A model backend is a program the engine starts and exchanges one message per
line with. Backends are supplied, so obtaining the model files and pointing a
configuration at them is the whole of the setup:

```sh
pip install -r requirements.txt
cargo run -p adi-cli -- models check     # names each missing file and its source
# obtain the files it names
cargo run -p adi-cli -- run --config models/models.example-local.yaml
```

`adi models check` reads the folder and reports which seams are served, so the
setup instructions cannot go out of date. See [`sdk/README.md`](./sdk/README.md)
for the model configuration in full, including how to use a model of your own.

### Tests

To run the test suites and the conformance corpus:

```sh
cargo test --workspace
python check-neutrality.py
cargo run -p adi-cli -- conformance conformance/espresso/cases --interpreter conformance/espresso/interpreter.yaml
```

## Documents

| File | Contents |
|---|---|
| [`PROTOCOL.md`](./PROTOCOL.md) | **ADI-P**, the integration contract. Safe Action Protocol, conformance and autonomy levels, the capability declaration, the skill, knowledge extensions, the execution model, the wire format and the espresso registry. |
| [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) | The path through the protocol for a firmware engineer: mechanism, driver, handler, declaration, and the optional skill entry. |
| [`ROADMAP.md`](./ROADMAP.md) | The build plan, in phases. |
| [`sdk/README.md`](./sdk/README.md) | The implementation: layout, build, model configuration and current limitations. |
| [`sdk/c/DESIGN.md`](./sdk/c/DESIGN.md) | `libadip`, the appliance library. Freestanding C99, no allocation, no floating point, 7198 B of flash on a Cortex-M4. |

## Status

Pre-release. The specification is stable, both implementations exchange
messages over a transport, and a cycle runs end to end: observe, diagnose,
plan, check the safety envelope, invoke, record, explain, remember. All three
autonomy levels run against a real appliance, and all four model seams are
reached in a session.

The appliance library occupies 7198 B of flash on a Cortex-M4, with no static
or heap allocation, and its full test suite passes on an emulated Cortex-M3.
Both are reproducible without a board: see [`sdk/README.md`](./sdk/README.md).

Answering a question is not safe in the way acting is. An action is checked
against the declaration, clamped to the declared range and validated
independently by the appliance, so a wrong one is refused. An answer is text
that reaches a person, and nothing verifies it. The mitigation is to answer only
from supplied knowledge and to record what was used, which narrows the risk
without removing it.

Not yet done:

- **No audio capture or playback.** Speech synthesis writes a file and
  transcription reads one. Driving a microphone and a speaker needs a platform
  library and a device, which belongs in a product and not in this SDK.
  `sdk/README.md` sets out what an integrator supplies and what the SDK handles,
  and no worked example against a real audio library is provided yet.
- **Models run beside the engine, not inside it.** The engine starts
  them and exchanges one message per line, so a developer supplies model files
  and writes no code. Backends that load models through ONNX Runtime and
  llama.cpp directly are not implemented.
- **The espresso perception rules have not been validated against real
  extractions.** Every trace in the conformance corpus is synthetic.
- **No port to physical hardware.** `libadip` compiles for Cortex-M and
  passes its tests under emulation. Real timing and real peripherals remain
  unverified.

## Licence

Apache 2.0. See [`LICENSE`](./LICENSE).

ADI-P may be implemented by anyone. Conformance certification and the
Artificial Domain Intelligence mark are separate, and are not granted by this
licence.
