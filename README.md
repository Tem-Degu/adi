# Artificial Domain Intelligence

Artificial Domain Intelligence is an approach to making physical machines
capable within the thing they are built to do. A machine measures what it just
produced, determines what was wrong with it, corrects itself, and accepts
spoken instruction from the person using it. It acts on the machine directly
rather than offering advice for the person to carry out.

Everything runs on the device. No internet connection, no account, no
companion application.

The intelligence is bounded to one domain, meaning the scope of what the
machine exists to do. An espresso machine reasons about extraction. A
greenhouse controller reasons about growth. Neither reasons about anything
else. That boundary is what keeps the system small enough to run on appliance
hardware, specific enough to be useful, and narrow enough that its behaviour
can be tested and certified.

This repository contains the protocol that defines the integration, and a
working implementation of both sides of it. The first domain is espresso.

---

## The problem it addresses

An appliance has sensors and actuators and no understanding of what it is
producing. A person operating it has the understanding and no direct access to
the mechanism. The gap between them is closed by experience, and most people
do not have it.

Existing approaches close that gap in two ways, neither sufficient. Connected
appliances move the decision to a server, which requires a network, an account
and the manufacturer's continued participation. Assistants describe what the
person should do, leaving the person to perform it.

Artificial Domain Intelligence places the reasoning on the device, in the
domain the machine already operates in, with the authority to act within
limits the manufacturer sets.

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

## What a manufacturer provides

Two data files and an implementation of the actions in their existing
firmware, in whatever language and on whatever release schedule they already
use.

They implement no reasoning, no models and no domain knowledge. An action they
invent works the day they declare it, without a release from this project.

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

The appliance library compiles that declaration into constant tables and
handles framing, decoding, validation, the safety envelope and the failsafe.
The manufacturer implements three platform functions and one handler that
drives the mechanism.

## Autonomy

Three levels, which are configuration rather than separate code paths.

| Level | Which action | What value |
|---|---|---|
| D0 | engine | engine |
| D1 | model, from the declared actions | engine |
| **D2** (default) | model | model |

The model turns speech into a structured request at every level, and explains
afterwards at every level. What the level determines is who decides what to do
about it.

D2 is the default because a machine limited to what someone anticipated in
advance cannot respond to a request nobody wrote down. The safety of a
model-chosen value does not depend on the model being right: the engine clamps
it to the declared range and snaps it to the declared resolution, the safety
envelope checks the whole plan, and the appliance validates it independently.
A wrong value produces a refused plan.

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

`--level d0` is the deterministic level and requires no model. The default is
D2, which requires a model backend and refuses rather than running at a level
below the one it reports.

Speech and the model-driven levels need model files and a configuration flag.
See [`sdk/README.md`](./sdk/README.md).

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
