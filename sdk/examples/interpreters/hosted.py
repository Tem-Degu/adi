#!/usr/bin/env python3
"""A model backend served by a hosted API.

Serves two seams through the `process` backend: the interpreter, which turns
an utterance into structured intent, and the chooser, which picks which
declared action serves a goal and at D2 what value to send. One JSON request
per line in, one JSON answer per line out. The engine starts two copies of
this program, one per seam, and each tells which it is from the shape of the
message rather than from an argument.

Why hosted at all. ADI is designed to run locally and the deterministic path
needs no model whatever. The reason to reach for an API is the one measured in
sdk/README.md: the local interpreter took 18 s median on a processor of this
class, an order of magnitude outside what a person waiting at a machine will
tolerate. A hosted call is a second or two.

Three things sdk/README.md says to get right, all of them below.

  A timeout. The engine blocks waiting for a backend and imposes no limit of
  its own. While it blocks it stops sending heartbeats, so a stalled
  connection would trip the appliance's failsafe after control_timeout_ms and
  present as a mystery hardware fault. REQUEST_TIMEOUT is set well inside the
  6000 ms bob.yaml declares, and an expiry returns a question rather than
  raising.

  A schema, not an instruction. Instruction is advice; a schema is not. The
  schema below is built from each request, so the model can only name a
  variable this machine declares, a style it offers, or a candidate index the
  engine actually supplied.

  A decision about what leaves the building. What is sent is listed in
  REDACTION. No audio passes through this file.

Configure with:

    seams:
      interpreter:
        backend: process
        command: python
        args: ["examples/interpreters/hosted.py"]

Environment:
    OPENAI_API_KEY   required. Never write it into a file in this repository.
    OPENAI_MODEL     optional, defaults below.
    ADI_HTTP_TIMEOUT optional, seconds.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import json
import io
import os
import sys
import urllib.error
import urllib.request

API_URL = "https://api.openai.com/v1/chat/completions"
# Named by the configuration that starts this program, so which model a
# machine uses is visible in its own config rather than in an environment
# variable nobody reads. An argument wins, then the environment, then this.
#
# Not a free choice, and measured rather than assumed. Over twelve trials
# gpt-4o-mini decided badly about one order in three - brewing with no water,
# or sending nothing at all - while gpt-5 got all twelve right and timed out
# twice at forty-five seconds.
#
# Round trips on a small prompt, four calls each:
#
#   gpt-4o-mini   1.4 - 1.6 s
#   gpt-5-mini    3.6 - 4.8 s
#   gpt-4.1       1.1 - 1.4 s
#
# So the obvious trade - accuracy for speed - turned out not to exist. gpt-4.1
# is both the faster of the three and the stronger of the two cheap ones. Bob's
# real prompts are much larger than that test's, so treat these as a floor.
MODEL = (
    sys.argv[1]
    if len(sys.argv) > 1 and not sys.argv[1].startswith("-")
    else os.environ.get("OPENAI_MODEL", "gpt-4o-mini")
)
# This MUST stay below the machine's control_timeout_ms, which is 6000 ms in
# bob.yaml. The engine blocks here and stops sending heartbeats meanwhile, so
# a timeout longer than that deadline means the appliance decides the link is
# dead and enters its safe state - and the cause is not remotely obvious from
# the symptom. Measured latency on this model is around 1 s, so four seconds
# is comfortable headroom and still well inside the deadline.
# Four seconds was too tight in practice: gpt-4o-mini regularly takes longer
# on a first call, and the engine then reports "could you say that again?" for
# a question it understood perfectly well. Ten is comfortable.
#
# IT STILL MATTERS THAT THIS IS BOUNDED, and the bound is not arbitrary. The
# engine blocks while a backend thinks, and while it blocks it sends no
# heartbeat. An appliance that hears nothing for its control_timeout_ms treats
# its controller as dead. When nothing is running that costs only a missed
# heartbeat; when something IS running the appliance abandons it, and a dropped
# coffee presents as a mystery hardware fault.
#
# So ten seconds is safe for a call made at the prompt, and would NOT be safe
# for one made during a drink - Bob's tolerance is six. Anything that comes to
# ask a question mid-operation needs its own, tighter limit, well inside that
# machine's control_timeout_ms.
REQUEST_TIMEOUT = float(os.environ.get("ADI_HTTP_TIMEOUT", "10.0"))

# What leaves the building, stated plainly so the decision is visible rather
# than buried: the utterance, the machine's identity, the names and meanings of
# the outcomes it models, its drink styles, and the manufacturer's notes. No
# audio, no identifiers, nothing about who is standing in front of the machine.
# The speech seams are not served from this file at all.
REDACTION = "utterance, machine identity, modelled outcomes, styles, notes"


# The engine sends its own `instruction` with every request, and that is the
# contract its reply is checked against. It is used verbatim here rather than
# restated, for the reason the engine's own comment on that field gives:
#
#   "A backend that holds its own copy of this has a copy that can drift, and
#    one did: the shipped Qwen backend's copy was missing the answer arm, so
#    the machine could not answer a question at all."
#
# Two things are added, and they are kept in a separate block rather than
# folded in so that what is ours stays visible.
ADDENDUM = """

ADDITIONS FROM THIS BACKEND, which are not part of the engine's own contract:

Where someone asks for a drink by name, name the style and give NO amount. The
machine holds the recipe for each drink it offers and will look it up. An
amount you supply overrides that recipe, so inventing one replaces a figure the
manufacturer measured with a figure you guessed.

WHERE SEVERAL STYLES FIT WHAT THEY SAID, ASK WHICH. "A coffee" on a machine
offering a ristretto, an espresso, a doppio, a coffee, a lungo and an americano
names six of them, and picking one is guessing at the order. Ask, in one short
question, naming a few of the choices. Only pick for them when one style
plainly matches the words - "a cappuccino" is a cappuccino - or when they have
said they do not mind.

Prefer this machine's own styles over the generic ones where both would fit.
A vendor style exists because the manufacturer measured it on this hardware; a
generic one describes the class of machine and may ask for something this
particular machine cannot deliver.

Give an amount only where the person stated one ("a forty gram coffee"), or
where they are changing something rather than ordering a drink ("more syrup",
"make it hotter"). A request that changes something MUST name what changes,
because there is no recipe to fall back on.

"ANOTHER ONE", "THE SAME AGAIN", "ONE MORE" MEAN THE DRINK THEY LAST HAD.
Look it up in the decisions you are given and order that style again. It does
NOT mean a fresh cup: "new_cup" tares the scale and makes nothing, so answering
with it leaves somebody who asked for a second coffee holding an empty cup and
being told it was done.

Where someone asks for TWO OR MORE of the numbered actions in one breath -
"rinse the milk and then make a cappuccino" - use reply_type "actions" and give
their numbers in the order they should happen. Each is carried out in turn, and
a failure stops the rest, which is why the order is yours to get right. Use it
only for a genuine sequence; a single action is "action".

Where the person asked for one of the numbered actions rather than for an
outcome, use reply_type "action" and give its number. Those are the things the
machine does that are no quantity at all: stopping what is pouring, declaring a
fresh cup, running a rinse. They take no value and there is no field for one.

Do not answer a request for one of those as though you had carried it out. The
machine performs it; saying it is done without naming the action means nothing
happens and the person is told otherwise.

Prefer an outcome this machine can actually move. The variable list describes
everything the domain models, not everything this machine has a mechanism for,
and naming one it cannot move gets the whole request refused.

A NAMED STYLE ALREADY CARRIES ITS FIGURES. Each drink above is listed with
what it aims for - "yield_mass = 110 g (may be 90 to 130)". Naming the style is
enough: give NO target for anything the style already states, and the machine
uses the recipe's own figure.

Give one only where the person asked for something different - "a lungo but
only 100 grams" - and then give the figure THEY said, not one of your own.

This is worth being exact about. Showing these figures without saying this made
the machine worse: a model that could see "90 to 130" helpfully filled in 95,
and a long coffee came out a fifth short of what the recipe asks for. Nothing
was wrong with 95 except that nobody had asked for it.

Give a target ONLY for what the request actually asks to change. Do not fill
in the other outcomes, and never list one at zero to mean "leave it alone": a
target of zero is an instruction to make it zero, and the machine will carry it
out. Asking for a coffee and listing syrup at 0 dispenses syrup. One or two
targets is the normal answer; the whole list never is.

Keep questions and answers to one or two short spoken sentences. You are
talking to someone standing at a machine, not writing to a screen.

A question about WHY the machine did something is answered, not acted on: use
reply_type "answer" and draw the answer from the recent decisions given below.
Say the figure that was sent and where it came from - "38 millilitres, because
the recipe for an espresso asks for 30 to 42 grams in the cup". If a limit
moved the value, say so. Never invent a reason, and never state a figure the
decisions do not contain: if they do not say, the honest answer is that you
cannot tell from what you were given.

A goal and a setting are not the same number. `note` gives the outcome wanted
in the cup; `arguments` gives what the mechanism was set to, which the engine
worked out from it. Never compare the two figures or call one larger than the
other - 116 millilitres of water making 110 grams of coffee is the machine
doing its job. And never say an identifier out loud: "yield_mass" is a name in
a file, and the words are "coffee in the cup".

With an intent or an action you MAY also fill `say` with one short friendly
sentence to speak out loud while the machine does it - "Cappuccino, coming
right up." Acknowledge what was asked for and nothing else: state no quantity,
no setting and no result, because you do not know what the machine will choose
and this is spoken before anything has happened. Never say it worked. Leave
`say` null for a clarification or an answer, which are already the sentence,
and null whenever nothing needs saying.
"""


# --------------------------------------------------------------- the replies


def _trace(label, text):
    """Write what the model was shown to ADI_TRACE, if that is set.

    There is no other way to see it. The engine assembles the context, this
    program turns it into a prompt, and the prompt is gone the moment the
    request returns - so a wrong answer cannot be told apart from a right
    answer to a prompt that was missing something. Off unless asked for, and
    the file is opened in append mode so one run holds every seam.
    """
    path = os.environ.get("ADI_TRACE")
    if not path:
        return
    try:
        with io.open(path, "a", encoding="utf-8") as fh:
            fh.write("\n===== %s =====\n" % label)
            fh.write(text)
            fh.write("\n")
    except OSError:
        pass


def clarify(question, because):
    """Ask rather than guess. ADI-P 9.4.3 makes this an outcome, not a failure."""
    return {"clarify": {"question": question, "because": because}}


def call_model(system, user, schema):
    """One request, schema-constrained, with a timeout that cannot hang.

    Returns the parsed object, or None if anything at all went wrong. The
    caller decides what a failure means for its seam, because the two fail in
    opposite directions: an interpreter asks, a chooser refuses.
    """
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        sys.stderr.write("hosted.py: OPENAI_API_KEY is not set\n")
        return None

    body = json.dumps(
        {
            "model": MODEL,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "adi_reply",
                    "strict": True,
                    "schema": schema,
                },
            },
        }
    ).encode("utf-8")

    request = urllib.request.Request(
        API_URL,
        data=body,
        headers={
            "Authorization": "Bearer " + key,
            "Content-Type": "application/json",
        },
    )

    try:
        with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT) as response:
            payload = json.loads(response.read().decode("utf-8"))
        reply = json.loads(payload["choices"][0]["message"]["content"])
        _trace("SYSTEM", system)
        _trace("USER", user)
        _trace("REPLY", json.dumps(reply, indent=2))
        return reply
    except urllib.error.HTTPError as error:
        sys.stderr.write(
            "hosted.py: HTTP {}: {}\n".format(
                error.code, error.read()[:400].decode("utf-8", "replace")
            )
        )
    except Exception as error:  # noqa: BLE001
        # A timeout lands here, and so does a malformed reply. Both are
        # ordinary on a network and neither may take the seam down.
        sys.stderr.write("hosted.py: {}\n".format(error))
    return None


# ------------------------------------------------------------- interpreting


def interpreter_schema(request):
    """The schema for one utterance, narrowed to this machine.

    Built per request rather than written down, so a model cannot name a
    variable this machine does not model or a style it does not offer. That is
    the difference between instructing a model and constraining it.

    The engine supplies its own schema too, but with `oneOf` at the root, which
    strict structured output does not accept. This is the same contract
    expressed flat, and interpret() converts it back.
    """
    variables = [v["name"] for v in request.get("variables", [])] or ["none"]
    # Every unit the protocol knows, from adi-protocol/src/units.rs. Left as a
    # free string, a model once returned "ml'}]}]}" and the engine refused the
    # whole reply as malformed. A schema removes the possibility.
    actions = list(range(len(request.get("actions", []) or [])))
    units = [
        "none", "g", "g_per_s", "ml", "ml_per_s",
        "c", "bar", "s", "ms", "step", "kg_f", "pct",
    ]
    styles = [s["name"] for s in request.get("styles", []) or []]
    materials = list(request.get("materials", []) or [])

    return {
        "type": "object",
        "additionalProperties": False,
        "required": [
            "reply_type",
            "action",
            "actions",
            "kind",
            "style",
            "material",
            "targets",
            "question",
            "because",
            "answer_text",
            "drew_on",
            "say",
        ],
        "properties": {
            "reply_type": {
                "type": "string",
                "enum": ["intent", "clarify", "answer"]
                + (["action", "actions"] if actions else []),
            },
            # Narrowed to the numbers the engine actually offered, so a model
            # cannot name an action this machine does not declare. The engine
            # refuses an unknown index anyway; this stops it being produced.
            "action": {"type": ["integer", "null"], "enum": actions + [None]},
            # Several of them, in the order they should happen. "Rinse the milk
            # and then make a cappuccino" is two of the machine's own products
            # in one breath, and `action` has one slot. Still indices into the
            # list the engine offered, so nothing opens up by allowing a
            # sequence: the engine carries each out on its own, through the
            # envelope and with its own record.
            "actions": {
                "type": ["array", "null"],
                "items": {"type": "integer", "enum": actions},
                "minItems": 2,
            },
            # What to say out loud while the machine does it. Beside the
            # decision, never inside it: nothing parses this, it is spoken and
            # discarded, so it cannot carry a tool name or a quantity into the
            # actuation path.
            #
            # Null for a clarification or an answer - those ARE the sentence,
            # and acknowledging one would be talking over it.
            "say": {"type": ["string", "null"]},
            "kind": {"type": ["string", "null"], "enum": ["produce", "adjust", None]},
            "style": {"type": ["string", "null"], "enum": styles + [None]},
            "material": {"type": ["string", "null"], "enum": materials + [None]},
            "targets": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": [
                        "variable",
                        "mode",
                        "value",
                        "unit",
                        "of",
                        "scale",
                        "offset_value",
                        "offset_unit",
                    ],
                    "properties": {
                        "variable": {"type": "string", "enum": variables},
                        # A relation is stated, never resolved: the engine
                        # holds the memory and does the arithmetic. A backend
                        # that returned "4 g" would have to be trusted to
                        # remember correctly and to divide correctly, and would
                        # be wrong silently when it did not.
                        "mode": {"type": "string", "enum": ["absolute", "relative"]},
                        "value": {"type": ["number", "null"]},
                        "unit": {"type": ["string", "null"], "enum": units + [None]},
                        "of": {"type": ["string", "null"]},
                        "scale": {"type": ["number", "null"]},
                        "offset_value": {"type": ["number", "null"]},
                        "offset_unit": {
                            "type": ["string", "null"],
                            "enum": units + [None],
                        },
                    },
                },
            },
            "question": {"type": ["string", "null"]},
            "because": {"type": ["string", "null"]},
            "answer_text": {"type": ["string", "null"]},
            "drew_on": {"type": "array", "items": {"type": "string"}},
        },
    }


def interpreter_prompt(request):
    """Everything the model is given, and nothing else."""
    lines = ["The person said: " + json.dumps(request.get("utterance", ""))]

    machine = request.get("machine")
    if machine:
        lines.append("\nThis machine is: " + machine)

    variables = request.get("variables", [])
    if variables:
        lines.append("\nOutcomes the domain models:")
        for v in variables:
            lines.append(
                "  {} (in {}): {}".format(
                    v["name"], v.get("unit", "none"), (v.get("meaning") or "").strip()
                )
            )

    styles = request.get("styles", [])
    if styles:
        # The descriptions are what let a request in someone's own words reach
        # the right drink. A model has no idea what x.bob.style.ristretto is.
        lines.append("\nDrinks it offers:")
        for style in styles:
            lines.append(
                "  {}: {}".format(
                    style["name"], (style.get("description") or "").strip()
                )
            )
            # The recipe itself, so a figure offered alongside a style is
            # informed rather than guessed. Without this a model saw a name
            # and a sentence, and one asked for 85 grams of a lungo whose
            # recipe says 90 to 130 - not ignoring the band, never shown it.
            #
            # It also shows what a drink does NOT specify, which is the other
            # half of the same mistake: "more water through the coffee" had a
            # model reach for a hot water step belonging to another drink.
            for variable, band in style.get("targets") or []:
                lines.append("      {} = {}".format(variable, band))

    actions = request.get("actions", [])
    if actions:
        # Numbered, because the number is the whole of the reply. The
        # description is what tells these apart: "stop" and "rinse" as bare
        # words are a coin toss whose outcome is a physical act.
        lines.append("\nActions it can carry out, by number:")
        for action in actions:
            text = (action.get("description") or "").strip() or "no description given"
            lines.append(
                "  {}: {} - {}".format(action["index"], action["name"], text)
            )

    materials = request.get("materials", [])
    if materials:
        lines.append("\nMaterials it knows: " + ", ".join(materials))

    measured = request.get("measured", [])
    if measured:
        lines.append("\nWhat it is reading now:")
        for reading in measured:
            lines.append("  " + json.dumps(reading))

    remembered = request.get("remembered", [])
    if remembered:
        lines.append("\nWhat it remembers making:")
        for item in remembered:
            lines.append("  " + json.dumps(item))

    conversation = request.get("conversation", [])
    if conversation:
        lines.append("")
        lines.append(
            "The conversation so far, oldest first. This is what was said "
            "between you and this person before the sentence you are "
            "answering now. Read it as you would read any conversation: it "
            "says what they have already told you and what you have already "
            "told them."
        )
        lines.append(
            "  One thing it is NOT: a source of numbers. Every quantity comes "
            "from a style, a skill response or the declared envelope. Where "
            "something said here and a decision below disagree, the decision "
            "is what actually happened."
        )
        for turn in conversation:
            lines.append("  they said: " + turn.get("said", ""))
            for reply in turn.get("replies", []) or []:
                lines.append("  you said:  " + reply)

    history = request.get("history", [])
    if history:
        # The engine has always sent this and this prompt never used it, so a
        # model could not answer "why did you do that" even about the decision
        # it had just made. Each entry now carries what was sent, what was
        # asked for before any limit moved it, and which layer every figure
        # came from - which is the actual answer to "why".
        lines.append(
            "\nWhat it has done recently, most recent first. Two things are "
            "answered from this list and nothing else:\n"
            "  - WHY it did something: name the figure and where it came "
            "from, and never guess beyond what is here.\n"
            "  - WHICH DRINK a past decision was: read `style`, not "
            "`tool`. The tool is the mechanism, and one brewer serves every "
            "size of coffee this machine makes, so `brew.start` with 118 "
            "millilitres could be any of them. `style` names the recipe. "
            "Where a decision has no style it was a named product or a plain "
            "adjustment, and `tool` names it well enough.\n"
            "  - THE USUAL. If someone asks for the usual, or the same "
            "again, work out from this list what they have ACTUALLY been "
            "ordering and treat that as the request. Do not ask them which "
            "drink they mean when the list already says. Nothing is saved "
            "anywhere - this list IS the memory, so read it afresh each "
            "time. Say which drink you settled on and roughly how often it "
            "appears, so they can correct you. If the list shows no clear "
            "pattern, or is nearly empty, say so plainly rather than "
            "picking one:"
        )
        for decision in history:
            lines.append("  " + json.dumps(decision))

    notes = request.get("notes", [])
    if notes:
        # Sent whole, on every utterance, with no relevance filter: selecting
        # by relevance guesses what a question needs, and a wrong guess
        # withholds the answer silently. Keeping the total inside the model's
        # context is the declaration author's responsibility, and the engine
        # warns when it has had to truncate.
        lines.append("\nWhat the manufacturer wrote down:")
        for note in notes:
            lines.append("  [{}] {}".format(note["id"], note["text"].strip()))

    return "\n".join(lines)


def interpret(request):
    system = (request.get("instruction") or "") + ADDENDUM
    reply = call_model(system, interpreter_prompt(request), interpreter_schema(request))

    if reply is None:
        return clarify(
            "Sorry, could you say that again?",
            "the language service did not answer in time",
        )

    def with_say(out, from_reply):
        """Attach the spoken acknowledgement, where the model offered one.

        Beside the decision, as a sibling key, because that is the shape the
        engine parses: `{"action": 3, "say": "..."}`. Blank is nothing - a
        model that returns "" has not offered a sentence, and speaking an
        empty string is a pause for no reason.
        """
        text = (from_reply.get("say") or "").strip()
        if text:
            out["say"] = text
        return out

    kind = reply.get("reply_type")

    if kind == "answer":
        return {
            "answer": {
                "text": reply.get("answer_text") or "I do not know.",
                "drew_on": reply.get("drew_on") or [],
            }
        }

    if kind == "actions":
        chosen = reply.get("actions") or []
        if len(chosen) < 2:
            # One is the other arm's business, and none is nothing asked for.
            return clarify(
                "Which of those would you like?",
                "the reply named a sequence without naming two things to do",
            )
        return with_say({"actions": chosen}, reply)

    if kind == "action":
        if reply.get("action") is None:
            return clarify(
                "Which of those would you like?",
                "the reply named an action without saying which",
            )
        return with_say({"action": reply["action"]}, reply)

    if kind == "clarify":
        return clarify(
            reply.get("question") or "Could you say that again?",
            reply.get("because") or "the request was not specific enough",
        )

    targets = {}
    for t in reply.get("targets") or []:
        if t.get("mode") == "relative" and t.get("of"):
            spec = {"of": t["of"]}
            if t.get("scale") is not None:
                spec["scale"] = t["scale"]
            elif t.get("offset_value") is not None:
                spec["offset"] = {
                    "value": t["offset_value"],
                    "unit": t.get("offset_unit") or "none",
                }
            targets[t["variable"]] = spec
        elif t.get("value") is not None:
            targets[t["variable"]] = {
                "value": t["value"],
                "unit": t.get("unit") or "none",
            }

    if not targets and not reply.get("style"):
        # A style is enough on its own: the engine resolves the drink's recipe
        # and seeds the targets from it. Without either, there is nothing to
        # act on, and asking is better than appearing to obey and doing
        # nothing.
        return clarify(
            "What would you like?",
            "the request named neither a drink nor anything measurable to change",
        )

    body = {"kind": reply.get("kind") or "produce", "targets": targets}
    if reply.get("style"):
        body["style"] = reply["style"]
    if reply.get("material"):
        body["material"] = reply["material"]
    return with_say({"intent": body}, reply)


# ---------------------------------------------------------------- choosing


CHOOSER_SYSTEM = """\
You decide which of an appliance's declared actions to use, and at D2 what
value to send it.

You are given goals, each naming an outcome and how far it should move, and
candidates, each a tool that can move one outcome with a stated effect per unit
of its parameter. Answer with one step per goal, naming each by its candidate
index.

ONE TOOL PER GOAL, AND NEVER THE SAME TOOL TWICE. Two steps that move the
same outcome make the result impossible to attribute - if a brew runs twice,
nothing can say which of them put what in the cup - and the engine refuses the
whole plan rather than guess. Several candidates may list the same tool under
different outcomes; pick a different one for each goal, and if the only
candidate for a goal is a tool you have already used, leave that goal out.

THE ORDER YOU ANSWER IN IS THE ORDER THINGS HAPPEN. The goals arrive sorted by
name, which means nothing; you decide the sequence. Put them in the order that
makes sense for the drink - coffee into the cup before syrup is stirred into
it, milk before a shot is pulled through it. Where the order does not matter,
any order will do.

The value you give is the ABSOLUTE setting to send, not a change. Work it out
from the candidate's current value plus the change you intend:

    value = current + (goal delta / per_unit)

If a candidate has no current value, give the setting that reaches the goal
outright.

Choose between candidates serving one outcome by what else each one changes.
A candidate's also_affects says what moves with it: prefer the one that
disturbs least of what was not asked for. Its description says what the
mechanism is: prefer the one that fits the request.

Obey every rule in the constraints you were given. If a previous answer was
refused, the reason is in the request and you must fix exactly that.

Your value is clamped to the declared range, snapped to the declared
resolution, checked against the safety envelope and validated again by the
appliance. A wrong answer is a refused plan, not an unsafe action, so answer
with your best judgement rather than declining.
"""


def chooser_schema(request=None):
    """The shape of a valid answer.

    THE ENGINE'S OWN SCHEMA FIRST, where it sent one. It arrives as
    `request["schema"]` and it is narrower than anything written here: it
    limits `candidate` to an enum of the indices actually offered, so naming a
    tool that was not on the list becomes impossible to emit rather than
    emitted and refused, and it caps `steps` at one per goal.

    That cap is why this now takes the request. The engine computed it,
    serialised it and sent it; this file ignored it in favour of the
    hand-written fallback below, which bounds nothing. So a chooser kept
    answering with two steps for one goal - about four times in ten - the
    engine kept refusing the plan, and the fix was already on the wire.
    Twice before, a second definition of a shape in this file has quietly won
    over the engine's: an extra reply arm sat unreachable for the same reason,
    and a style's recipe bands were dropped on the way out.

    The fallback stays for a backend that sends no schema.
    """
    if request:
        sent = request.get("schema")
        if isinstance(sent, dict) and sent.get("properties"):
            return sent

    return {
        "type": "object",
        "additionalProperties": False,
        "required": ["steps"],
        "properties": {
            "steps": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["candidate", "value"],
                    "properties": {
                        "candidate": {"type": "integer"},
                        "value": {"type": ["number", "null"]},
                    },
                },
            }
        },
    }


def choose(request):
    level = request.get("level", "d2")

    lines = []

    # What was asked for, where a style named it. The one piece of information
    # genuinely added rather than rearranged: a goal alone does not say which
    # drink it belongs to, and three tools serve "move yield_mass" on this
    # machine. Asked for a doppio, a chooser given only the goal picked the
    # single-shot tool.
    if request.get("style"):
        lines.append("Asked for: " + request["style"])
        lines.append("")

    lines.append("Goals:")
    for goal in request.get("goals", []):
        variable = goal.get("variable") or "(unnamed)"
        lines.append("  " + variable)
        for label, key in (
            ("wanted change", "delta"),
            ("why", "cause"),
            ("confidence", "confidence"),
        ):
            if goal.get(key) is not None:
                lines.append("      {:<14} {}".format(label, goal[key]))

    lines.append("")
    lines.append("Candidates:")
    for candidate in request.get("candidates", []):
        lines.append(
            "  [{}] {}".format(
                candidate.get("index"), candidate.get("tool") or "(unnamed)"
            )
        )
        for label, key in (
            ("moves", "variable"),
            ("by setting", "parameter"),
            ("in", "unit"),
            ("now at", "current"),
            ("per unit", "per_unit"),
            ("confidence", "confidence"),
        ):
            if candidate.get(key) is not None:
                lines.append("      {:<14} {}".format(label, candidate[key]))
        also = candidate.get("also_affects") or []
        if also:
            lines.append("      {:<14} {}".format("also moves", ", ".join(also)))
        # On its own lines rather than inside the object, because this is the
        # part that says what the tool is FOR and it was the least readable
        # thing in the request.
        text = (candidate.get("description") or "").strip()
        if text:
            lines.append("      what it is:")
            for sentence in text.replace("\n", " ").split(". "):
                sentence = sentence.strip().rstrip(".")
                if sentence:
                    lines.append("        " + sentence + ".")

    constraints = request.get("constraints", [])
    if constraints:
        lines.append("")
        lines.append("Constraints you must satisfy:")
        for rule in constraints:
            lines.append("  - " + rule)

    remembered = request.get("remembered", [])
    if remembered:
        lines.append("")
        lines.append("What the machine remembers:")
        for item in remembered:
            lines.append("  " + str(item))

    if request.get("rejected"):
        lines.append("")
        lines.append("Your previous answer was refused because:")
        lines.append("  " + request["rejected"])

    lines.append(
        "\nLevel is {}. {}".format(
            level,
            "Give a value for every step."
            if level == "d2"
            else "Give the candidate index only; the engine computes the value.",
        )
    )

    reply = call_model(CHOOSER_SYSTEM, "\n".join(lines), chooser_schema(request))

    if reply is None:
        # No steps refuses the whole plan, which the selector treats as an
        # empty plan and declines. Correct: a guessed magnitude here becomes a
        # physical action, and the engine saying it could not decide is a
        # better outcome than acting on nothing.
        return {"steps": []}

    steps = []
    for step in reply.get("steps", []):
        out = {"candidate": step["candidate"]}
        if level == "d2" and step.get("value") is not None:
            out["value"] = step["value"]
        steps.append(out)
    return {"steps": steps}


# -------------------------------------------------------------------- main


EXPLAINER_SYSTEM = """\
You are the voice of a machine that has just done something, saying what it
did. You are given the machine's own record of the decision, which is already
true. Your job is to say it the way a person would.

  cause      what the machine diagnosed
  tool       the mechanism it used
  chose      who chose it: the engine, the model, or the person
  arguments  what was actually sent, with units. `asked_for` appears where the
             machine's own limits moved the value
  inputs     every figure that fed the decision, and which layer it came from
  note       what the domain rules wrote. This is the thing you are rephrasing

RULES, and the first one is the only one that matters:

  Never state a number that is not in the record, and never contradict `note`.
  You are rewording a fact, not adding to it. If the record does not say
  something, you do not know it.

  NEVER INVENT A REASON. This is the one that gets broken. Asked why, a model
  reaches for something plausible - "based on what it learned from past
  experience", "to suit your taste" - and every word of that is made up. If
  the record gives no reason beyond what was done, SAY ONLY WHAT WAS DONE.

  Say nothing about learning, memory, habits or previous drinks unless an entry
  in `inputs` actually says its figure came from something learned. If none
  does, nothing was learned and claiming otherwise is a lie about how the
  machine works.


  A GOAL AND A SETTING ARE NOT THE SAME NUMBER. This is the thing to get right.
  `note` says what outcome was wanted, in the outcome's units - "asked for
  yield_mass to be 110 g" is 110 grams of coffee IN THE CUP. `arguments` says
  what the mechanism was set to, in the mechanism's units - "volume 116 ml" is
  116 millilitres of water THROUGH the machine. The engine worked the second
  out from the first.

  So never compare them, never subtract one from the other, and never say one
  was "more than requested" because its figure is larger. 116 ml of water
  producing 110 g in the cup is the machine working correctly, not overshooting.
  If you want to mention both, say it the way it happened: "I put 116
  millilitres through to get 110 grams in the cup."

  NO IDENTIFIERS. "yield_mass" is a name in a file, not a word anyone says.
  Say "coffee in the cup".

  DO NOT NAME THE DRINK. `tool` is a mechanism identifier and the record does
  not say what was ordered: "adi.espresso.brew.start" is the brewing mechanism,
  and 116 ml through it is a long coffee rather than an espresso. Say what
  arrived - "116 millilitres", "a long one" - and leave the naming to whoever
  took the order.

  Say "I" and not "the machine". You are the machine.

  One or two sentences. Warm, plain, no jargon and no identifiers - say "a
  little more coffee", not "dose_mass".

  Where `asked_for` is present, say so plainly: a machine asked for more than
  it will do should admit it rather than quietly delivering less.

  Where `chose` says the person named it, do not claim you decided anything.

  Where a figure came from something learned, saying so is worth more than
  hiding it: "because you had it that way last time" is the honest reason.

Answer with JSON only: {"text": "..."}
"""


REFUSAL_SYSTEM = """\
You are the voice of a machine that has just REFUSED to do something, saying
why. You are given what the person asked for and the reason the machine gave.

The reason is already accurate. It is also written for an engineer. Your job is
to say the same thing to the person standing in front of the machine, and where
you can, what they could do about it.

  "a condition that tool requires does not hold"
      -> "I cannot, there is no milk in the hose."

RULES:

  Never invent a reason. If the reason given does not tell you WHICH condition
  failed, do not guess at one - say that something the machine needs is not
  ready, and that it can say which if asked.

  One sentence. Apologetic but not grovelling. No identifiers.

  NO WORDS FROM COMPUTING. Never "rate limit", "timeout", "invalid", "error",
  "request failed" - those make a kitchen refusal sound like a web page that is
  down, and they tell the person nothing they can act on. Say it the way a
  barista would:

    too soon after the last one -> "I have just run one of those; they are
                                    only meant to be done now and again."
    a condition is not met      -> "Something I need is not ready."
    a limit was reached         -> "That is more than I am set up to put in
                                    one cup."

  Do not suggest overriding a safety limit. A refusal from the appliance is the
  appliance doing its job.

Answer with JSON only: {"text": "..."}
"""


SAID_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["text"],
    "properties": {"text": {"type": "string"}},
}


def explain(request):
    """Say what happened, or why it did not.

    Both shapes answer `{"text": ...}` because both are one sentence spoken to
    a person. What differs is what they are given: a record, or a refusal.
    """
    if "refusal" in request:
        system = REFUSAL_SYSTEM
        body = request["refusal"]
    else:
        system = EXPLAINER_SYSTEM
        body = request.get("explain", {})

    reply = call_model(system, json.dumps(body, ensure_ascii=False), SAID_SCHEMA)
    text = (reply.get("text") or "").strip()

    # Nothing is better than a blank. The engine falls back to the record's own
    # words, which are flat and true.
    return {"text": text}


LEARNER_SYSTEM = """You decide whether a machine has learned something about the person using it.

You are given `moments` - what the machine has done recently, oldest first -
and `current`, the ranges already in force.

Propose a NARROWER range on a variable, or propose nothing.

WHAT YOU MAY PROPOSE. Only variables listed in `current`, and only ranges
inside the ones given. You cannot widen a range: a proposal outside what is
already in force will be refused and thrown away, so there is no point making
one.

WHEN TO PROPOSE. When the moments show the same preference asked for more than
once, in the same direction. "Stronger" twice is a preference beginning to
show; "stronger" once is a person wanting one particular coffee stronger.

HOW CAUTIOUS TO BE IS YOUR JUDGEMENT. There is no threshold to satisfy. But
weigh what being wrong costs: this machine may be used by a stranger who will
never return, and inventing a preference for them makes every later drink worse
for the next person. When the moments are few, or point in different
directions, or look like different people, propose NOTHING. That is the
ordinary answer and it is not a failure.

Narrow gently. Move the bound part of the way, not to the extreme - a person
who has twice asked for a stronger coffee has not asked for the strongest
coffee the machine can make.

`because` must say what you saw, in one sentence a person would accept: "you
asked for it stronger twice in a row". Never "based on your preferences", which
says nothing. It goes in a file the person can read, and it is the only account
they will have of why their coffee changed.

Answer with JSON only:
  {"constraints": [{"variable": "...", "min": 0, "max": 0, "unit": "..."}],
   "because": "..."}

Nothing learned is {"constraints": [], "because": ""}.
"""


LEARNED_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["constraints", "because"],
    "properties": {
        "constraints": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["variable", "min", "max", "unit"],
                "properties": {
                    "variable": {"type": "string"},
                    "min": {"type": "number"},
                    "max": {"type": "number"},
                    "unit": {"type": "string"},
                },
            },
        },
        "because": {"type": "string"},
    },
}


def learn(request):
    """Say whether a preference has emerged, or that none has."""
    reply = call_model(
        LEARNER_SYSTEM,
        json.dumps(request.get("learn", {}), ensure_ascii=False),
        LEARNED_SCHEMA,
    )
    if not reply:
        # A failure to decide is "nothing learned". The alternative is writing
        # a preference nobody proposed.
        return {"constraints": [], "because": ""}
    return {
        "constraints": reply.get("constraints") or [],
        "because": (reply.get("because") or "").strip(),
    }


def main():
    sys.stderr.write(
        "hosted.py: model={} timeout={}s sending={}\n".format(
            MODEL, REQUEST_TIMEOUT, REDACTION
        )
    )

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
            # One program, four seams, each told apart by the shape of the
            # message rather than by an argument. A selection carries
            # candidates; an explanation carries a record or a refusal; an
            # interpretation carries an utterance.
            if "candidates" in request:
                reply = choose(request)
            elif "learn" in request:
                reply = learn(request)
            elif "explain" in request or "refusal" in request:
                reply = explain(request)
            else:
                reply = interpret(request)
        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down with it. One that
            # answers badly leaves the machine usable and puts the reason in
            # the record.
            sys.stderr.write("hosted.py: {}\n".format(error))
            reply = (
                {"constraints": [], "because": ""}
                if "learn" in line
                else {"text": ""}
                if ("explain" in line or "refusal" in line)
                else {"steps": []}
                if "candidates" in line
                else clarify(
                    "Sorry, something went wrong. Could you say that again?",
                    "backend error: {}".format(error),
                )
            )

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
