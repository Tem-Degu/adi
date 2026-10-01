# Artificial Domain Intelligence

An embedded intelligence layer that appliance manufacturers integrate into their
machines. The machine measures what it just produced, works out what went wrong,
corrects itself, and takes instruction in plain speech. Everything runs on the
device, with no internet connection, no account and no application.

It is bounded to one domain. A domain is the scope of what the machine is for.
The narrow scope is what makes it small enough to run on appliance hardware,
testable enough to certify, and cheap enough to ship.

The first domain is espresso.

---

## How it works

An appliance declares what it can physically do: set the grind, dose by mass,
start extraction, stop at a target weight, dispense milk, eject the puck. Each
declared action carries its own range, its unit, its resolution and the
conditions under which it may be used.

The intelligence acts only by invoking actions from that list. It cannot
perform an action the manufacturer never built, cannot send a value outside a
declared range, and cannot breach a limit the appliance enforces in its own
firmware. This is the Safe Action Protocol: safety follows from the shape of
the action surface rather than from the model behaving well.

The engine observes what the machine produced, identifies what was wrong,
plans a correction, checks the whole plan against a safety envelope, and
records every value it used along with the layer it came from. A language
model turns speech into a structured request and phrases an explanation
afterwards. It does not choose which action to take at the lower autonomy
levels, and the values it proposes at the highest level are clamped to the
declared range and validated independently by the appliance.

A manufacturer describes their machine in two data files and implements the
actions in their existing firmware, in their own language, on their own
release schedule. They implement no reasoning, no models and no domain
knowledge. An action they invent works the day they declare it, without a
release from us.

## Architecture

Two halves, one protocol.

**On the appliance**, a microcontroller runs `libadip`: framing, decoding,
validation, the stateful envelope and the failsafe, driven by tables compiled
from the manufacturer's own declaration. The manufacturer implements three
port functions and one invocation handler.

**On the module**, an applications processor runs the engine:

```
adi-protocol   Wire types. Declaration, skill, knowledge, intent, invocation.
adi-wire       ADI-P 10.2 encoding, verified against libadip.
adi-safety     Safe Action Protocol. Envelope, invariants, rate limits.
adi-core       The reasoning. Cycle, targets, diagnosis, correction.
adi-decision   Autonomy levels D0, D1 and D2 behind one selector trait.
adi-language   The four model seams: transcribe, interpret, explain, speak.
adi-session    The loop. Link, heartbeat, cycle, envelope, record, memory.
adi-domain-*   One domain profile. Registry, knowledge, diagnosis rules.
adi-cli        Validator, code generator and development driver.
```

All but the domain profile and the CLI are domain-neutral and must stay that
way. `sdk/check-neutrality.py` enforces it, and `sdk/conformance/greenhouse/`
exists to verify the abstraction against a domain with a very different
shape.

## What is in this repository

The specification, both halves of the implementation, a worked example
appliance, and the conformance corpus that holds them to the specification.

| File | What it is |
|---|---|
| [`PROTOCOL.md`](./PROTOCOL.md) | **ADI-P**, the integration contract. Safe Action Protocol, conformance and autonomy levels, the capability declaration, the skill, knowledge extensions, the execution model and the espresso registry. |
| [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) | The practical path through the protocol for a firmware engineer: mechanism, driver, C handler, declaration, and the optional skill entry. |
| [`ROADMAP.md`](./ROADMAP.md) | The build plan, in phases with gates. |
| [`sdk/`](./sdk/) | The implementation. A domain-neutral engine plus espresso as the first profile. |
| [`sdk/c/`](./sdk/c/) | **`libadip`**, the appliance SDK. Freestanding C99, no allocation, no floating point, around 12 KB of flash. This is the half that runs on the machine. |


The engine runs a complete cycle today: observation, diagnosis, planning,
safety enforcement, invocation, recording and memory. The appliance SDK
implements the full protocol and is exercised against the engine over a real
transport on every build. See [Status](#status) for what is not finished.

## Running it

Requires a Rust toolchain and a C compiler.

```sh
cd sdk
cargo build --release
cd c && make example && cd ..

cargo run -p adi-cli -- run
```

This starts the example appliance as a separate process, opens a session
against it, and presents a prompt. Typing `cycle` observes a finished cycle:
the profile diagnoses it, the engine plans a correction, the safety envelope
checks the plan, and the appliance accepts the invocation. No model is
required, because the reasoning path is deterministic.

Speech requires three model files and a configuration flag. See
[`sdk/README.md`](./sdk/README.md).

To run the test suites and the conformance corpus:

```sh
cargo test --workspace
python check-neutrality.py
cargo run -p adi-cli -- conformance conformance/espresso/cases --interpreter conformance/espresso/interpreter.yaml
```

One corpus case carries a spoken request. Without `--interpreter` it is
reported as pending rather than failing, since interpreting speech requires a
model backend.

## Status

The specification is complete. Both implementations exchange messages over a
transport, and a cycle runs end to end: observe, diagnose, plan, check the
safety envelope, invoke, record, remember.

| | |
|---|---|
| Rust tests | 298 |
| C checks | 322 |
| Conformance corpus | 9 passing |
| Appliance footprint | 12212 B flash, 0 data, 0 bss |

Current limitations:

- **No model runs inside the engine.** Models run as separate programs
  alongside it, started by the engine and exchanging one message per line.
  Backends for speech, understanding and voice are supplied, so a developer
  writes no code, but the two that load real models have not yet been run
  against real weights. In-process llama.cpp and ONNX Runtime backends are not
  written; see `sdk/README.md`.
- **The espresso perception rules have not been validated against real
  extractions.** Every trace in the conformance corpus is synthetic.
- **No microcontroller port.** `libadip` is tested on a host. The footprint
  above is an x86-64 measurement.

See [`ROADMAP.md`](./ROADMAP.md) and [`sdk/README.md`](./sdk/README.md) for
detail.

## Licence

Apache 2.0. See [`LICENSE`](./LICENSE).

ADI-P may be implemented by anyone. Conformance certification and the
Artificial Domain Intelligence mark are separate, and are not granted by this
licence.
