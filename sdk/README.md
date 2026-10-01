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

## Quickstart

```sh
# 1. Build the engine and the example appliance
cargo build --release
cd c && make example && cd ..

# 2. Drive a real appliance
cargo run -p adi-cli -- run --level d0
```

At the prompt, type `cycle`. The engine observes a finished cycle, diagnoses
it, plans a correction, checks it against the safety envelope, and sends it to
a real `libadip` build running as a separate process. Type `help` for the
other commands.

`--level d0` is the deterministic level and needs no model. The default is
D2; see [Autonomy levels](#autonomy-levels).

### Adding speech

```sh
pip install -r requirements.txt
adi models check                 # reports each missing file and its source
# obtain the three model files it names
cargo run -p adi-cli -- run --config models/models.example-local.yaml
```

This runs at D2, the default: a model turns speech into a structured request,
chooses which declared action serves it, and proposes the value. The engine
clamps and checks everything before it is sent.

---

## Layout

```
sdk/
├── crates/                   The engine side
│   ├── adi-protocol/         Wire types. Declaration, skill, knowledge,
│   │                         intent, invocation. Parsing and validation.
│   ├── adi-wire/             ADI-P 10.2 message encoding. CBOR and framing,
│   │                         verified against libadip.
│   ├── adi-safety/           Safe Action Protocol. Envelope, stateful
│   │                         invariants, rate limits, resource accounting.
│   ├── adi-core/             The reasoning. Cycle, targets, diagnosis,
│   │                         correction planning, layer resolution, records.
│   ├── adi-decision/         D0, D1 and D2 selectors behind one trait.
│   ├── adi-language/         The four model seams: transcribe, interpret,
│   │                         explain, speak. Placeholders that run today.
│   ├── adi-session/          The loop. Holds the link, sends the heartbeat,
│   │                         runs a cycle, enforces the envelope.
│   ├── adi-models/           Model configuration, provenance, output schema,
│   │                         and the backend that runs an external program.
│   ├── adi-domain-espresso/  First domain profile. Registry and rules.
│   └── adi-cli/              Validator, generator and development driver.
├── c/                        libadip, the appliance SDK. See c/DESIGN.md.
│   ├── example/              A complete working appliance, to copy.
│   └── generated/            Tables compiled from a declaration.
├── models/                   Where model weights go. Ships empty; see Models.
├── eval/espresso/            Utterances and reference answers, for measuring
│                             an interpreter with `adi models bench`.
├── examples/interpreters/    Model backends that are separate programs.
│                             reference*.py implement the protocol with no
│                             models; qwen_interpreter.py and
│                             speech_backends.py run real ones.
├── requirements.txt          Python packages those backends need.
├── profiles/espresso/        Knowledge base as data.
├── conformance/
│   ├── espresso/cases/       Input and expected-decision pairs.
│   └── greenhouse/           Second profile. Data only, no product.
│                             Verifies the abstraction against a second domain.
└── examples/                 Sample declarations, skills and knowledge.
```

## The one rule

**No domain vocabulary in the neutral crates**, meaning `adi-protocol`,
`adi-wire`, `adi-safety`, `adi-core`, `adi-decision`, `adi-language`,
`adi-session` and `adi-models`.

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

Comments are stripped before searching, so the check flags structural
dependencies and ignores explanatory prose. Run it before any commit that
touches a neutral crate. It is not advisory: the loopback tests were relocated
out of `adi-wire` because the example appliance they drive is an espresso
machine.

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

A model-driven level needs a **chooser**: the component that asks a model which
declared action serves a goal, and at D2 what value to send. `ProcessChooser`
supplies one over an external program, configured exactly as the other model
seams are.

Requesting D1 or D2 without a chooser is refused rather than served at D0. A
decision record naming a level that did not run would be false, and the record
is what the audit argument rests on.

```sh
adi run --level d0                                         # no model required
adi run --level d2 --config models/models.example-process.yaml
```

The chooser runs the same program as the interpreter, since an appliance with
one model resident answers both questions from it.

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

## Models

The SDK ships no model weights. They are obtained from the parties that publish
them, under those parties' licences, and placed in `models/`. There is no
downloader: this avoids redistributing third-party weights, keeps network code
out of a product that does not otherwise require it, and makes substituting a
different model the ordinary path rather than a special case.

### Obtaining them

```sh
adi models check --config models/models.yaml
```

This reads the folder and reports which seams are served, which files are
present, whether each matches a published checksum, and for anything absent,
the file to obtain and its source. It is generated from the folder rather than
written down, so it cannot describe a state the machine is not in.

`models/manifest.yaml` lists the recommended files with their sources,
licences and checksums. `models/models.yaml` states which file serves which
seam.

### Recommended models

| Seam | Model | Licence |
|---|---|---|
| Interpreter | Qwen 3 1.7B or 4B, GGUF | Apache-2.0 |
| Transcriber | Moonshine base, or Whisper base | MIT |
| Speaker | Piper | MIT |

Llama models are not listed. The Llama licence requires permission from Meta
above a usage threshold and withholds multimodal rights from companies
domiciled in the European Union. A manufacturer licensing this SDK should not
inherit either condition.

### Selecting hardware

Interpreter latency on a processor without an accelerator is the constraint
that governs whether a voice interaction is usable, and published figures are
measured on other hardware with other prompts.

```sh
adi models bench
```

This runs the evaluation corpus in `eval/espresso/` against the configured
interpreter on the host it is invoked on, and reports how often the answer was
correct and what each request cost. Run it before selecting a board.

Three tiers are published as a default for anyone who does not wish to
evaluate:

| Tier | Board class | Interpreter |
|---|---|---|
| Minimal | Raspberry Pi 5, 8 GB | Qwen 3 1.7B Q4 |
| Standard | RK3588 class | Qwen 3 4B Q4 |
| Custom | Measured with `adi models bench` | Any |

### Using a different model

Three levels, in increasing order of effort.

**Change the configuration.** Point `models/models.yaml` at a different file.
A model absent from the manifest loads normally and is recorded as unverified,
identified by its checksum. Verification exists so that a decision can be
attributed to a specific model afterwards, as ADI-P 9.3 requirement 5 requires,
not to restrict what may be loaded.

**Use another program.** The `process` backend runs an external program and
exchanges one JSON request and one JSON answer per line over its standard
input and output. It serves the interpreter, transcriber and speaker seams.
The program requires no dependency on this SDK and may be written in any
language.

```
interpreter  -> {"utterance":"half of last time","styles":[...]}
             <- {"intent":{"kind":"produce","targets":{...}}}
             or {"clarify":{"question":"...","because":"..."}}

transcriber  -> {"audio":"<base64>","sample_rate":16000}
             <- {"text":"...","confidence":0.91}

speaker      -> {"text":"..."}
             <- {"audio":"<base64>","sample_rate":22050}
```

Audio is base64 of 16-bit signed, little endian, single channel samples.

Two sets of backends are supplied.

`examples/interpreters/reference.py` and
`examples/interpreters/reference_speech.py` implement the
protocol and contain no models. They are the specification by example, and are
used by the test suite.

`examples/interpreters/qwen_interpreter.py` and
`examples/interpreters/speech_backends.py` run real models, and are configured by `models/models.example-local.yaml`. They have
been written from the libraries' documentation and **not yet verified against
real weights**, because the machine they were written on cannot hold them.

`models/models.example-process.yaml` configures the reference set.

**Implement the trait.** `adi-language` defines the four seams as traits. A
manufacturer shipping at volume implements them directly against their own
runtime.

### Running without models

An engine with an unserved seam is a supported configuration. It refuses the
operations that seam provides and names the seam, rather than failing. The
explainer works without any model, producing text from the decision record.

---

## Status

Pre-release. The specification is stable, both implementations exchange
messages over a transport, and a cycle runs end to end: observe, diagnose,
plan, check the safety envelope, invoke, record, remember. All three autonomy
levels run against a real appliance.

The appliance library occupies roughly 12 KB of flash with no static or heap
allocation, against the 16 KB budget stated in [`c/DESIGN.md`](./c/DESIGN.md).
That figure is measured on x86-64 and is expected to be smaller on a
Cortex-M target.

### Current limitations

- **No model runs inside the engine.** The `llama` and `onnx` backends are
  named in the configuration format and have no implementation; `adi models
  check` reports this rather than reporting a present model file as ready.
  Three of the four seams can be served today by an external program through
  the `process` backend, and the fourth, the explainer, needs no model.
- **The espresso perception rules have not been validated against real
  extractions.** Every trace in the conformance corpus is synthetic. Synthetic
  traces are plausible by construction, which is the condition under which an
  incorrect rule is least likely to be detected.
- **Diagnosis considers one variable.** The knowledge layer resolves ratio and
  temperature targets, and `diagnose` does not yet use them.
- **No learning.** `Layer::User` is defined and nothing writes to it.
- **No microcontroller port.** `libadip` is tested on a host. Milestone M6 in
  [`c/DESIGN.md`](./c/DESIGN.md) requires a target board, and the footprint
  above is an x86-64 measurement.
- **The decoder has not been fuzzed**, and no MISRA review has been performed.

`serde_yaml` 0.9 is unmaintained upstream. No part of the design depends on
which YAML implementation is used.
