# Artificial Domain Intelligence

Artificial Domain Intelligence is an approach to making physical machines
intelligent and capable within the thing they are built to do (their domain).
A machine measures what it just produced, determines what was wrong with it,
corrects itself, and accepts spoken instruction from the person using it. It
acts on the machine directly rather than offering advice for the person to
carry out.

It also holds the knowledge of its domain, so a person can ask it questions
and receive instruction without consulting anything else. Why the last result
came out as it did, what a particular setting changes, how to prepare for what
they are about to make. The machine answers from the same knowledge it uses to
correct itself, so the explanation and the action agree.

Everything runs on the device. No internet connection, no account, no
companion application.

The intelligence is bounded to one domain, meaning the scope of what the
machine exists to do. An espresso machine reasons about extraction while 
greenhouse controller reasons about growth. That boundary is what keeps the 
system small enough to run on appliance
hardware, specific enough to be useful, and narrow enough that its behaviour
can be tested and certified.

This repository contains the protocol that defines the integration, and a
working implementation of both sides of it. The first domain is espresso.

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
that safety is a property of the declared action surface rather than of a
model behaving correctly.

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
7. It records every value that contributed to the decision, along with where
   that value came from, and remembers the cycle for the next one.

A language model turns speech into a structured request and phrases the
explanation afterwards. Three autonomy levels determine how much of the
decision it makes; see [Autonomy](#autonomy).

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

**4. Declare it.** One entry in a data file, stating the name, the
parameters, their limits, and the conditions under which the tool may run.

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
```

`adi generate-c` compiles that into constant tables, so a limit exists in one
place and nothing can drift from it.

**5. Optionally, describe its effect.** A separate file states how much the
mechanism moves a result, which is what lets the engine size a correction
rather than guess at one. A tool with no such entry remains callable; it is
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

The manufacturer implements no reasoning, no models and no domain knowledge. A
tool they implement works the day they declare it, without a release from this
project.

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
model. It is configuration rather than a separate code path, and it appears in
every decision record.

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
to ask for what they want in their own words rather than select from a fixed
set of options. The safety of a model-chosen value does not depend on the model being right:
the engine clamps it to the declared range and snaps it to the declared
resolution, the safety envelope checks the whole plan, and the appliance
validates it independently. 

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
a very different shape: cycles measured in weeks rather than seconds, feedback
arriving long after the action, and operations that overlap.

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
| [`sdk/c/DESIGN.md`](./sdk/c/DESIGN.md) | `libadip`, the appliance library. Freestanding C99, no allocation, no floating point, around 12 KB of flash. |

## Status

Pre-release. The specification is stable, both implementations exchange
messages over a transport, and a cycle runs end to end: observe, diagnose,
plan, check the safety envelope, invoke, record, remember. All three autonomy
levels run against a real appliance.

The appliance library occupies roughly 12 KB of flash with no static or heap
allocation, measured on x86-64.

Not yet done:

- **Models run beside the engine rather than inside it.** The engine starts
  them and exchanges one message per line, so a developer supplies model files
  and writes no code. Backends that load models through ONNX Runtime and
  llama.cpp directly are not implemented.
- **The espresso perception rules have not been validated against real
  extractions.** Every trace in the conformance corpus is synthetic.
- **No microcontroller port.** `libadip` is developed and tested on a host.

## Licence

Apache 2.0. See [`LICENSE`](./LICENSE).

ADI-P may be implemented by anyone. Conformance certification and the
Artificial Domain Intelligence mark are separate, and are not granted by this
licence.
