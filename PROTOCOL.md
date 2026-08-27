# ADI-P: Artificial Domain Intelligence Protocol

**Version 0.2 (draft) · Domain profile: espresso v1 · StreetAI**

The integration contract between a StreetAI Artificial Domain Intelligence (ADI)
engine and a manufacturer's appliance.

A conforming appliance describes three things: what it can do, how it behaves,
and anything it knows that the base domain knowledge does not. The engine reads
those descriptions and reasons over them. Nothing about the engine is rebuilt per
machine.

---

## 0. Status and scope

This document specifies:

1. **Safe Action Protocol**, the safety architecture the rest of the document
   implements. Section 1.1.
2. **Levels**: what a machine can do (conformance), and who decides what it does
   (autonomy). Section 3.
3. The **capability declaration**, a machine-readable description of an
   appliance's tools, signals and limits. Section 4.
4. The **skill**, a description of how those tools behave on this particular
   machine. Section 5.
5. **Knowledge extensions**, how a manufacturer adds to the domain knowledge base
   without altering it. Section 6.
6. The **semantic registry** for espresso, so a standard tool or variable means
   the same thing everywhere. Section 7.
7. **Vendor tools and procedures**, for functionality the registry does not
   cover. Section 8.
8. The **execution model** and the **safety envelope**. Sections 4.6 and 9.

It does not specify the engine's internal reasoning, the base domain knowledge
base, or the physical form of the compute module. Those are implementation.

### 0.1 Requirement keywords

MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as defined in RFC 2119.

### 0.2 Changes from 0.1

- **Safe Action Protocol** is named and stated as a principle in section 1.1.
  The mechanism was present in 0.1 but unnamed and scattered.
- **Autonomy levels D0 to D2** are specified in section 3.3. They existed in
  implementations but not in this document, and section 3 now distinguishes them
  from conformance levels, which are a separate axis.
- The **skill** is promoted to a first-class concept. In 0.1 this existed only as
  a deferred "knowledge pack".
- **Knowledge extensions** are introduced, with explicit layering and precedence.
- **Procedures** are demoted from a headline feature to a narrow escape hatch,
  and constrained by the binding rule in 8.3.2.

---

## 1. Model

```
   user speech or text
          |
   [ language layer ]        turns words into structured intent.
          |                  never produces an actuation value.
       intent
          |
   [ reasoning engine ]      intent
          |                    + signals        (what is happening)
          |                    + capability     (what this machine can do)
          |                    + skill          (how this machine behaves)
          |                    + knowledge      (base + vendor + user)
          |                  -> a concrete tool invocation
    tool invocation
          |
   [ constraint layer ]      clamps to declared ranges and envelope
          |
======= ADI-P ==============================================
          |
   [ appliance ]             validates independently, executes,
                             enforces its own envelope, reports
```

Five properties define the contract.

**The appliance is the final authority.** It MUST validate every invocation
against its own envelope and reject anything outside it, even when the engine has
already clamped. Enforcement is duplicated on purpose.

**The engine is stateless with respect to hardware.** Everything it knows about a
machine comes from the declaration, the skill and any knowledge extension. No
machine-specific code paths exist in the engine.

**Actuation values are deterministic.** See section 9.3. This is normative, not a
design preference, because the certification argument for the whole system rests
on it.

**Capability is discovered, not assumed.** An engine given a machine that cannot
set temperature solves the problem with the tools that exist, or degrades to
advising the operator.

**The protocol never limits the appliance.** This is a guarantee. A manufacturer
MAY declare any functionality it can build, including functionality this
specification has never anticipated, and MAY teach the engine how that
functionality behaves. Nothing a manufacturer invents is blocked, degraded or
made second class by integrating with an ADI engine.

### 1.1 Safe Action Protocol

The safety architecture of ADI-P has a name, because it is the part that
generalises beyond any one domain.

**Safe Action Protocol: an intelligence may decide freely, but may act only
through declared actions that are individually bounded and collectively
constrained.**

Safety is a property of the action surface rather than of the reasoning. This
matters because it does not depend on the model behaving well. A model that
reasons badly still cannot reach an action nobody built, cannot exceed a
declared range, and cannot breach a limit the appliance enforces in firmware.

Four requirements make the principle real. Each is specified in full elsewhere
in this document, and they are collected here because together they are the
argument.

1. **The action surface is closed.** An engine may invoke only what the
   declaration lists. There is no general-purpose escape from the declared set.
   See §4.4 and §8.
2. **Each action is bounded.** Every parameter carries a declared range, and the
   appliance rejects anything outside it rather than clamping. See §4.2 and §4.6.
3. **Sequences are bounded too.** Individually valid actions can compose into an
   invalid sequence, so invariants, rate limits and resource accounting
   constrain what actions add up to over time. See §4.6.
4. **Enforcement is independent.** The appliance enforces its own envelope in
   firmware, without relying on the engine being correct. See §4.6 requirement 3.

Point 3 is the one that is easy to miss. Three dispense actions of 150 ml each
are each within range, and their sum overflows a 320 ml container. No per-call
check can express that, which is why the envelope is stateful.

### 1.2 The division of knowledge

Three parties know three different things, and the protocol keeps them separate.

| Who | Knows | Expressed as |
|---|---|---|
| Manufacturer | What my machine can do | Capability declaration (§4) |
| Manufacturer | How my machine responds | Skill (§5) |
| Manufacturer | What is special about my product | Knowledge extension (§6) |
| StreetAI | What espresso is and what makes it good | Base knowledge base |
| Engine | What to do right now for this person | Runtime reasoning |

A manufacturer never writes reasoning. It describes hardware and behaviour. The
engine never contains machine-specific logic. It reads descriptions.

### 1.3 No engine release is required

Normative, because integration schedules depend on it.

1. Declarations, skills and knowledge extensions are **data read at runtime**,
   not engine code. An engine MUST NOT require a new build to accept any of them.
2. A manufacturer MAY add, change or remove tools, skill entries or knowledge
   entries at any time by publishing new documents. The engine MUST adopt them on
   the next session without modification, rebuild or redeployment.
3. A manufacturer MAY author its own skill and knowledge extensions. Authorship
   MUST NOT be reserved to the engine vendor.
4. No part of adding functionality to an appliance MAY depend on the engine
   vendor's release schedule.

A manufacturer's product timeline is therefore never gated on StreetAI.

---

## 2. Terminology

| Term | Meaning |
|---|---|
| **Appliance** | The physical device implementing this protocol. |
| **Engine** | The ADI reasoning engine invoking tools. |
| **Tool** | A named, parameterised action the appliance can perform. |
| **Signal** | A named, sampled measurement the appliance reports. |
| **Event** | An asynchronous, discrete notification. |
| **Declaration** | The document describing tools, signals, events and limits. |
| **Skill** | The document describing how this machine's tools affect outcomes. |
| **Domain variable** | A quantity the engine models, such as `shot_time`. |
| **Knowledge extension** | Manufacturer additions to the domain knowledge base. |
| **Envelope** | Absolute limits the appliance enforces in hardware or firmware. |
| **Semantic ID** | A registry identifier fixing the meaning of a tool, signal or variable. |
| **Procedure** | A declared sequence of tool invocations. Escape hatch only. |
| **Session** | A bounded period of engine control over an appliance. |

---

## 3. Levels

Two independent level systems appear in this specification, and they are easy to
confuse because both count from zero. They answer different questions.

| System | Question it answers | Declared by | Where |
|---|---|---|---|
| **Conformance level 0 to 2** | What can this machine do? | The manufacturer, in the declaration | §3.1 |
| **Autonomy level D0 to D2** | Who decides what to do? | The integrator, as configuration | §3.3 |

They are orthogonal. A Conformance Level 2 machine can be operated at autonomy
D0, and frequently is, because a machine capable of running unattended is
exactly the kind that may require every decision to be reproducible.
Conformance Level 2 is named "Autonomous" because the *machine* needs no human
hands. It says nothing about who chooses the actions.

Autonomy levels are always written with the D prefix in this document, for
exactly this reason.

### 3.1 Conformance levels

A declaration MUST state exactly one level. Levels are cumulative.

### Level 0: Advisory

The appliance reports signals and exposes no tools with physical effect on the
process. The engine observes, diagnoses and advises the operator. This is the
retrofit and coach case, and the only level that works on an unmodified machine
through an external sensor.

Required: at least one signal.

### Level 1: Parameterised control

The appliance accepts process parameters and start and stop commands, while the
operator performs manual preparation.

Required: `adi.espresso.brew.start`, `adi.espresso.brew.stop`, and at least one
signal usable for closed-loop stop.

### Level 2: Autonomous

The appliance can complete the process without human contact.

Required: everything in Level 1, plus `adi.espresso.grind.set_setting` and
`adi.espresso.dose.by_mass`.

An appliance MAY declare Level 2 while omitting tamping if its brew unit makes
tamping implicit. It MUST then declare `adi.espresso.tamp` as `implicit: true`
rather than omitting the concept silently.

### 3.2 Skill requirement by conformance level

| Level | Skill |
|---|---|
| 0 | OPTIONAL. Improves diagnosis but is not required. |
| 1 | SHOULD be provided. Without it the engine uses conservative defaults. |
| 2 | MUST be provided, covering every actuation tool the engine may select. |

An engine MUST NOT autonomously select a tool for which it has neither a registry
default nor a skill entry. See section 5.6.

### 3.3 Autonomy levels

Autonomy level sets how much of each decision the language model makes. It is
configuration on the engine side, chosen per integration, and it is not part of
the declaration.

Two decisions are involved in any correction: **which action** to take, and
**what value** to give it. The levels differ only in who makes each.

| Level | Chooses the action | Chooses the value | Reproducible |
|---|---|---|---|
| **D0** | Engine | Engine | Yes, fully |
| **D1** | Model, from declared candidates | Engine | Values yes, selection no |
| **D2** | Model | Model, within declared bounds | No |

**D0.** The engine ranks the candidate tools by declared confidence and takes
the best. Every decision replays identically from the same inputs. This is the
level to use where certification requires reproducibility.

**D1.** The engine derives the candidate list from the declaration and the
skill, and the model chooses among it. This is what makes a vendor tool usable
without the engine having been taught what it means in advance. The model never
sees or produces a magnitude.

**D2.** The model chooses the value as well. More flexible on genuinely novel
requests, and correspondingly less predictable.

#### 3.3.1 Requirements at every level

These hold at D0, D1 and D2 without exception.

1. The candidate set MUST be derived from the declaration. A selector MUST NOT
   be able to name a tool the machine did not declare.
2. Every value MUST be clamped to the declared range before invocation, and the
   appliance MUST validate independently regardless.
3. The full envelope of §4.6 applies, including the stateful constraints.
4. Everything in §9.3 that does not concern selection still applies, including
   the requirement to record what produced each value.

An engine MUST record the autonomy level in the decision record, because a
D1 or D2 decision is not reproducible from inputs alone and an auditor has to
be able to tell which decisions those were.

#### 3.3.2 D2 requires a stateful envelope

An engine MUST refuse to operate at D2 against a declaration that carries no
invariants and no rate limits.

The reason is specific. A model choosing its own magnitudes is exactly the case
where individually valid actions compose into an invalid sequence, and the
stateful constraints in §4.6 are the only thing that catches it. Without them
the Safe Action Protocol argument in §1.1 does not hold at that level.

This refusal is a normative requirement rather than a recommendation.

---

## 4. The capability declaration

A conforming appliance MUST expose exactly one declaration document, retrievable
before any tool is invoked. YAML is used here for readability. JSON is equally
valid and is the recommended wire format.

### 4.1 Top-level structure

```yaml
adip_version: "0.2"
domain: "espresso"
domain_profile: "espresso/v1"
conformance_level: 2

device:
  vendor: "Example Appliances"
  model: "EX-900"
  hardware_revision: "C"
  firmware_version: "3.2.1"
  serial: "EX900-0000A1B2"
  declaration_id: "sha256:6f1c..."

session:
  max_concurrent: 1
  heartbeat_interval_ms: 500
  control_timeout_ms: 2000

references:
  skill: "skill/EX-900/2.1"                 # section 5
  knowledge_extension: "x.example/kb/1.4"   # section 6, optional

signals:    [ ... ]    # section 4.3
tools:      [ ... ]    # section 4.4
events:     [ ... ]    # section 4.5
envelope:   { ... }    # section 4.6
procedures: [ ... ]    # section 8.3, optional
```

`declaration_id` MUST change whenever any part of the declaration changes. The
engine SHOULD cache declarations keyed on this value.

### 4.2 Parameter types

Every numeric parameter MUST declare a unit, a range and a resolution. Units MUST
be drawn from the table below. Unitless numbers are not permitted except for
`index` and `count` types.

| Unit token | Quantity |
|---|---|
| `g` | mass, grams |
| `g_per_s` | mass flow |
| `ml` | volume |
| `ml_per_s` | volume flow |
| `c` | temperature, degrees Celsius |
| `bar` | pressure |
| `s`, `ms` | time |
| `step` | a discrete mechanism position |
| `kg_f` | tamping force |
| `pct` | percentage, 0 to 100 |

Type forms:

```yaml
parameters:

  # continuous
  temperature:
    { type: number, unit: c, min: 85.0, max: 96.0, resolution: 0.1,
      default: 92.0 }

  # discrete mechanism position
  setting:
    { type: integer, unit: step, min: 0, max: 240, resolution: 1,
      monotonic: finer_is_lower }

  # enumerated
  style:
    { type: enum, values: ["espresso", "lungo", "ristretto"] }

  # structured
  profile:
    { type: object, schema: "adi.espresso.profile/v1" }
```

`monotonic` is required on any mechanism where direction has domain meaning. For
a grinder it tells the engine which way is finer, which it cannot otherwise know
and MUST NOT guess.

### 4.3 Signals

```yaml
signals:
  - id: "adi.espresso.signal.mass"
    name: "cup_mass"
    unit: g
    range: { min: 0, max: 2000 }
    sample_rate_hz: 20
    accuracy: { abs: 0.1, unit: g }
    latency_ms: 50
    available: ["idle", "brewing"]

  - id: "adi.espresso.signal.flow"
    name: "mass_flow"
    unit: g_per_s
    sample_rate_hz: 20
    derived_from: "adi.espresso.signal.mass"
    filtered: true

  - id: "adi.espresso.signal.group_temp"
    name: "group_temperature"
    unit: c
    range: { min: 0, max: 160 }
    sample_rate_hz: 4
    accuracy: { abs: 0.5, unit: c }
```

`accuracy` is mandatory. The engine uses it to decide whether an observed
difference is real or noise, and an undeclared accuracy forces conservative
behaviour that will make the appliance look worse than it is.

`derived_from` MUST be set on any signal computed rather than measured, and
`filtered` MUST state whether smoothing has already been applied, so the engine
does not filter twice.

### 4.4 Tools

```yaml
tools:
  - id: "adi.espresso.grind.set_setting"
    name: "set_grind"
    parameters:
      setting:
        type: integer
        unit: step
        min: 0
        max: 240
        resolution: 1
        monotonic: finer_is_lower
    preconditions: ["state == idle"]
    effects: ["grind_setting"]
    duration_ms_typ: 1800
    duration_ms_max: 6000
    reversible: true
    authority: ["engine", "operator"]

  - id: "adi.espresso.brew.start"
    name: "brew"
    parameters:
      stop_at_mass: { type: number, unit: g, min: 10, max: 120, resolution: 0.1 }
      stop_at_time: { type: number, unit: s, min: 5, max: 90, optional: true }
    preconditions: ["state == ready", "portafilter_present == true"]
    effects: ["cup_mass", "puck_state"]
    duration_ms_typ: 30000
    duration_ms_max: 90000
    reversible: false
    abortable: true
    authority: ["engine", "operator"]
    hazard: ["hot_liquid", "pressure"]
```

Field requirements:

- `id` MUST be a registry semantic ID (section 7) or a vendor ID (section 8).
- `preconditions` MUST list every state the appliance requires. The engine will
  not invoke a tool whose preconditions it cannot satisfy or verify.
- `effects` names what the tool changes, using domain variable names from section
  7.3 where applicable. The engine uses this to avoid changing two things at
  once, which is a hard rule in espresso dial-in.
- `reversible` MUST be `false` for anything that consumes material, produces heat
  or pressure, or cannot be undone by a further call.
- `hazard` informs confirmation policy and logging. It does not replace the
  envelope.
- `authority` states who may invoke. An appliance MAY reserve a tool to the
  operator only.

Physical behaviour such as settling time and retention is **not** declared here.
It belongs in the skill, section 5.

### 4.5 Events

```yaml
events:
  - id: "adi.espresso.event.first_drip"
    payload: { elapsed_s: { type: number, unit: s } }
  - id: "adi.espresso.event.fault"
    payload:
      code: { type: string }
      severity: { type: enum, values: ["warning", "error", "critical"] }
      recoverable: { type: boolean }
```

An appliance MUST emit a fault event before or simultaneously with entering a
safe state.

### 4.6 The safety envelope

The envelope is separate from parameter ranges and stricter in status. A
parameter range is the operating window the engine may request within. The
envelope is what the appliance enforces no matter what arrives.

```yaml
envelope:
  enforced_by: "firmware"          # "firmware" | "hardware" | "both"
  independent_of_engine: true      # MUST be true to claim conformance

  limits:
    - parameter: "brew_temperature"
      unit: c
      hard_min: 80.0
      hard_max: 98.0
    - parameter: "brew_pressure"
      unit: bar
      hard_max: 12.0
    - parameter: "brew_duration"
      unit: s
      hard_max: 120.0

  interlocks:
    - id: "portafilter_present"
      blocks: ["adi.espresso.brew.start"]
    - id: "drip_tray_present"
      blocks: ["adi.espresso.brew.start", "adi.espresso.puck.eject"]

  failsafe:
    on_control_timeout: "abort_to_safe"
    on_signal_loss: "abort_to_safe"
    on_fault: "abort_to_safe"
    safe_state: "no_heat_no_pressure_no_motion"
    max_time_to_safe_ms: 500
```

Normative requirements:

1. The appliance MUST enforce every declared limit independently of the engine.
2. The appliance MUST reject an out-of-range invocation with `REJECTED` and a
   reason code, and MUST NOT clamp silently. Silent clamping hides engine faults
   and destroys the audit trail.
3. `independent_of_engine` MUST be `true`. An appliance whose safety depends on
   the engine behaving correctly does not conform at any level.
4. On loss of the control link for longer than `session.control_timeout_ms`
   during a non-reversible operation, the appliance MUST enter `safe_state`
   within `max_time_to_safe_ms`.
5. Nothing in a skill or knowledge extension may relax the envelope. See 6.4.

---

## 5. The skill

The declaration says what a machine can do. The skill says what happens when it
does it.

This is the knowledge an experienced operator builds about one specific machine
in their first weeks: that this grinder's steps bite harder at the fine end, that
this boiler takes fifteen seconds to settle, that this brew unit retains under a
gram. The manufacturer already knows all of it. Written down once, it transfers
to every engine on every unit they ship.

A skill is **descriptive**. It states how the machine responds. It MUST NOT state
what the engine should do. That distinction is what keeps the reasoning portable
and the audit trail intact.

### 5.1 Structure

```yaml
skill_version: "1.0"
skill_id: "skill/EX-900/2.1"
applies_to:
  vendor: "Example Appliances"
  model: "EX-900"
  firmware_range: { min: "3.0.0", max: "3.9.99" }

responses:    [ ... ]   # 5.2
dynamics:     [ ... ]   # 5.3
interactions: [ ... ]   # 5.4
operating:    { ... }   # 5.5
notes:        [ ... ]   # 5.7
```

### 5.2 Responses

A response says how much a domain variable moves when a tool parameter moves.

```yaml
responses:
  - tool: "adi.espresso.grind.set_setting"
    parameter: "setting"
    variable: "shot_time"
    per_unit: -1.2            # variable change per +1 of parameter
    unit: s
    valid_range: { min: 12, max: 60 }
    confidence: 0.7

  - tool: "adi.espresso.brew.set_temperature"
    parameter: "temperature"
    variable: "shot_time"
    per_unit: -0.4
    unit: s
    valid_range: { min: 88.0, max: 94.0 }
    confidence: 0.5
```

Requirements:

1. `variable` MUST name a domain variable from the registry (section 7.3). An
   engine that does not model the named variable MUST ignore the response and
   MUST NOT guess a meaning.
2. `per_unit` is signed. Negative means increasing the parameter decreases the
   variable. Direction is expressed numerically rather than in words so it cannot
   be misread.
3. `valid_range` bounds where the stated linearity holds. Outside it the engine
   MUST fall back to conservative default step sizes.
4. `confidence` is 0 to 1 and states how firmly the manufacturer holds the
   figure. The engine uses it to size correction steps and to decide how much to
   trust its own observations over the declared value.

A response MUST NOT contain a target, a recommendation or an actuation value. It
describes the machine, not the coffee.

### 5.3 Dynamics

When a change does not take effect immediately.

```yaml
dynamics:
  - tool: "adi.espresso.grind.set_setting"
    retention:
      mass: { value: 0.8, unit: g }
      remedy_tool: "adi.espresso.grind.purge"
    hysteresis:
      steps: 2
      direction: "coarser_to_finer"

  - tool: "adi.espresso.brew.set_temperature"
    settling:
      time: { value: 15, unit: s }
      tolerance: { value: 0.3, unit: c }

  - scope: "device"
    warmup:
      time: { value: 900, unit: s }
      note_id: "cold_start_runs_fast"
```

Retention is the case that matters most in espresso. After a grind change some
previous-setting grounds remain, so the next result is a blend. An engine that
does not know this misreads the following shot and corrects in the wrong
direction. Declaring it lets the engine purge first or discount the result.

Hysteresis captures mechanisms that must be approached from one direction, which
is true of many grinder threads and is otherwise invisible to the engine.

### 5.4 Interactions

```yaml
interactions:
  - changing: "grind_setting"
    invalidates: ["last_shot_baseline"]
  - changing: "brew_temperature"
    invalidates: ["last_shot_baseline"]
  - changing: "dose_mass"
    also_affects: ["shot_time", "extraction_yield"]
```

`invalidates` tells the engine that a stored comparison point is no longer valid,
so it does not attribute a change to the wrong cause. `also_affects` warns that a
parameter moves more than one variable, which interacts with the
one-variable-at-a-time rule in 9.4.

### 5.5 Operating limits

Distinct from the safety envelope. These are limits on where the machine performs
*well*, not where it is *safe*.

```yaml
operating:
  reliable_ranges:
    - variable: "brew_temperature"
      min: 88.0
      max: 94.0
      unit: c
      note_id: "temp_above_94_not_held_back_to_back"
  recovery:
    between_shots: { value: 25, unit: s }
```

The engine treats a reliable range as a soft boundary. It may propose a value
outside it when nothing else will solve the problem, and SHOULD tell the operator
that it is doing so. It may never propose a value outside the envelope.

Declaring honest operating limits makes a machine perform better, not worse. An
engine that knows the ceiling stops chasing it and solves the problem another
way.

### 5.6 Absence of a skill

An engine encountering a tool with no skill entry MUST:

1. Use the registry default step size for that tool, if one exists.
2. Treat its confidence as low, and take smaller corrective steps.
3. Prefer a tool it does understand where one is available.
4. Never select the tool autonomously if no registry default exists. It remains
   callable by the operator and by stored recipes.

This is what makes a partial skill useful rather than useless. A manufacturer can
describe two tools well and leave the rest, and the engine degrades gracefully.

### 5.7 Notes

```yaml
notes:
  - id: "cold_start_runs_fast"
    text: "The first shot after warm-up typically runs two seconds fast."
  - id: "temp_above_94_not_held_back_to_back"
    text: "Above 94C the boiler cannot hold setpoint across consecutive shots."
```

Notes are **advisory and non-normative**. They MAY be surfaced to the operator by
the language layer as explanation. They MUST NOT be used as an input to any
actuation value. Anything intended to change engine behaviour MUST be expressed
as a response, a dynamic, an interaction or an operating limit, all of which are
structured and testable.

---

## 6. Knowledge extensions

A manufacturer may add to the domain knowledge base. It may not alter it.

### 6.1 Layers

Knowledge resolves through ordered layers. Each layer may add entries and refine
existing ones. No layer may delete or contradict a lower one except as permitted
in 6.4.

| Layer | Owner | Contents |
|---|---|---|
| 0 `base` | StreetAI | The domain knowledge base: targets, priors, step sizes, constraints |
| 1 `vendor` | Manufacturer | Additions specific to their product |
| 2 `deployment` | Integrator or region | Local water, regional preference, market defaults |
| 3 `user` | The engine, learned | Palate bias, saved recipes, per-bean history |

Resolution is highest layer first. Every effective value MUST be attributable to
the layer that produced it, because section 9.3 requires the engine to be able to
reconstruct any decision.

### 6.2 Namespacing

A vendor knowledge extension MUST place all new identifiers under its own
namespace.

```yaml
knowledge_extension:
  namespace: "x.example"
  extension_version: "1.4"
  extends: { domain_profile: "espresso/v1", base_min_version: "1.0" }

  drink_styles:
    - id: "x.example.style.flat_white"
      inherits: "adi.espresso.style.espresso"
      targets:
        ratio:       { min: 1.8, max: 2.2 }
        shot_time:   { min: 25, max: 32, unit: s }
        milk_volume: { min: 90, max: 130, unit: ml }

  bean_priors:
    - id: "x.example.bean.house_blend"
      roast: "medium_dark"
      starting_grind_offset: -3
      targets:
        brew_temperature: { min: 90.0, max: 92.0, unit: c }

  defaults:
    - variable: "dose_mass"
      value: 9.0
      unit: g
      rationale_note_id: "brew_unit_basket_size"
```

Requirements:

1. Every identifier a vendor introduces MUST begin with its namespace.
2. A vendor extension MUST NOT redefine an identifier it does not own. Attempting
   to redefine `adi.espresso.style.espresso` MUST be rejected at validation.
3. A vendor extension MAY declare `inherits` from a base entry. The new entry is
   a distinct identifier that starts from the base values and overrides only the
   fields it names. The base entry is unaffected for every other machine and for
   this one.
4. Removing a vendor extension MUST return the engine to exactly the base
   behaviour. Extensions are additive by construction.

### 6.3 What may be extended

| Extendable | Not extendable |
|---|---|
| New drink styles | Base drink style definitions |
| New bean priors | Base diagnosis rules |
| Machine-specific defaults and starting points | Hard constraints, in the permissive direction |
| New domain variables in the vendor namespace | Registry semantic IDs |
| Narrower target ranges for the vendor's own styles | The meaning of a base identifier |

Base **diagnosis rules** are deliberately not extendable in this version. Those
rules are the engine's reasoning, they are what StreetAI certifies, and a
manufacturer changing them would move the safety and correctness argument onto
the manufacturer's side of the boundary. A manufacturer that believes a rule is
wrong for their machine expresses that through the skill, which is the correct
mechanism: describe the machine differently, do not rewrite the reasoning.

### 6.4 Constraint monotonicity

Normative, and the rule that makes layering safe.

1. A higher layer MAY narrow a range. It MUST NOT widen one.
2. A higher layer MUST NOT relax any hard constraint from the base knowledge
   base, and MUST NOT affect the appliance's safety envelope at all. The envelope
   lives in firmware and is unreachable from knowledge.
3. The effective constraint for any variable is the intersection of every layer.
4. An extension violating 1 or 2 MUST be rejected in full at validation. Partial
   loading is not permitted.

The direction of travel is always more restrictive. A manufacturer can make its
machine more careful. It cannot make it less careful.

### 6.5 Validation

Before loading, the engine MUST verify:

1. `extends` names a domain profile the engine implements, at a compatible base
   version.
2. Every introduced identifier is correctly namespaced.
3. Every `inherits` target exists.
4. No redefinition of an unowned identifier.
5. Constraint monotonicity per 6.4.
6. No cycles in `inherits`.

A failing extension MUST be rejected with a reason and the engine MUST continue
on base knowledge alone. A bad extension degrades a machine to standard
behaviour. It never stops it working.

---

## 7. Semantic registry, espresso v1

A standard ID fixes meaning across manufacturers. An appliance declaring
`adi.espresso.grind.set_setting` is asserting the tool adjusts grind fineness and
nothing else.

### 7.1 Tools

| Semantic ID | Meaning | Level |
|---|---|---|
| `adi.espresso.grind.set_setting` | Set grind fineness | 2 |
| `adi.espresso.grind.purge` | Discard retained grounds | 2 |
| `adi.espresso.dose.by_mass` | Dispense ground coffee by mass | 2 |
| `adi.espresso.tamp` | Compress the puck | 2 |
| `adi.espresso.brew.set_temperature` | Set brew water temperature | 1 |
| `adi.espresso.brew.load_profile` | Install a pressure or flow profile | 1 |
| `adi.espresso.brew.start` | Begin extraction | 1 |
| `adi.espresso.brew.stop` | End extraction | 1 |
| `adi.espresso.brew.set_live_target` | Adjust target mid-extraction | 1 |
| `adi.espresso.puck.eject` | Discard the spent puck | 2 |
| `adi.espresso.additive.dispense` | Dispense milk, water or syrup | 1 |
| `adi.espresso.maintenance.flush` | Flush the group | 0 |

`adi.espresso.additive.dispense` MUST declare a `substance` enum parameter. The
engine treats additives as preference variables, never as extraction variables,
and will not attempt to correct a taste fault by changing them.

### 7.2 Signals

| Semantic ID | Quantity |
|---|---|
| `adi.espresso.signal.mass` | Mass in the cup |
| `adi.espresso.signal.flow` | Mass flow rate |
| `adi.espresso.signal.pressure` | Brew pressure |
| `adi.espresso.signal.group_temp` | Group temperature |
| `adi.espresso.signal.elapsed` | Time since extraction start |
| `adi.espresso.signal.grind_setting` | Current grind position |
| `adi.espresso.signal.dose_mass` | Mass of the dose |

### 7.3 Domain variables

The quantities the engine models. A skill response MUST name one of these, or a
vendor variable declared under 6.3.

| Variable | Unit | Meaning |
|---|---|---|
| `dose_mass` | g | Dry coffee in |
| `yield_mass` | g | Liquid out |
| `ratio` | none | `yield_mass / dose_mass` |
| `shot_time` | s | First drip to stop |
| `grind_setting` | step | Current grinder position |
| `brew_temperature` | c | Brew water temperature |
| `extraction_yield` | pct | Dissolved solids extracted, when measurable |
| `flow_peak` | g_per_s | Peak mass flow |
| `flow_variance` | none | Flow irregularity, the channeling indicator |

### 7.4 Minimum useful set

An appliance declaring `adi.espresso.signal.mass` and
`adi.espresso.signal.elapsed` at 10Hz or better gives the engine enough to
compute yield, ratio, shot time and a flow curve, which is enough for full
diagnosis at Level 0. Everything else improves confidence rather than enabling
the core function.

---

## 8. Vendor tools and procedures

### 8.1 Vendor tools

The registry describes what is known today. It is not a limit on what an
appliance may do, and a manufacturer is never required to fit a new idea into an
existing semantic ID. Invent the mechanism, declare it, ship it.

A manufacturer MAY declare tools outside the registry using its namespace:

```yaml
tools:
  - id: "x.example.steam.microfoam_program"
    name: "microfoam"
    parameters:
      texture: { type: enum, values: ["flat", "latte", "cappuccino"] }
    preconditions: ["state == idle", "milk_present == true"]
    effects: ["milk_texture"]
    reversible: false
    authority: ["engine", "operator"]
```

### 8.2 How a vendor tool becomes usable

**Callable, immediately.** A declared tool can be invoked when the operator names
it or a stored recipe references it, and it enters the language layer's
vocabulary. This requires nothing beyond a valid declaration.

**Reasoned over, once described.** For the engine to select a vendor tool on its
own in pursuit of a goal, the skill MUST contain a response for it, naming a
domain variable the engine models. That is the entire mechanism. There is no
separate approval and no engine change.

**Promoted, eventually.** A vendor tool adopted by several manufacturers may be
taken into the registry in a later profile version, at which point every machine
declaring it inherits full support.

An engine MUST NOT autonomously invoke a tool for which it has no skill response
and no registry default. Calling an unknown tool on a hot machine to see what
happens is precisely the behaviour this protocol exists to prevent.

### 8.3 Procedures

A **procedure** is a declared sequence of tool invocations. It exists for cases
where the hardware mechanically requires an order that cannot be expressed
through preconditions and effects alone.

Procedures are an escape hatch, not a primary mechanism. Most sequencing should
emerge from the engine reasoning over tools, skill and knowledge. A manufacturer
reaching for procedures to describe how a drink is made is usually describing
something that belongs in a knowledge extension instead.

#### 8.3.1 Declarative only

1. A procedure MUST be declarative data. It MUST NOT contain executable code,
   scripts or arbitrary expressions.
2. Every step MUST reference a tool declared in the same document.
3. The only control constructs are `await`, with mandatory `timeout_s` and
   `on_timeout`, and `on_error` with values `abort` or `continue`.

Because every step is a declared tool with declared ranges and preconditions, the
engine can verify an entire procedure before performing a single physical action.

#### 8.3.2 The binding rule

Normative, and the reason procedures are safe to allow.

Each argument declares where its value comes from: `fixed`, `parameter`,
`operator`, or `engine`.

**An argument corresponding to a domain variable in section 7.3 MUST be
engine-bound.** A manufacturer MAY fix values that are mechanical necessities of
its hardware. It MUST NOT fix a value that determines the outcome.

Without this rule a manufacturer could hardcode an entire recipe, and the
appliance would degrade into preset programs with a voice front end. The engine
MUST reject a procedure that fixes an outcome variable.

#### 8.3.3 Validation

Before the first step the engine MUST verify that every referenced tool exists,
every `fixed` argument is in range, every `engine` binding names a modelled
variable, preconditions are satisfiable given preceding effects, the binding rule
in 8.3.2 holds, worst-case duration fits `max_duration_s`, and no procedure calls
itself. Nesting depth MUST NOT exceed 4.

A procedure failing validation MUST be rejected with `PRECONDITION_FAILED` and
MUST NOT be partially executed.

#### 8.3.4 Abort

`abortable_until` names the last step before which an abort leaves no wasted
material or unsafe state. The engine MUST be able to abort at any step, the
appliance MUST honour it, and the failsafe in 4.6 applies throughout.

#### 8.3.5 Example

```yaml
procedures:
  - id: "x.example.sequence.purge_and_dose"
    name: "purge_then_dose"
    effects: ["grinder_retention", "dose_mass"]
    abortable_until: "dose"
    max_duration_s: 30
    steps:
      - id: "purge"
        tool: "adi.espresso.grind.purge"
        arguments:
          mass: { binding: fixed, value: 0.8 }
        on_error: "abort"

      - id: "dose"
        tool: "adi.espresso.dose.by_mass"
        arguments:
          mass: { binding: engine, domain_variable: "dose_mass" }
        await:
          signal: "adi.espresso.signal.dose_mass"
          condition: "stable"
          timeout_s: 20
          on_timeout: "abort"
```

The purge mass is fixed because it is a property of the burr chamber. The dose is
engine-bound because it determines the coffee. That is the line 8.3.2 draws.

---

## 9. Execution model

### 9.1 Session

1. Engine requests the declaration, then the referenced skill and knowledge
   extension if present.
2. Engine validates all three. It MUST reject on unknown `adip_version` or a
   malformed envelope, and MUST continue without a skill or extension that fails
   validation rather than refusing the session.
3. Engine opens a session. The appliance MAY refuse if another session holds
   control.
4. Engine maintains a heartbeat at `session.heartbeat_interval_ms`.
5. Engine closes the session, or the appliance times it out.

The operator MUST be able to take control at any moment by physical means. A
physical override MUST abort an engine-initiated operation and MUST emit an
event.

### 9.2 Invocation lifecycle

```
  invoke ──> ACCEPTED ──> RUNNING ──> COMPLETED
                │             │
                │             └────> ABORTED   (reason)
                └──────────────────> REJECTED  (reason)
```

The appliance MUST respond `ACCEPTED` or `REJECTED` within 200ms.

| Code | Meaning |
|---|---|
| `OUT_OF_RANGE` | A parameter fell outside the declared range |
| `ENVELOPE_VIOLATION` | The request breached a hard limit |
| `PRECONDITION_FAILED` | A declared precondition was not met |
| `INTERLOCK` | A safety interlock is open |
| `BUSY` | Another operation is in progress |
| `NOT_AUTHORISED` | The engine may not invoke this tool |
| `FAULT` | The appliance is in a fault state |

An invocation carrying an out-of-range parameter indicates an engine defect. The
appliance MUST reject it and SHOULD log it. This is how a manufacturer detects a
misbehaving engine in the field.

### 9.3 Determinism requirement

Normative, and central to the certification argument.

1. Every actuation parameter MUST be produced by the deterministic reasoning
   engine from declared signals, the skill, the resolved knowledge layers and
   structured intent.
2. A language model MUST NOT produce, modify or select any actuation parameter.
3. The language model's output MUST be confined to structured intent, validated
   against a closed schema before use.
4. Every invocation MUST be reproducible: identical signals, knowledge, skill and
   intent MUST produce an identical invocation.
5. The engine MUST retain, for each invocation, the inputs, the resolved
   knowledge layer for each value used, and the rule that produced it, sufficient
   to reconstruct the decision.

Requirement 5 is what lets a manufacturer answer the question a regulator or an
insurer will eventually ask, which is why the machine did that. Layer
attribution matters here: "the dose came from the vendor default, the temperature
from the user's learned preference" is the level of detail required.

### 9.4 One variable at a time

When correcting a diagnosed fault, the engine MUST NOT invoke two tools whose
`effects` sets intersect the same domain variable within one iteration, unless
the knowledge base explicitly declares the combination.

Where a skill declares `also_affects` (5.4), those variables count toward this
rule. A machine whose dose mechanism also shifts shot time is prevented from
having both adjusted at once.

---

## 10. Transport bindings

The core of this protocol is transport neutral. A binding MUST provide:

1. **Ordered, reliable delivery** of invocations and responses.
2. **Bounded latency**, below `session.control_timeout_ms`.
3. **Liveness detection** at the transport or protocol layer.
4. **Integrity** on the declaration, skill and knowledge extension.
5. **Authentication**, where the transport is reachable beyond the enclosure.

| Binding | Use |
|---|---|
| Shared memory or local IPC | Module on the appliance main board |
| UART with framing and CRC | Module as a separate board |
| BLE GATT | Retrofit and development |
| USB CDC | Development and factory test |

A network binding is permitted but MUST NOT be the only control path, since
requirement 2 cannot be guaranteed over an uncontrolled network.

### 10.1 BLE GATT sketch

| Characteristic | Properties | Contents |
|---|---|---|
| Declaration | read, chunked | The declaration document |
| Skill | read, chunked | The skill document |
| Knowledge | read, chunked | The knowledge extension |
| Invocation | write | One invocation, CBOR encoded |
| Response | notify | Lifecycle transitions and reason codes |
| Signals | notify | Batched signal frames |
| Events | notify | Discrete events |

Signal frames SHOULD batch to keep notification rate under 50Hz. Loss of
notification for longer than `control_timeout_ms` during a non-reversible
operation MUST trigger the failsafe.

---

## 11. Versioning

`adip_version` is `MAJOR.MINOR`. A minor increment MAY add optional fields,
registry entries or reason codes. A major increment MAY change or remove them.

An engine MUST refuse a declaration whose major version it does not implement. An
engine MUST tolerate unknown optional fields within a supported major version,
and MUST NOT infer meaning from an unrecognised semantic ID.

Four things version independently:

| Artifact | Versioned by | Owned by |
|---|---|---|
| The protocol | `adip_version` | StreetAI |
| The espresso registry | `domain_profile` | StreetAI |
| A machine's behaviour | `skill_version` | Manufacturer |
| A vendor's knowledge | `extension_version` | Manufacturer |

A manufacturer can revise its skill as it learns more about its own machine
without touching the declaration, and revise its knowledge without touching
either. None of it requires anything from StreetAI.

---

## 12. Conformance checklist

**All levels**

- [ ] Declaration retrievable before any invocation, with a stable `declaration_id`
- [ ] Every numeric parameter carries unit, range and resolution
- [ ] Every signal carries a declared accuracy and sample rate
- [ ] `monotonic` declared on every direction-bearing mechanism
- [ ] Envelope enforced independently of the engine
- [ ] Out-of-range invocations rejected, never silently clamped
- [ ] Failsafe reaches safe state within the declared time, verified by test
- [ ] Physical operator override aborts engine control and emits an event
- [ ] Fault events emitted before or with entry to a safe state

**Level 1 and above**

- [ ] `ACCEPTED` or `REJECTED` returned within 200ms
- [ ] All non-reversible tools marked `abortable` where physically possible
- [ ] Interlocks declared and enforced
- [ ] Skill provided covering the actuation tools the engine may select

**Level 2**

- [ ] Grinder retention declared in the skill with a remedy tool
- [ ] Settling times declared for every parameter that does not act immediately
- [ ] Dose accuracy declared and met across the working range
- [ ] Tamping declared, including `implicit: true` where mechanically inherent

**If a knowledge extension is supplied**

- [ ] All identifiers namespaced to the vendor
- [ ] No redefinition of base identifiers
- [ ] Constraint monotonicity verified: ranges narrowed, never widened
- [ ] Removing the extension returns the machine to base behaviour

---

## Appendix A: Level 0, external scale

The retrofit case. No actuation, no skill required.

```yaml
adip_version: "0.2"
domain: "espresso"
domain_profile: "espresso/v1"
conformance_level: 0

device:
  vendor: "StreetAI"
  model: "Coach Reference"
  firmware_version: "0.1.0"
  declaration_id: "sha256:9ab3..."

session:
  max_concurrent: 1
  heartbeat_interval_ms: 1000
  control_timeout_ms: 5000

signals:
  - id: "adi.espresso.signal.mass"
    name: "cup_mass"
    unit: g
    range: { min: 0, max: 2000 }
    sample_rate_hz: 20
    accuracy: { abs: 0.1, unit: g }
    filtered: false
  - id: "adi.espresso.signal.elapsed"
    name: "elapsed"
    unit: s
    sample_rate_hz: 20
    accuracy: { abs: 0.05, unit: s }

tools: []

events:
  - id: "adi.espresso.event.first_drip"
    payload: { elapsed_s: { type: number, unit: s } }

envelope:
  enforced_by: "firmware"
  independent_of_engine: true
  limits: []
  interlocks: []
  failsafe:
    on_control_timeout: "abort_to_safe"
    on_signal_loss: "abort_to_safe"
    on_fault: "abort_to_safe"
    safe_state: "idle"
    max_time_to_safe_ms: 100
```

Two signals, no tools, full diagnosis, and it works on any espresso machine ever
built.

---

## Appendix B: Level 2 skill, abridged

```yaml
skill_version: "1.0"
skill_id: "skill/EX-900/2.1"
applies_to:
  vendor: "Example Appliances"
  model: "EX-900"
  firmware_range: { min: "3.0.0", max: "3.9.99" }

responses:
  - { tool: "adi.espresso.grind.set_setting", parameter: "setting",
      variable: "shot_time", per_unit: -1.4, unit: s,
      valid_range: { min: 8, max: 45 }, confidence: 0.75 }
  - { tool: "adi.espresso.grind.set_setting", parameter: "setting",
      variable: "flow_peak", per_unit: 0.06, unit: g_per_s,
      valid_range: { min: 8, max: 45 }, confidence: 0.5 }
  - { tool: "adi.espresso.brew.set_temperature", parameter: "temperature",
      variable: "shot_time", per_unit: -0.4, unit: s,
      valid_range: { min: 88.0, max: 94.0 }, confidence: 0.5 }
  - { tool: "adi.espresso.dose.by_mass", parameter: "mass",
      variable: "shot_time", per_unit: 1.1, unit: s,
      valid_range: { min: 7.0, max: 12.0 }, confidence: 0.6 }

dynamics:
  - tool: "adi.espresso.grind.set_setting"
    retention:
      mass: { value: 0.8, unit: g }
      remedy_tool: "adi.espresso.grind.purge"
    hysteresis:
      steps: 2
      direction: "coarser_to_finer"
  - tool: "adi.espresso.brew.set_temperature"
    settling:
      time: { value: 15, unit: s }
      tolerance: { value: 0.3, unit: c }

interactions:
  - { changing: "grind_setting", invalidates: ["last_shot_baseline"] }
  - { changing: "dose_mass", also_affects: ["shot_time", "extraction_yield"] }

operating:
  reliable_ranges:
    - { variable: "brew_temperature", min: 88.0, max: 94.0, unit: c,
        note_id: "temp_ceiling" }
  recovery:
    between_shots: { value: 25, unit: s }

notes:
  - id: "temp_ceiling"
    text: "Above 94C the boiler cannot hold setpoint across consecutive shots."
```

---

## Appendix C: What a manufacturer actually implements

1. **Publish the declaration.** Mostly a description of hardware that already
   exists.
2. **Implement the invocation handler:** validate, check preconditions, execute,
   report. Most of the engineering effort sits here.
3. **Expose the envelope.** Usually already present in firmware and needs
   declaring plus testing against the failsafe timing.
4. **Stream signals** from sensors already fitted.
5. **Wire the physical override** to session abort.
6. **Write the skill.** A day or two of measurement by someone who already knows
   the machine. This is the highest-leverage document a manufacturer produces,
   because it is what makes the intelligence work well rather than merely work.
7. **Optionally, a knowledge extension**, if the product has drinks, beans or
   defaults the base knowledge does not cover.

No reasoning, no models and no domain knowledge are implemented by the
manufacturer. Those come from StreetAI and are identical on every machine.
