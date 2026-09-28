#!/usr/bin/env python3
"""A model backend that is not a model.

This serves the interpreter seam through the process backend. The engine writes
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


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
            reply = interpret(request)
        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down. One that asks leaves
            # the machine usable, and the reason reaches the record.
            reply = answer(
                "Sorry, something went wrong. Could you say that again?",
                f"backend error: {error}",
            )

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
