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

## Documents

| File | What it is |
|---|---|
| [`PROTOCOL.md`](./PROTOCOL.md) | **ADI-P**, the integration contract. Safe Action Protocol, conformance and autonomy levels, the capability declaration, the skill, knowledge extensions, the execution model and the espresso registry. |
| [`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) | The practical path through the protocol for a firmware engineer: mechanism, driver, C handler, declaration, and the optional skill entry. |
| [`ROADMAP.md`](./ROADMAP.md) | The build plan, in phases with gates. |
| [`sdk/`](./sdk/) | The implementation. A domain-neutral engine plus espresso as the first profile. |

## The idea in one paragraph

A machine publishes a list of what it can physically do. Set the grind, dose by
mass, start extraction, stop at weight, dispense milk, eject. The intelligence
acts by invoking those declared actions and nothing else. It cannot take an
action nobody built, cannot exceed a declared range, and cannot breach a limit
the appliance enforces in its own firmware. That is Safe Action Protocol, and it
means safety is a property of the action surface rather than of the model
behaving well.

A manufacturer describes their machine in two data files and implements the
actions in their existing firmware, in their own language, on their own
schedule. No reasoning, no models and no domain knowledge are implemented on
their side. Anything they invent works the day they declare it, with no release
from us.

## Architecture

```
adi-protocol   Wire types. Declaration, skill, knowledge, invocation.
adi-safety     Safe Action Protocol. Envelope, invariants, rate limits.
adi-core       The reasoning loop. Cycle, targets, diagnosis, correction.
adi-decision   Autonomy levels D0, D1 and D2 behind one selector trait.
adi-domain-*   One domain profile. Registry, knowledge, diagnosis rules.
adi-cli        Validator and development driver.
```

The first four are domain-neutral and must stay that way. `sdk/check-neutrality.py`
enforces it, and `sdk/conformance/greenhouse/` exists to keep the abstraction
honest against a domain with a very different shape.

## Build

Requires a Rust toolchain.

```sh
cd sdk
cargo test --workspace
python check-neutrality.py
cargo run -p adi-cli -- validate examples/declarations/ex900.yaml \
                                --skill examples/declarations/ex900.skill.yaml
```

## What is public

Public: `PROTOCOL.md`, `HOW-TO-ADD-A-TOOL.md`, `ROADMAP.md`, `LICENSE`, this
file, and the `sdk/` crates listed above.

Not public, and excluded by [`.gitignore`](./.gitignore):

- `Decks/` and `uae-investor-targets.txt`, which are fundraising material
- `WHITEPAPER.md` and `BUILD-GUIDE.md`, which are superseded internal documents
  that contradict the current work in several places

Before publishing, verify the split:

```sh
git ls-files | grep -Ei 'investor|wefunder|deck|whitepaper|build-guide'
```

That must return nothing.

## Status

The specification is complete. The engine builds and passes its test suite
against example machine definitions, including the sequence-constraint cases
that Safe Action Protocol turns on. The espresso domain is implemented on it:
measurement, diagnosis, fault detection and correction planning.

Outstanding: hardware integration, trials with users on production machines, and
a first manufacturer agreement. See [`ROADMAP.md`](./ROADMAP.md).

## Licence

Apache 2.0. See [`LICENSE`](./LICENSE).

ADI-P may be implemented by anyone. Conformance certification and the
Artificial Domain Intelligence mark are separate, and are not granted by this
licence.
