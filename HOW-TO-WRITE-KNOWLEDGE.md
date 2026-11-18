# How to write knowledge

A declaration says what a machine can do. A **knowledge layer** says what the
things it makes are: the styles, the material priors, the notes. See ADI-P 6
for the format and `HOW-TO-ADD-A-DOMAIN.md` for writing a profile in code.

This guide is about the content, because that is where the mistakes are.

Knowledge is the only part of an ADI deployment a model reads as prose and acts
on. Everything else is checked: a value is clamped to a declared range, an
answer validated against a schema, a plan refused if it moves one variable
twice. A style's description is checked by nothing. It is advice, taken
literally, by something with no other source of information about what you sell.

Every fault below has happened, in a shipped profile or a manufacturer's layer,
and each one looked like a reasonable sentence at the time.

---

## 1. State what a thing is, not how it is made

A style describes an outcome. The declaration already describes the mechanisms.
A description that reaches for a mechanism invites a model to drive it.

```yaml
# Wrong. The mechanism is named, and the machine has a variable for it.
- id: "x.example.style.lungo"
  description: >
    A long cup. More water through the same grounds, so it is weaker.

# Right. What ends up in the cup.
- id: "x.example.style.lungo"
  description: >
    A long cup: weaker and lighter than a short one, filling a larger vessel.
```

The first version produced orders that drove a separate hot-water mechanism
belonging to a different style — because the machine had such a variable, the
prose pointed at it, and connecting the two was the obvious inference. It was
not a misreading. It was the only reading available.

This generalises past drinks. A kiln style that explains itself as *"held
longer at temperature"* will have something trying to extend a hold time that
the recipe never mentions. Say what the fired piece is like.

---

## 2. A style MUST NOT claim a generic word

The commonest fault, and the hardest to see, because the sentence reads as
helpfulness.

```yaml
# Wrong. This is editorial, and it is in the wrong file.
- id: "adi.espresso.style.espresso"
  description: >
    A short, strong cup. The default where someone asks for a coffee without
    saying which kind.
```

Two things are wrong with it.

**It decides something that is not a profile's business.** What a person means
by a bare generic word depends on the market, the venue and the machine in
front of them. A profile describes a class of machine and cannot know.

**It collides.** Some other style will also have a claim on that word, and then
the behaviour depends on which description a model weighs more heavily on the
day. The symptom is the worst kind: a machine that asks which kind on some runs
and picks silently on others, from identical input.

That exact sentence was found three times — twice in a shipped profile and once
in a manufacturer's layer that had copied it.

**The rule.** A style says what it is and what words name *it*. Where several
styles could answer a bare word, the right behaviour is to ask, and an engine
will ask on its own once no style claims to be the answer.

```yaml
# Right. It names itself and claims nothing it does not own.
- id: "adi.espresso.style.espresso"
  description: >
    A short, strong cup, and the base of most milk drinks. What someone means
    by a short black, a shot or a single.
```

---

## 3. Give every target an aim

ADI-P 6.2 lets a range carry `aim`: the value the parameter is set to when this
style is produced and nothing else was asked for.

**Write one.** Where it is absent an engine uses the midpoint of the range,
which is arithmetic standing in for a figure you did not state.

```yaml
# Thin. The engine will aim at 110, whether or not that is your recipe.
targets:
  yield_mass: { min: 90.0, max: 130.0, unit: g }

# Complete. The recipe is 120; the range is tolerance.
targets:
  yield_mass: { aim: 120.0, min: 90.0, max: 130.0, unit: g }
```

A range on its own says how far a parameter may be pushed. It does not say what
the product is. A vendor whose recipe sits off the centre of its tolerance has
no other way to say so, and a model shown a range and asked for a figure will
offer one from inside it that has no relation to your intent.

---

## 4. Two styles with the same figures are one style with two names

If two styles declare the same targets over the same ranges, they are
indistinguishable to the engine, however different their prose.

This is easy to create by accident and invisible until someone compares. The
check is mechanical: for each pair of styles, do their effective targets differ?
If not, either the figures are wrong or one of the styles should not exist.

An `aim` is often what separates them. Two products may share a tolerance band
and sit at different points inside it, and before you write the aims they are
the same product.

---

## 5. Descriptions are read by a model, so write for a reader

Not prose for its own sake — a description is the only thing standing between a
person's words and the right style.

**Say what someone would call it.** Including the informal words. A model maps
*"something short and strong"* onto a style through its description or not at
all; an identifier never does the job, and never can for a style a manufacturer
introduced.

**Keep it to one idea.** A description carrying three clauses about three
different qualities gives a model three things to match on, and it will match on
the wrong one.

**Do not restate the figures in prose.** They are supplied to the model
separately and in a form it cannot misread (ADI-P 9.4.1). Prose figures go stale
the moment someone edits the range, and then the layer disagrees with itself.

**Do not write instructions to the model.** A description is a fact about a
product. *"Prefer this one where the customer seems unsure"* is a policy, it
belongs nowhere in a knowledge layer, and an engine will not honour it.

---

## 6. A note answers a question; it does not set a value

Notes (ADI-P 6.7) are free text a machine may answer from: the basket size, the
water hardness assumed, why a figure is what it is.

They reach no actuation value, and they must not try. A note reading *"we run
these a little hotter than standard"* will be quoted at a person as fact and
will change nothing, which is worse than either saying nothing or declaring the
temperature.

If a figure matters, declare it. If the reasoning behind a figure matters,
that is what a note is for.

---

## 7. Check the layer against the machine, not only against the schema

Validation catches what ADI-P 6.5 can catch: namespacing, redefinition, ranges
that cannot resolve, a style outside its constraints. It cannot catch a style
describing something the machine in front of it has no way to produce.

A base profile describes a class of machine. It will offer styles written in
terms of mechanisms a given machine does not have, and that is legitimate — but
a machine must not offer a person a style it cannot make.

**Check which variables the machine can actually move**, with the declaration
loaded:

```sh
adi capabilities <machine>.yaml --skill <skill>.yaml --knowledge <layer>.yaml
```

It prints the tools the engine may invoke and, under
`variables the engine can move on this machine`, the outcomes it has a mechanism
for. It does not print styles — the check is to compare that list against each
style's targets yourself.

**A style whose targets include a variable absent from that list cannot be
produced on this machine.** That is the thing to look for, and it is worth doing
deliberately, because the failure is quiet: an engine will either refuse the
order or quietly leave that variable alone, and neither tells you the layer and
the machine disagree.

Two readings to be careful of. A style may be perfectly producible while one of
its targets is unservable, if the engine seeds that target and no tool contests
it. And if *no* style survives the comparison, suspect the layer or the
declaration rather than the machine — the only time that has happened in
practice, the fault was a filter in the engine hiding every style on a
correctly written appliance.

---

## Review checklist

Before shipping a knowledge layer:

- [ ] No description names a mechanism, a pump, a dial or a duration.
- [ ] No style claims a generic word, or says it is a default.
- [ ] Every target carries an `aim`.
- [ ] No two styles resolve to the same effective targets.
- [ ] Every description includes the informal words a person would use.
- [ ] No description restates a figure that is declared elsewhere.
- [ ] No description or note instructs the model.
- [ ] Every style's targets name only variables that `adi capabilities` lists
      as movable, against a real declaration.

Seven of the eight have a real fault behind them. The eighth is the one that
catches the rest.
