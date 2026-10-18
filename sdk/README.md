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

The machine also answers questions, which actuates nothing:

```
> what model are you
  answers      Example Appliances EX-900, firmware 3.2.1

> why was it sour
  answers      Sour usually means under-extraction: the water passed through
               too quickly to dissolve enough...
  from         adi.espresso.note.sour_vs_bitter
```

It answers from the declaration it already reasons over and from notes the
knowledge base carries, and names what it drew on. `listen <file.wav>`
transcribes a recorded utterance and treats it as a request.

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
│   ├── adi-domain-base/      The profile for a machine whose domain nobody
│   │                         has written. No knowledge of its own, and one
│   │                         generic correction.
│   ├── adi-domain-espresso/  First real domain profile. Registry and rules.
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

## Which profile a machine runs on

Every declaration states its `domain_profile`, and that is what the tools load.
Two are compiled in.

| | |
|---|---|
| `espresso/v1` | The first real profile. Knows what a good extraction is, and which faults look alike |
| `base/v1` | No knowledge of its own. Everything comes from the manufacturer's knowledge extension |

`base/v1` is what makes the SDK usable before a profile exists for an industry.
A machine declaring it supplies its outcomes, its targets and its notes in one
file, and gets spoken requests, answers and a correction for anything that
drifts outside its target. What it does not get is a cause: separating two
faults that look alike is what a profile holds. See
[`../HOW-TO-ADD-A-DOMAIN.md`](../HOW-TO-ADD-A-DOMAIN.md), and
`examples/declarations/printer.*` for a machine that is not a coffee machine.

A profile has to be compiled in to be selectable, and a declaration naming one
this build does not carry is reported rather than guessed at. `adi` is a
development driver; a product links the engine and names its own profile.

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
python3 check-neutrality.py
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

D2 is the default because an engine choosing on its own can only act on
situations someone expressed as a rule beforehand, and a model can act on a
request, or a cycle, that no rule describes. It refuses to initialise against a
declaration without stateful invariants and rate limits, and that refusal is the
point of the level system rather than an inconvenience.

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
one model resident answers both questions from it. A backend therefore serves
two seams and has to tell them apart: a selection request carries `candidates`
and a spoken one does not. Both shipped backends branch on that, and
`cargo test -p adi-cli --test shipped_backends` holds them to it without
needing the weights.

The latency table above measures one of those two questions. Interpreting an
utterance has been timed against real weights; choosing a mechanism has not,
so D1 and D2 carry no measured figure.

## What a machine declares

Three files, and only the first is required.

**The declaration** says what the machine can physically do. Each tool carries
its parameters with units, ranges and resolutions, its preconditions, and a
`description`: prose for a model choosing between two tools that serve the same
purpose.

A registry tool need not write one, because the profile supplies it: an
identifier in the registry means the same thing on every machine that declares
it. Writing your own is worth it where your mechanism differs from the usual, and
it takes precedence. A **vendor tool with a skill response must carry one**, and
`adi validate` refuses a declaration that omits it, because a model handed
`x.acme.aux.actuate_3` with a magnitude and a confidence has no way to tell what
it does.

A description may influence which candidate is picked and may never affect which
candidates exist or what value is sent, so one that is wrong, stale or
promotional cannot reach the actuator.

**The skill** says how the machine behaves: how far one unit of a parameter
moves an outcome, signed, with a confidence. This is what makes a tool
*selectable* rather than merely callable.

**A knowledge extension** is optional and holds three things a manufacturer may
add:

| | |
|---|---|
| Narrower ranges | A machine that cannot hold 96 degrees says so. Layers may narrow and never widen |
| Their own outcomes | Namespaced, with a unit and a meaning, for a tool that moves nothing the profile models |
| Notes | Free text the machine may answer from |

Declaring an outcome is what makes a genuinely new capability reachable. A tool
that wipes a tray moves nothing the espresso profile models; declare
`x.acme.tray_state` with a unit and a description, name it in a skill response,
and the engine can select the tool for a goal on it. Validation requires the
identifier to sit in the vendor's namespace and refuses any redefinition of a
base identifier.

### Outcomes are data

The variables a profile models are declared in its knowledge base, each with a
unit and a statement of what it means, and that declaration is authoritative:
the profile reads its list from there rather than holding one. Every style
target is checked against it, both that the outcome exists and that the target
is stated in the outcome's unit.

The meanings are sent to the model. A variable identified only by name can be
mapped onto a request only where the model was trained on the domain, which is
not a property a profile may assume, and is certainly not true of a
manufacturer's own.

### Notes

Every note reaches the model on every utterance. There is no relevance filter,
because filtering is a guess about what a question needs and a wrong guess
withholds the answer silently.

**Keeping the total within the context of the smallest model you target is
therefore your responsibility.** The engine applies its budget, discards notes
before anything a request depends on, and prints a warning when it has to.

A note may inform an answer and may never determine an action. Supplying notes
changes nothing about how a request is validated. A note must not state a target
or an effect magnitude, because those belong in a style and in the skill where
they are validated, and a figure written twice will diverge.

## Build

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
python3 check-neutrality.py

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

All four seams are reached in a session. A cycle is explained and spoken, an
answer and a question are spoken, and `listen <file.wav>` transcribes a recorded
utterance and treats it as a request.

Speech output is written to `spoken.wav` rather than played, and input is read
from a file rather than captured. Playback and capture need an output device, a
microphone and a platform library, which is a packaging concern for a product
rather than part of the protocol. What the SDK demonstrates is that text reaches
the seam and audio comes back.

| Seam | Model | Licence | Status |
|---|---|---|---|
| Interpreter | Qwen 3 4B Q4_K_M, GGUF | Apache-2.0 | verified, see latency below |
| Transcriber | Moonshine base | MIT | verified |
| Speaker | Piper, en_GB-alba-medium | MIT | verified |

Verified means the shipped backend was run against the real weights and
produced correct output on this corpus. `models/manifest.yaml` carries the
checksums and what each was measured to do.

Qwen 3 1.7B is listed in the manifest and **not recommended**. On the shipped
corpus it reached 3 of 18, and it never asked a question where asking was the
correct answer. That is the wrong direction to fail in, because an ambiguous
request then becomes a physical action.

Llama models are not listed. The Llama licence requires permission from Meta
above a usage threshold and withholds multimodal rights from companies
domiciled in the European Union. A manufacturer licensing this SDK should not
inherit either condition.

### Selecting hardware

**Interpreter latency is the open problem in this SDK, and it is unsolved.**

Measured on four shared cloud cores with no accelerator, each backend kept
running as the engine keeps them:

| Seam | Model | Steady-state latency | Verdict |
|---|---|---|---|
| Speaker | Piper medium | **0.2 s** for 3 s of speech, 12x realtime | comfortable |
| Transcriber | Moonshine base | around 2 s for a short utterance | acceptable |
| Interpreter | Qwen 3 4B Q4 | **18 s median, 24 s worst** | **unusable for speech** |

A person waiting on a machine tolerates perhaps two seconds. The interpreter is
an order of magnitude outside that, and the smaller model is half the latency
with a third of the accuracy, so it does not help.

What this means in practice:

- **The deterministic path is unaffected.** Observing a cycle, diagnosing it,
  planning a correction and enforcing the envelope involve no model. That runs
  in milliseconds and is what a machine does unattended.
- **Explaining is unaffected.** The explainer needs no model, and the speaker
  is fast.
- **Spoken requests are not yet viable** on a processor of this class, from a
  model running on it. An accelerator, a smaller task-specific model, or a
  different approach to the interpreter seam is required, and none of those has
  been measured. A hosted model is the fourth option and is available today;
  see [Using a hosted model](#using-a-hosted-model).

Measure on the board you intend to ship before committing to a voice
interface:

```sh
adi models bench
```

This runs the corpus in `eval/espresso/` against the configured interpreter on
the host it is invoked on, and reports how often the answer was correct and
what each request cost.

### Using a different model

Three levels, in increasing order of effort.

**Change the configuration.** Point `models/models.yaml` at a different file.
A model absent from the manifest loads normally and is recorded as unverified,
identified by its checksum. Verification exists so that a decision can be
attributed to a specific model afterwards, as ADI-P 9.3 requirement 5 requires,
not to restrict what may be loaded.

**Use another program.** The `process` backend runs an external program and
exchanges one JSON request and one JSON answer per line over its standard
input and output. It serves the interpreter, chooser, transcriber and speaker
seams.
The program requires no dependency on this SDK and may be written in any
language.

```
interpreter  -> {"utterance":"half of last time","styles":[...],
                 "variables":[...],"notes":[...],"machine":"..."}
             <- {"intent":{"kind":"produce","targets":{...}}}
             or {"clarify":{"question":"...","because":"..."}}
             or {"answer":{"text":"...","drew_on":["<note id>"]}}

chooser      -> {"goals":[{"variable":"shot_time","delta":-3.0,...}],
                 "candidates":[{"index":0,"tool":"...","variable":"shot_time",
                                "per_unit":-1.4,"current":24,...}],
                 "level":"d2","constraints":[...]}
             <- {"steps":[{"candidate":0,"value":21}]}

transcriber  -> {"audio":"<base64>","sample_rate":16000}
             <- {"text":"...","confidence":0.91}

speaker      -> {"text":"..."}
             <- {"audio":"<base64>","sample_rate":22050}
```

Audio is base64 of 16-bit signed, little endian, single channel samples.

An interpreter has three outcomes rather than two. A request becomes an intent,
an unresolvable one becomes a question back, and a question from the person
becomes an answer, which actuates nothing. `machine` carries the device identity
and `notes` every note the knowledge base holds, so a backend needs no access to
either.

Notes are sent whole, on every utterance, with no relevance filter: selecting by
relevance guesses what a question needs and a wrong guess withholds the answer
silently. **Keeping the total within the context of the smallest model you
target is your responsibility.** The engine applies its budget, discards notes
before anything a request depends on, and prints a warning when it has to, so
exceeding the capacity is visible rather than silent.

Each candidate carries what its tool changes besides the outcome it was offered
for, as `also_affects`. That is what separates two tools serving one outcome:
changing the dose moves the strength as well, changing the grind does not, and
the magnitudes alone do not say so. It also carries a `description`, which is the
manufacturer's where they wrote one and the profile's for a registry tool.

A chooser answers with an index into the candidate list the engine supplied, so
it cannot name a tool the machine does not declare. `constraints` states the
rules the answer is checked against, including that no two steps may move the
same variable, so a backend author does not have to reproduce them. Where an
answer is refused for a reason the model could fix, the engine asks once more
with a `rejected` field explaining why. At D2 a value is the absolute setting to
send, not a change, which is why each candidate carries its `current` value.

Two sets of backends are supplied.

`examples/interpreters/reference.py` and
`examples/interpreters/reference_speech.py` implement the
protocol and contain no models. They are the specification by example, and are
used by the test suite.

`examples/interpreters/qwen_interpreter.py` and
`examples/interpreters/speech_backends.py` run real models, and are configured
by `models/models.example-local.yaml`. Both have been **run against real
weights** and corrected where they were wrong. See the table above for what
each was measured to do, and `models/manifest.yaml` for the checksums of the
files used.

`models/models.example-process.yaml` configures the reference set.

**Implement the trait.** `adi-language` defines the four seams as traits. A
manufacturer shipping at volume implements them directly against their own
runtime.

### Using a hosted model

ADI is designed to run fully locally. No account, no key, no network, and the
figures above were measured without an accelerator.

Where a network is available, a hosted model can serve any of the four seams.
The reason to use one is the interpreter latency above. A seam is a program that
reads one JSON request per line and writes one answer, so a script calling an
API is a complete implementation: no SDK dependency, any language, no change to
this engine.

Three things to get right.

**Put a timeout in your script.** The engine blocks waiting for a backend's
reply and imposes no limit of its own. A local model is slow and then finishes;
a stalled connection may never answer. While the engine waits it stops sending
heartbeats, so after `control_timeout_ms` the appliance decides the link is dead
and enters its safe state. That behaviour is correct and the cause will not be
obvious. Use a few seconds, and return a `clarify` when it expires.

**Constrain the output to the schema, narrowed to the machine.** Instruction is
advice; a grammar is not. Hosted models expose this as structured output or a
JSON schema, and it should be built from the declaration each request carries.
See `schema_for` in `examples/interpreters/qwen_interpreter.py`.

**Decide what leaves the building.** On the speech seams that is audio of
whoever is in the room.

D0 uses no model at all, so none of this touches the deterministic path. A
machine with its network removed still corrects itself. It loses speech, not
intelligence.

### Wiring a microphone and a speaker

The SDK handles sound once it is samples and once it has produced samples. Moving
those to and from a device is yours, because an output device, a microphone and
the library that drives them belong to a product and a platform rather than to a
protocol.

Two methods are the whole interface.

```rust
use adi_language::{Audio, Language};

// Build it once, from whichever backends your configuration names.
let language = Language::new(transcriber, interpreter, explainer, speaker);

// Someone spoke. Your capture code produced i16 mono samples.
let audio = Audio::new(samples, 16_000);
let understood = language.understand(&audio, &context)?;

// A decision was made. Hand the samples to your output device.
let (text, spoken) = language.report(&record)?;
```

`understand` transcribes, checks the result, and interprets in one step. A
transcript the backend is not confident in comes back as a question rather than
an intent, which matters more here than anywhere else: mishearing "hotter" as
"shorter" is a physical action on a machine with a heating element, and a
transcript is the one place where a wrong word is indistinguishable downstream
from a right one. The threshold is `min_transcript_confidence`, which defaults to
0.6 rather than something near certainty, because speech recognition over a
grinder is not a quiet-room benchmark and the cost of asking again is a second of
someone's time against a wasted drink.

What you supply:

**Samples in the rate the transcriber wants.** `transcriber.required_sample_rate()`
says which, and `understand` refuses a mismatch rather than resampling, since
resampling badly is worse than refusing. Signed 16 bit, mono.

**A decision about when to listen.** A button, a wake word, voice activity
detection. The SDK has no opinion and should not: a button on a commercial
machine and a wake word on a domestic one are different products, and neither
belongs in a protocol.

**Playback.** `report` returns the words alongside the samples, so a build with
no synthesis configured still has something to display.

What the SDK will not do for you is capture, playback, resampling, echo
cancellation or noise suppression. Those are the parts that need the device in
front of you, and no worked example against a real audio library is supplied yet.

### When something is refused

A refusal is an ordinary outcome, not a crash. The engine refuses when no
declared tool can move what was asked for, when a plan would breach the safety
envelope, when a plan would change one variable two ways, and when the appliance
declines a call.

`adi run` states which of those happened and continues, so the next request can
be made. Only a lost link ends a session, because nothing further can be
attempted.

```
> raise the extraction
  understood   Adjust
  target       extraction_yield = 22 Pct
  cannot       nothing this machine declares can move that.
               `adi capabilities` lists what it can change.
> status
  link         alive
```

The engine's own errors are precise and intended for a log. Translating them
into something a person can act on is the integrator's job, and `adi run` shows
one way of doing it.

### When a correction cannot be carried out in full

A correction held at the end of a declared range is partial. The engine records
what it wanted alongside what it sent, and `adi run` says so:

```
  sent         setting = 60 Step
  at limit     setting could not reach 999; this correction is partial
```

Without that, the record says the machine acted and nothing distinguishes a small
correction from a mechanism that cannot move. The person is told their request
was carried out when it was not.

Snapping to resolution is not this. A parameter declared in whole steps receiving
21.4 is set to 21, which is ordinary; only a value outside the range is reported.

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

The appliance library occupies **about 7.3 KB of flash on a Cortex-M4** with no
static or heap allocation, against the 16 KB budget stated in
[`c/DESIGN.md`](./c/DESIGN.md). A product adds its own generated tables.

The full appliance test suite, 332 checks, passes on an emulated Cortex-M3:

```sh
cd c
make arm-size     # footprint for Cortex-M4
make arm-test     # every suite, on an emulated Cortex-M3
```

Both need `arm-none-eabi-gcc` and `qemu-system-arm`, and neither needs a
board.

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
- **One transport binding, plus a serial port.** `Transport` is implemented
  for an appliance started as a child process over standard input and output,
  and for a serial port. BLE and USB are a product's own to write, which is two
  methods against a trait; `sdk/crates/adi-cli/src/serial.rs` is the worked
  example.
- **A decision record does not hold what a model was shown.** Every value that
  fed a decision is now recorded with the layer behind it, which is ADI-P 9.3
  requirement 5. Requirement 4 asks for more at D1 and D2: the record must also
  hold what the model was given and what it returned, and it does not. Until it
  does, a decision made with a model is reconstructable in its arithmetic and
  not in its selection.
- **No learning.** The machine remembers recent cycles and acts on them, so
  "half the sugar of last time" resolves correctly. What is missing is
  accumulation: `Layer::User` is defined, resolves correctly, and nothing writes
  to it, so twenty sweeter drinks do not change where the next one starts.
- **No port to physical hardware.** `libadip` compiles for Cortex-M and its
  full test suite passes on an emulated Cortex-M3, so the arithmetic and the
  logic are known to survive a 32-bit core. What emulation cannot establish is
  real timing, which matters for the reply deadline and the failsafe interval,
  and the port functions against real peripherals. The remaining milestone in
  [`c/DESIGN.md`](./c/DESIGN.md) is a reference port to a board.
- **The decoder has not been fuzzed**, and no MISRA review has been performed.

`serde_yaml` 0.9 is unmaintained upstream. No part of the design depends on
which YAML implementation is used.
