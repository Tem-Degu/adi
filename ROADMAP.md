# Roadmap

**StreetAI · Artificial Domain Intelligence**

This is the authoritative build plan. It supersedes BUILD-GUIDE sections 2 and 5,
which were written before the SDK and protocol strategy existed.

---

## What we are building, in one paragraph

A domain-neutral SDK that turns a declared set of physical actions into an
intelligent machine, plus espresso as the first domain implemented on top of it.
The SDK is the product. Espresso is the proof and the first revenue. Every domain
after espresso reuses the SDK unchanged and supplies only a profile.

---

## The architecture in three layers

| Layer | What it is | Domain-neutral | Owner |
|---|---|---|---|
| **Safe Action Protocol** | The safety architecture: an AI may decide freely, but may act only through declared, bounded, individually safe actions, under a stateful envelope enforced by the device. | Yes | StreetAI |
| **ADI-P** | The wire contract: capability declaration, skill, knowledge extensions, execution model. Specified in `PROTOCOL.md`. | Yes, except the registry | StreetAI |
| **Domain profile** | Registry, knowledge base and diagnosis rules for one domain. | No, by definition | StreetAI, extended by manufacturers |

**The rule that keeps this honest:** the word "shot" must never appear in
`adi-core`, `adi-protocol`, `adi-safety` or `adi-decision`. Not in a type, a
field, a variable or a comment. If it does, the abstraction has leaked.

---

## Autonomy level

We ship **D1**, with **D2** available as a configuration switch.

| Level | Tool chosen by | Value chosen by | Status |
|---|---|---|---|
| D0 | Engine | Engine | Supported, used as the value path inside D1 |
| **D1** | **Model, from declared tools** | **Engine** | **Default** |
| D2 | Model | Model | Switchable, gated on the stateful envelope |

D1 means the model reads what a machine can actually do and decides which actions
serve the request, including on machines whose tools we have never seen. The
engine computes every number from the skill and the knowledge base. D2 hands the
numbers to the model as well, and MUST NOT be enabled on a device whose
declaration lacks stateful invariants and rate limits.

---

## Phases

Phases 1 to 4 need no hardware at all.

### P1. Protocol types and conformance harness
Rust workspace, protocol structs with serialisation, validators for declaration,
skill and knowledge extension. A conformance runner that reads test cases from
data files rather than code.

*Gate:* every example in `PROTOCOL.md` parses, validates, and round-trips.

### P2. The safety layer
Envelope enforcement, and the stateful additions: invariants, rate and duty
limits, resource accounting. This is Safe Action Protocol in code, and it is the
prerequisite for D2 ever being switched on.

*Gate:* a sequence of individually valid actions that would overflow a cup is
rejected, and the rejection names the invariant.

### P3. The reasoning core and the espresso profile
The domain-neutral loop: observe a cycle, resolve targets through the knowledge
layers, diagnose, plan a correction from the skill's declared responses, clamp,
record. Then espresso as the first profile implementing the `Domain` trait.

*Gate:* textbook cycles diagnosed correctly, offline, no hardware. Decision
records reconstruct every value back to the layer that produced it.

### P4. The decision layer
D0, D1 and D2 selectors behind one trait. D1 wired to a model for tool selection
with a closed output schema. Level is configuration, not a code path.

*Gate:* the same request resolves correctly at D0 and D1 on a machine whose
sweetener tool is outside the registry. D2 refuses to run without stateful
invariants present.

### P5. Perception
BLE scale, cycle recorder, flow-curve smoothing. Python for exploration, Rust for
what ships.

*Gate:* a live cycle captured accurately: yield, time, flow curve.

### P6. The coach, end to end
Cycle in, diagnosis out, advice delivered, result stored, preference learned.
Text interface. This is the Tier 1 product and the investor demo.

*Gate:* two-shot dial-in works on a real machine.

### P7. Data and calibration
One hundred logged cycles with refractometer readings. Calibrate step sizes
against a real grinder. Unglamorous, most likely to be skipped, most expensive to
skip.

*Gate:* declared response values match measured behaviour within stated
confidence.

### P8. Channeling detection
The headline capability and the least certain item in this plan. Label data by
filming through a bottomless portafilter.

*Gate:* distinguishes channeling from a genuine grind error on held-out cycles.

### P9. Closed loop, cheaply
Motorise the grind adjustment. Around one hundred dollars, and it turns the coach
into a machine that corrects itself.

*Gate:* "that was sour" produces an automatic regrind and a second cycle.

### P10. Language, local
Rust linking whisper.cpp, llama.cpp and piper. One binary, no network.

*Gate:* the P6 demo runs with no connection of any kind.

### P11. The module
Off-the-shelf system-on-module, our firmware, our brand. Only when a manufacturer
conversation requires it, ideally funded by them.

### P12. The manufacturer package
C reference SDK, declaration validator, conformance test suite. These three turn
`PROTOCOL.md` from a document into something a firmware team can schedule.

Status:

- **C reference SDK**: `sdk/c`, milestones M1 to M5 complete. Types, generated
  tables, framing, decode, validation, dispatch, the stateful envelope and the
  failsafe. 232 host checks. M6, a port to real hardware, needs a board.
- **Validator**: `adi validate`, plus `adi capabilities` to show what the
  engine can actually move on a given machine.
- **Conformance suite**: `adi conformance`. Runs a corpus of input and expected
  decision pairs, reports each case, and fails the process on any regression.
  Wired into `cargo test` so it cannot rot.
- **Code generator**: `adi generate-c`, which was not in the original plan.
  It compiles a declaration into C tables so a device never parses one, and it
  removes the drift risk of a range written in both the declaration and the
  firmware.

---

## The second-profile discipline

You cannot design a good abstraction from one example. Alongside P3, maintain a
second domain profile as **data only**, no product and no hardware. A greenhouse
or reef tank is ideal, because the feedback loop is slow and the sensing is
different, which is exactly where espresso assumptions break.

Every time something is added to the neutral crates, check it against that second
profile. If it does not fit, it was espresso logic wearing a general name.

Cost: about a day, once. It is the difference between a platform and a coffee app
with ambitions.

---

## Equipment

**Buy now,** roughly $200 to $400 if a machine is already owned: Bluetooth scale,
grinder with fine adjustment, bottomless portafilter, WDT tool and tamper,
refractometer.

**Later, only if required:** a superautomatic if chasing full autonomy, a Decent
DE1 only if a manufacturer specifically asks for profile control.

---

## Honest risk read

| Phases | Confidence | Notes |
|---|---|---|
| P1 to P4 | High | Pure software, fully testable, no physical dependency |
| P5, P6 | ~85% | Scale protocol is the main unknown |
| P7, P8 | 50 to 65% | Noisy physical process, few samples, no ground truth without the refractometer |
| P9, P10 | Moderate | Mechanical fitting and embedded model work |
| P11, P12 | Needs a hardware contractor and manufacturer access |

The skills this plan requires span software, embedded, electronics, coffee
science and business-to-business sales. P1 through P10 are carryable by one
capable developer. P11 needs a contractor. P12 needs rooms with manufacturers.
