# ADI SDK

The Artificial Domain Intelligence SDK. A domain-neutral engine that turns a
declared set of physical actions into an intelligent machine, plus espresso as
the first domain built on it.

Specification: [`../PROTOCOL.md`](../PROTOCOL.md).
Plan: [`../ROADMAP.md`](../ROADMAP.md).

---

## Layout

```
sdk/
├── crates/
│   ├── adi-protocol/         Wire types. Declaration, skill, knowledge,
│   │                         invocation. Parsing and validation only.
│   ├── adi-safety/           Safe Action Protocol. Envelope, stateful
│   │                         invariants, rate limits, resource accounting.
│   ├── adi-core/             The reasoning loop. Cycle, targets, diagnosis,
│   │                         correction planning, layer resolution, records.
│   ├── adi-decision/         D0 / D1 / D2 selectors behind one trait.
│   ├── adi-domain-espresso/  First domain profile. Registry and rules.
│   └── adi-cli/              Validator and development driver.
├── c/                        libadip, the C reference SDK for the appliance
│                             side. Design only, not yet implemented.
├── profiles/
│   └── espresso/             Knowledge base as data.
├── conformance/
│   ├── espresso/cases/       Input and expected-decision pairs.
│   └── greenhouse/           Second profile. Data only, no product.
│                             Exists to keep the abstraction honest.
└── examples/declarations/    Sample device declarations.
```

## The one rule

**No domain vocabulary in the neutral crates**, meaning `adi-protocol`,
`adi-safety`, `adi-core` and `adi-decision`.

Precisely: no type, field, variant, function, constant or string literal in
those crates may name anything specific to one domain. A cycle is a cycle. A
subjective rating is a subjective rating. If a neutral crate needs to know it is
making coffee, the abstraction has leaked, and the second domain will expose it
later at much greater cost.

Illustrative mentions in doc comments are **allowed and encouraged**, because
"in one domain a cycle is a coffee extraction, in another a wash" is exactly the
sentence that teaches the abstraction. What matters is that nothing *structural*
depends on the domain.

Check with:

```sh
python check-neutrality.py
```

It strips comments before searching, so it flags real leaks and ignores
explanatory prose. Run it before every commit that touches a neutral crate.

## Autonomy levels

Configuration, not a code path. See `adi-decision`.

| Level | Tool chosen by | Value chosen by |
|---|---|---|
| D0 | Engine | Engine |
| **D1** (default) | Model, from declared tools | Engine |
| D2 | Model | Model |

D2 refuses to initialise against a declaration without stateful invariants and
rate limits. That refusal is deliberate and is the point of the level system.

## Build

Requires a Rust toolchain, which is not currently installed on this machine.

```sh
rustup toolchain install stable
cd sdk
cargo build
cargo test
cargo run -p adi-cli -- validate ../examples/declarations/ex900.yaml
```

## Status

Scaffold. Types and traits encode the architecture; most behaviour is `todo!()`.
Written without a compiler available, so expect to fix compile errors on first
build.

`serde_yaml` 0.9 is unmaintained upstream. If it causes trouble, swap it for a
maintained YAML crate. Nothing in the design depends on which one.
