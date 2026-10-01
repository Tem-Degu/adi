#!/usr/bin/env python3
"""A model backend that is not a model.

Serves two seams through the process backend: the interpreter, which turns a
request into structured intent, and the chooser, which picks which declared
action serves a goal and at D2 what value to send. The engine writes
one JSON request per line on this program's standard input and reads one JSON
answer per line from its standard output. Nothing else is required of it: no
dependency on the SDK, no particular language, no particular runtime.

It exists to demonstrate the boundary rather than to interpret well. The rules
below are deliberately simple, and a real backend would put a language model
here. What a real backend would not change is the contract:

  in   {"utterance": "...", "styles": [...], "materials": [...],
        "variables": [...], "remembered": [...], "history": [...]}

  out  {"intent": {"kind": "produce"|"adjust", "style": "<id>",
                   "targets": {"<variable>": <target>}}}
  or   {"clarify": {"question": "<what to ask>", "because": "<why>"}}

A target is {"value": <number>, "unit": "<unit>"} or
{"of": "last"|"<cycle id>", "scale": <number>, "offset": {...}}.

Two properties matter more than accuracy, and both are visible below.

Asking is an answer. ADI-P 9.4.3 permits a backend to return a question
instead of an intent, and this one does so whenever the request names a
quantity with no variable, refers to a cycle that does not exist, or names a
style the machine does not declare. The alternative to asking is guessing, and
a guess on this path becomes a physical action.

Relations are stated, not resolved. "Half of last time" is returned as
{"of": "last", "scale": 0.5}, not as a number. The engine holds the memory and
does the arithmetic. A backend that emitted "4 g" would have to be trusted to
remember correctly and to divide correctly, and would be wrong silently when it
did not.

Run it through the SDK rather than directly:

    seams:
      interpreter:
        backend: process
        command: python3
        args: ["examples/interpreters/reference.py"]

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import json
import re
import sys

# Words that scale a quantity, and what they scale it by. A real backend would
# infer these; this one is told.
SCALES = {
    "half": 0.5,
    "double": 2.0,
    "twice": 2.0,
    "no": 0.0,
    "none": 0.0,
    "without": 0.0,
}

# Words naming a direction, and the sign they imply. The magnitude is left to
# the engine, which knows the step size the knowledge base declares.
DIRECTIONS = {
    "more": 1,
    "less": -1,
    "hotter": 1,
    "cooler": -1,
    "colder": -1,
    "longer": 1,
    "shorter": -1,
    "stronger": 1,
    "weaker": -1,
}


def answer(question, because):
    """Ask rather than guess."""
    return {"clarify": {"question": question, "because": because}}


def intent(kind, style=None, targets=None):
    body = {"kind": kind}
    if style:
        body["style"] = style
    if targets:
        body["targets"] = targets
    return {"intent": body}


def find_style(words, styles):
    """Match a spoken word against a declared style identifier.

    Only identifiers the machine actually declared are considered. A backend
    that invented one would produce an intent the engine refuses, which is
    safe but unhelpful, and the refusal would arrive as a fault rather than as
    a question.
    """
    for style in styles:
        leaf = style.rsplit(".", 1)[-1].lower()
        if leaf in words:
            return style
    return None


def find_variable(words, variables):
    """Match a spoken word against a modelled variable."""
    for variable in variables:
        parts = set(re.split(r"[._]", variable.lower()))
        if parts & words:
            return variable
    return None


def interpret(request):
    utterance = request.get("utterance", "")
    words = set(re.findall(r"[a-z']+", utterance.lower()))

    styles = request.get("styles", [])
    variables = request.get("variables", [])
    remembered = request.get("remembered", [])

    style = find_style(words, styles)
    variable = find_variable(words, variables)

    scale_word = next((w for w in words if w in SCALES), None)
    direction_word = next((w for w in words if w in DIRECTIONS), None)
    refers_back = bool(words & {"last", "before", "previous", "again", "same"})

    # A relation with nothing to relate to. The machine has produced nothing,
    # so there is no "last time" to be half of.
    if refers_back and not remembered:
        return answer(
            "I have not made anything yet. What would you like?",
            "the request refers to a previous cycle and none exists",
        )

    # A quantity with no variable. "Make it half" does not say half of what,
    # and the candidates are not interchangeable: half the sugar and half the
    # volume are different drinks.
    if scale_word and not variable:
        return answer(
            "Half of what, the strength or the amount?",
            "a scaling word with no variable named",
        )

    # A relation to a variable: state it, do not resolve it.
    if variable and (scale_word or direction_word or refers_back):
        target = {"of": "last"}
        if scale_word:
            target["scale"] = SCALES[scale_word]
        kind = "produce" if style or "coffee" in words or "make" in words else "adjust"
        return intent(kind, style, {variable: target})

    # A named style, with nothing else asked of it.
    if style:
        return intent("produce", style)

    # A request that mentions a drink this machine does not make. Saying so is
    # more useful than producing something else.
    if words & {"make", "get", "like", "want", "please"}:
        return answer(
            "I can make: " + ", ".join(s.rsplit(".", 1)[-1] for s in styles) + ". Which?",
            "no declared style matched the request",
        )

    return answer(
        "Sorry, I did not follow. What would you like me to change?",
        "the request matched no rule in this backend",
    )


# --------------------------------------------------------------- choosing
#
# ADI-P 3.3 grades an engine by who answers this. At D1 the model picks the
# tool and the engine computes the value; at D2 it proposes both.
#
# The reply names a candidate by its index in the list the engine supplied.
# That list was derived from the machine's own declaration, so a chooser
# cannot name a tool the machine does not have. At D2 the proposed value is
# then clamped to the declared range, snapped to the declared resolution,
# checked against the stateful envelope with the rest of the plan, and
# validated again by the appliance.


def choose(request):
    """Pick a candidate per goal, and at D2 a value.

    The rule below is the same one the engine applies at D0: take the most
    confident candidate that serves the goal. A real model would weigh the
    history, the material and what the person said. What matters here is the
    shape of the answer, not the quality of the judgement.
    """
    candidates = request.get("candidates", [])
    steps = []

    for goal in request.get("goals", []):
        variable = goal.get("variable")

        serving = [c for c in candidates if c.get("variable") == variable]
        if not serving:
            # Every goal must be served or the whole plan refused. Returning
            # fewer steps than goals is how a plan looks complete and is not,
            # and the selector refuses on the count.
            return {"steps": []}

        best = max(serving, key=lambda c: c.get("confidence", 0.0))
        step = {"candidate": best["index"]}

        if request.get("level") == "d2":
            # At D2 the value sent is the absolute setting, not a change.
            #
            # per_unit is the change in the variable per unit of the
            # parameter, and is signed, so dividing the change the profile
            # wants by it gives the change in the parameter. Adding that to
            # the present setting gives the value to send. A model proposing
            # a delta here would have it read as an absolute and clamped to
            # the end of the range.
            delta = goal.get("delta") or 0.0
            per_unit = best.get("per_unit") or 0.0
            current = best.get("current")

            if per_unit == 0.0 or current is None:
                # Nothing to size the correction from. Returning no steps
                # refuses the plan, which is correct: a guess here becomes a
                # physical action.
                return {"steps": []}

            step["value"] = round(current + delta / per_unit, 3)

        steps.append(step)

    return {"steps": steps}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
            # One program, two seams. A selection request carries candidates;
            # an interpretation request carries an utterance.
            reply = choose(request) if "candidates" in request else interpret(request)
        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down. One that asks leaves
            # the machine usable, and the reason reaches the record.
            # A chooser that fails returns no steps, which the selector
            # refuses as an empty plan. An interpreter that fails asks.
            if "candidates" in line:
                sys.stderr.write(f"chooser error: {error}\n")
                reply = {"steps": []}
            else:
                reply = answer(
                    "Sorry, something went wrong. Could you say that again?",
                    f"backend error: {error}",
                )

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
