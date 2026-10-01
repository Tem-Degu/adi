# Roadmap

The build plan for Artificial Domain Intelligence, in phases with the test
that completes each one.

Phases marked DONE are implemented and covered by the test suites. The rest
are stated so that the shape of the work, and what is not yet known, is
visible rather than implied.

---

## What is being built

A domain-neutral SDK that turns a declared set of physical actions into an
intelligent machine, with espresso as the first domain built on it. Every
domain after espresso reuses the SDK unchanged and supplies only a profile.

---

## The architecture in three layers

| Layer | What it is | Domain-neutral |
|---|---|---|
| **Safe Action Protocol** | The safety architecture: an intelligence may decide freely, but may act only through declared, bounded, individually safe actions, under a stateful envelope the device enforces. | Yes |
| **ADI-P** | The integration contract: capability declaration, skill, knowledge extensions, execution model, wire format. Specified in `PROTOCOL.md`. | Yes, except the registry |
| **Domain profile** | Registry, knowledge base and diagnosis rules for one domain. Extended by manufacturers. | No, by definition |

**The rule that enforces this:** the word "shot" must never appear in
`adi-core`, `adi-protocol`, `adi-safety` or `adi-decision`. Not in a type, a
field, a variable or a comment. If it does, the abstraction has leaked.

---

## Autonomy levels

The default is **D2**. This reverses an earlier decision to default to D1.

| Level | Tool chosen by | Value chosen by | Status |
|---|---|---|---|
| D0 | Engine | Engine | Runs with no model. The level the conformance corpus uses |
| D1 | Model, from declared tools | Engine | Runs with a chooser |
| **D2** | **Model** | **Model** | **Default.** Runs with a chooser, gated on the stateful envelope |

A model-driven level needs a *chooser*, which is the component that asks a
model which declared action serves a goal. `ProcessChooser` supplies one over
an external program. Without a chooser, requesting D1 or D2 is refused rather
than quietly served at D0.

The model turns speech into structured intent at every level including D0, and
explains afterwards at every level. What the level grades is what happens after
intent.

D2 is the default because the alternative confines the machine to the
situations its developers enumerated in advance, and the useful requests are
frequently the ones outside that set. The safety of a
model-chosen magnitude does not rest on trusting the model: the engine clamps it
to the declared range and snaps it to the declared resolution, and the appliance
validates it again independently. A plan failing any check is refused whole.

D2 refuses to initialise against a declaration carrying no invariants and no
rate limits, because that is exactly the case where individually valid actions
compose into an invalid sequence.

---

## Phases

Phases 1 to 4 need no hardware at all.

**P1 to P4 are complete**, along with the wire format, the session loop and
the appliance SDK. See `sdk/README.md` for what is not yet done.

### P1. Protocol types and conformance harness  DONE
Rust workspace, protocol structs with serialisation, validators for declaration,
skill and knowledge extension. A conformance runner that reads test cases from
data files rather than code.

*Gate:* every example in `PROTOCOL.md` parses, validates, and round-trips.

### P2. The safety layer DONE
Envelope enforcement, and the stateful additions: invariants, rate and duty
limits, resource accounting. This is Safe Action Protocol in code, and it is the
prerequisite for D2 ever being switched on.

*Gate:* a sequence of individually valid actions that would overflow a cup is
rejected, and the rejection names the invariant.

### P3. The reasoning core and the espresso profile DONE
The domain-neutral loop: observe a cycle, resolve targets through the knowledge
layers, diagnose, plan a correction from the skill's declared responses, clamp,
record. Then espresso as the first profile implementing the `Domain` trait.

*Gate:* textbook cycles diagnosed correctly, offline, with no hardware.

The decision record carries the structure for tracing every value back to the
layer that produced it, and the session does not yet populate it. Filling it
belongs with P7, where values stop being estimates and provenance starts to
matter.

### P4. The decision layer DONE
D0, D1 and D2 selectors behind one trait. D0 is deterministic; D1 and D2 are
reached through a chooser, the component that asks a model which declared
action serves a goal, with a closed output schema. The level is configuration
rather than a code path.

*Gate:* the same request resolves correctly at D0 and D1 on a machine whose
sweetener tool is outside the registry. D2 refuses to run without stateful
invariants present.

### P5. Perception
BLE scale, cycle recorder, flow-curve smoothing. Python for exploration, Rust for
what ships.

*Gate:* a live cycle captured accurately: yield, time, flow curve.

### P6. The coach, end to end
Cycle in, diagnosis out, advice delivered, result stored, preference learned.
Text interface. This is the first complete product tier.

*Gate:* two-shot dial-in works on a real machine.

### P7. Data and calibration
One hundred logged cycles with refractometer readings, and step sizes
calibrated against a real grinder. The declared response values in the skill
are currently estimates, and this is what replaces them with measurements.

*Gate:* declared response values match measured behaviour within stated
confidence.

### P8. Channeling detection
The least certain item in this plan. Training data is labelled by filming
through a bottomless portafilter.

*Gate:* distinguishes channeling from a genuine grind error on held-out cycles.

### P9. Closed loop, cheaply
Motorise the grind adjustment, which is what turns a machine that advises
into one that corrects itself.

*Gate:* "that was sour" produces an automatic regrind and a second cycle.

### P10. Language, local
Speech recognition, understanding and synthesis running on the device with no
network.

This is partly done. Models run as separate programs that the engine starts
and exchanges one message per line with, and backends are supplied, so a
developer obtains model files and writes no code. What remains is loading
models inside the engine itself, which replaces two processes and a Python
runtime with one binary. That matters for a cheaper module rather than for
capability, so it is sequenced with P11.

*Gate:* the P6 demonstration runs with no connection of any kind.

### P11. The module
An off-the-shelf system-on-module carrying the engine, so a manufacturer
integrates one component rather than an application processor and a software
stack.

### P12. The manufacturer package
C reference SDK, declaration validator, conformance test suite. These three turn
`PROTOCOL.md` from a document into something a firmware team can schedule.

Status:

- **C reference SDK**: `sdk/c`. Types, generated tables, framing, decoding,
  validation, dispatch, the stateful envelope, the failsafe, and the outbound
  half: signals, events and the heartbeat. Tested on a host. A port to real
  hardware needs a board and is the one milestone outstanding.
- **Validator**: `adi validate`, plus `adi capabilities` to show what the
  engine can actually move on a given machine.
- **Conformance suite**: `adi conformance`. Runs a corpus of input and expected
  decision pairs, reports each case, and fails the process on any regression.
  Wired into `cargo test` so it cannot rot.
- **The wire**: specified in `PROTOCOL.md` section 10.2 and implemented on
  both sides, so the two exchange bytes rather than agreeing on paper. The
  test suite drives a real `libadip` build as a child process on every run.
- **The session**: `adi-session` holds the link, sends the heartbeat and runs a
  cycle from observation to invocation, enforcing the envelope across a whole
  plan before any step is sent.
- **Code generator**: `adi generate-c`, which was not in the original plan.
  It compiles a declaration into C tables so a device never parses one, and it
  removes the drift risk of a range written in both the declaration and the
  firmware.

---

## Keeping the abstraction general

An abstraction derived from one example fits that example. Building "for any
appliance" from espresso alone produces something that looks general and fits
only coffee.

The correction is a second domain profile, maintained as data only, with no
product and no hardware behind it. A greenhouse suits this because its
assumptions differ from espresso in the places that matter: cycles measured in
weeks rather than seconds, feedback arriving long after the action, and
operations that overlap rather than running one at a time.

Anything added to the neutral crates is checked against it. What does not fit
was domain logic carrying a general name.

`sdk/conformance/greenhouse/` holds that second profile. It has no product
and no hardware, and exists to keep the neutral layer general.

---

## Hardware required for development

Work on the espresso profile beyond P4 requires physical equipment, because the
perception rules cannot be validated against synthetic traces.

Required from P5 onward: a Bluetooth scale reporting mass at 10 Hz or better, a
grinder with fine adjustment, a bottomless portafilter, distribution and
tamping tools, and a refractometer for extraction yield.

Required only for specific work: a superautomatic machine for full closed-loop
autonomy, and a profiling machine only where a manufacturer requires pressure
or flow profile control.

---

## Confidence by phase

| Phases | Confidence | Notes |
|---|---|---|
| P1 to P4 | High | Pure software, fully testable, no physical dependency |
| P5, P6 | ~85% | Scale protocol is the main unknown |
| P7, P8 | 50 to 65% | Noisy physical process, few samples, no ground truth without the refractometer |
| P9, P10 | Moderate | Mechanical fitting and embedded model work |
| P11, P12 | Depends on hardware and manufacturer integration rather than on software |

The uncertainty in P7 and P8 is not a software risk. Extraction is a noisy
physical process, sample counts are small, and there is no ground truth for
extraction yield without a refractometer. Phases P1 to P4 carry no physical
dependency and are verified by the test suites and the conformance corpus.
