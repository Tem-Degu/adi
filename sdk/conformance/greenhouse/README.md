# Greenhouse: the honesty test

**This is not a product.** There is no hardware, no code and no plan to build
one. It exists so the neutral crates cannot quietly become espresso-shaped.

You cannot design a good abstraction from one example. Building "for any
appliance" from espresso alone reliably produces something that looks general
and fits only coffee. The standard fix is to build two products. This is the
cheap version: a second profile as **data only**.

Greenhouse was chosen because it breaks espresso's assumptions in exactly the
places that matter:

| | Espresso | Greenhouse |
|---|---|---|
| Cycle length | 30 seconds | days to weeks |
| Feedback | taste, immediately | plant health, delayed |
| Correction shows up | next cycle | next week |
| Cycles overlap | never | always |
| Subject | one shot | many plants at once |
| A wrong move costs | one coffee | a crop |

## How to use it

Whenever you add something to `adi-protocol`, `adi-safety`, `adi-core` or
`adi-decision`, check it against `registry.yaml` here. Ask:

1. Does this concept exist in a domain where a cycle takes three weeks?
2. Does it survive cycles that overlap rather than run one at a time?
3. Does it survive feedback arriving long after the action?
4. Does it survive there being no single "result" to rate?

If the answer is no, what you added was espresso logic wearing a general name.

## Known pressure points

Things this profile already suggests the neutral layer will need, and that
espresso alone would never reveal:

- **`Cycle` assumes one bounded run with a start and an end.** A greenhouse has
  no such boundary. Either cycles need to be allowed to overlap, or the concept
  needs to be a window over continuous operation.
- **`SubjectiveFeedback` assumes feedback follows its cycle.** Here it arrives
  weeks later and refers to a period rather than an event, so feedback needs to
  carry the time range it describes.
- **Correction confidence has no time cost model.** Overshooting espresso costs
  thirty seconds. Overshooting here costs a fortnight, so step sizing should
  scale with the cost of being wrong, not only with confidence.
- **`Trace` assumes samples relative to a cycle start.** Continuous domains want
  absolute time.

None of these are urgent. All of them are cheaper to know about now than to
discover on the first real second domain.
