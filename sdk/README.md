# ADI SDK

The Artificial Domain Intelligence SDK. A domain-neutral engine that turns a
declared set of physical actions into an intelligent machine, plus espresso as
the first domain built on it.

Two halves that speak one protocol. The engine side is Rust and runs on an
applications processor. The appliance side is `libadip`, freestanding C99, and
runs on whatever microcontroller the machine already has.

Specification: [`../PROTOCOL.md`](../PROTOCOL.md).
Plan: [`../ROADMAP.md`](../ROADMAP.md).

---

## Layout

```
sdk/
├── crates/                   The engine side
│   ├── adi-protocol/         Wire types. Declaration, skill, knowledge,
│   │                         intent, invocation. Parsing and validation.
│   ├── adi-wire/             ADI-P 10.2 message encoding. CBOR and framing,
│   │                         written to match libadip byte for byte.
│   ├── adi-safety/           Safe Action Protocol. Envelope, stateful
│   │                         invariants, rate limits, resource accounting.
│   ├── adi-core/             The reasoning. Cycle, targets, diagnosis,
│   │                         correction planning, layer resolution, records.
│   ├── adi-decision/         D0, D1 and D2 selectors behind one trait.
│   ├── adi-language/         The four model seams: transcribe, interpret,
│   │                         explain, speak. Placeholders that run today.
│   ├── adi-session/          The loop. Holds the link, sends the heartbeat,
│   │                         runs a cycle, enforces the envelope.
│   ├── adi-domain-espresso/  First domain profile. Registry and rules.
│   └── adi-cli/              Validator, generator and development driver.
├── c/                        libadip, the appliance SDK. See c/DESIGN.md.
│   ├── example/              A complete working appliance, to copy.
│   └── generated/            Tables compiled from a declaration.
├── profiles/espresso/        Knowledge base as data.
├── conformance/
│   ├── espresso/cases/       Input and expected-decision pairs.
│   └── greenhouse/           Second profile. Data only, no product.
│                             Exists to keep the abstraction honest.
└── examples/                 Sample declarations, skills and knowledge.
```

## The one rule

**No domain vocabulary in the neutral crates**, meaning `adi-protocol`,
`adi-wire`, `adi-safety`, `adi-core`, `adi-decision`, `adi-language` and
`adi-session`.

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
explanatory prose. Run it before every commit that touches a neutral crate. It
has caught a real leak as recently as the loopback tests, which had to move out
of `adi-wire` because the example appliance they drive is an espresso machine.

## Autonomy levels

Configuration, not a code path. See `adi-decision`.

| Level | Tool chosen by | Value chosen by |
|---|---|---|
| D0 | Engine | Engine |
| D1 | Model, from declared tools | Engine |
| **D2** (default) | Model | Model |

The model turns speech into structured intent at every level, including D0, and
explains afterwards at every level. What the level grades is what happens after
intent.

D2 is the default because a machine that can only do what someone anticipated in
advance is a program with a microphone attached. It refuses to initialise
against a declaration without stateful invariants and rate limits, and that
refusal is the point of the level system rather than an inconvenience.

D1 currently has no model backend and `selector_for` says so rather than
silently running at D0.

## Build

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
python check-neutrality.py

cargo run -p adi-cli -- validate examples/declarations/ex900.yaml \
                                 --skill examples/declarations/ex900.skill.yaml
cargo run -p adi-cli -- capabilities examples/declarations/ex900.yaml \
                                     --skill examples/declarations/ex900.skill.yaml
cargo run -p adi-cli -- conformance conformance/espresso/cases
```

The appliance side needs a C compiler:

```sh
cd c
make test        # unit tests, no hardware, no platform
make example     # a working appliance the engine can talk to
make size        # flash and RAM against the budget in DESIGN.md
```

With the example built, the loopback tests run the engine against a real
`libadip` build as a child process:

```sh
cargo test -p adi-cli --test loopback
```

Without it they skip rather than fail, so `cargo test` does not require a C
toolchain.

## Status

| | |
|---|---|
| Rust tests | 222 |
| C checks | 322 across six binaries |
| Conformance | 8 passing, 1 pending |
| Appliance footprint | 12212 B flash, 0 data, 0 bss, against a 16 KB budget |

The specification is complete and the two implementations talk to each other
over a real wire. A cycle runs end to end: observe, diagnose, plan, check the
envelope, invoke, record, remember.

**What is not done**, so nobody has to discover it by reading:

- **No model backends.** The four seams in `adi-language` have working
  placeholders and no real transcriber, interpreter or speaker behind them. The
  one pending conformance case is the one that needs them.
- **The perception rules have never seen a real shot.** Every trace in the
  corpus is hand-written. They are plausible, which is the condition under which
  they are most likely to be quietly wrong.
- **Diagnosis reads one variable.** The knowledge layer resolves ratio and
  temperature targets correctly and `diagnose` ignores them.
- **Nothing learns.** `Layer::User` is a variant nothing writes.
- **No MCU port.** `libadip` is tested on a host. M6 in `c/DESIGN.md` needs a
  board, and the footprint above is an x86-64 figure that Thumb-2 should
  improve on.
- **The decoder is not fuzzed** and there has been no MISRA pass.

`serde_yaml` 0.9 is unmaintained upstream. Nothing in the design depends on
which YAML crate is used.
