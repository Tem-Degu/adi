# Artificial Domain Intelligence: adding a domain

A **domain** is what the machine is for. It answers two questions: what counts
as a good result, and when a result is bad, what caused it.

The first is a description, and it is data. The second is judgement, and it is
code. The split is the point of this document, because most machines need only
the first to become useful.

This guide has two parts. Part one needs no programming beyond your own
firmware. Part two is for when you want the machine to know why something went
wrong.

Read [`HOW-TO-PORT-TO-A-BOARD.md`](./HOW-TO-PORT-TO-A-BOARD.md) and
[`HOW-TO-ADD-A-TOOL.md`](./HOW-TO-ADD-A-TOOL.md) first. This one assumes your
machine already answers and has at least one tool.

---

## Part one: describe the machine

Say `base/v1` in your declaration:

```yaml
domain_profile: "base/v1"
```

That profile ships no knowledge of its own. Everything your machine reasons
about comes from one more file, a knowledge extension, and that file is your
domain.

A worked example is in `sdk/examples/declarations/printer.*`: a printer, which
is not a coffee machine and has no profile written for it.

### What goes in it

**Your outcomes.** Not the controls, the results. Each with a unit and a
sentence saying what it measures and which direction means what. The sentence
is what lets "it came out faint" reach the right outcome, and a model given a
bare identifier is guessing.

```yaml
variables:
  x.inkworks.ink_coverage:
    unit: pct
    description: >
      Share of the page the ink actually covers. Low reads as faint and washed
      out; high reads as heavy and slow to dry.
```

**What a good result is.** Named sets of targets. The engine compares what was
measured against the one that was asked for.

```yaml
styles:
  - id: "x.inkworks.style.photo"
    targets:
      x.inkworks.ink_coverage: { min: 55.0, max: 70.0, unit: pct }
```

**What the machine should be able to say.** Free text, which may inform an
answer and may never determine an action.

Every identifier you introduce must sit in your own namespace, and validation
refuses one that does not. On this profile that rule carries more weight than
usual, because the profile declares nothing itself, so it is the only thing
keeping two manufacturers' identifiers apart.

### One more file you already have

Your skill says how far each control moves each outcome. Without an entry
there a tool is callable and never selectable: the engine builds its candidate
list from those entries, so a tool naming no outcome is never offered and no
spoken request can reach it.

### What you get

Spoken requests reach your tools. Questions are answered from what you
declared. An outcome that drifts outside its target is corrected back towards
it, cautiously, over two or three cycles.

```sh
adi validate my.yaml --skill my.skill.yaml --knowledge my.knowledge.yaml
adi capabilities my.yaml --skill my.skill.yaml --knowledge my.knowledge.yaml
```

`capabilities` is the one to read. It lists what the engine can actually move,
and names any outcome your skill mentions that nothing declared, which is the
commonest reason a tool is never selected.

### What you do not get

A cause.

The base profile knows that an outcome drifted. It cannot tell a blocked print
head from a low reservoir, and both read as low coverage. Told to fix low
coverage it will add ink, which spends ink and leaves a blocked head blocked.

Knowing the difference, and knowing which correction would make things worse,
is what a profile is for.

---

## Part two: write a profile

A profile supplies two things the base cannot.

**Perception.** Turning raw sensor curves into outcomes. Espresso reads one
mass curve and produces four variables from it: the yield, the shot time and
two flow figures. Nothing general can do that, because the arithmetic is
specific to the machine.

Skip this if your appliance reports finished measurements rather than raw
curves. Most can, and then there is nothing to write.

**Diagnosis, in order.** Which fault a bad result points to, and critically
which faults must be ruled out first. In espresso, erratic flow means the puck
was packed badly, and prescribing a grind change for it ruins the next cup. So
that check comes first, and the ordering is the whole value of the file.

For a printer the equivalent is: check for banding before acting on low
coverage, because banding is a blocked head and more ink will not fix it.

### How

Implement `Domain`, in `sdk/crates/adi-core/src/domain.rs`. Six methods, two of
them optional. Copy `sdk/crates/adi-domain-espresso/`, which is 489 lines
including its comments, and `sdk/crates/adi-domain-base/`, which is the
smallest complete example.

Your knowledge base becomes a file your profile carries rather than one a
manufacturer supplies, and manufacturers extend it exactly as they extend
espresso.

### Making it load

The shipped `adi` tool can only run profiles compiled into it, which is why a
profile it does not have is reported rather than guessed at. It is a
development driver and not the binary that goes inside a machine.

A product links the engine as a library and names its own profile, which is a
few lines. If you want your profile in the shipped tool, add it to
`sdk/crates/adi-cli/src/domain.rs`, which is one line in a `match`.

---

## Which rung to start on

Start on part one. It takes an afternoon, it needs no Rust, and it tells you
whether the outcomes you chose are the ones people actually ask about. Those
are the same outcomes a profile would diagnose, so nothing is wasted.

Write a profile when you can state, in order, the faults your machine has and
which correction each one rules out. If you cannot write that list yet, a
profile would encode a guess, and a guess that acts on a machine is worse than
an honest "it drifted".

---

## What a profile cannot change

A profile decides what the machine understands. It never decides what the
machine may do. Actions come from the declaration, are clamped to declared
ranges, pass the safety envelope, and are validated again by the appliance. A
profile that is wrong produces a poor correction and never an unsafe one.
