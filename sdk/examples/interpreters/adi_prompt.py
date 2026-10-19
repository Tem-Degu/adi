#!/usr/bin/env python3
"""How a request from the engine becomes a prompt.

Shared by every backend here, and the reason there is one copy of it. The
engine states the contract and sends it with each request: the schema a reply
must satisfy, narrowed to what the machine declares, and the instruction that
goes with it. A backend's remaining job is to render the context as text and
hand both to whatever model it drives.

That division is what this module exists to hold. The local backend used to
carry its own schema, the copy was missing the arm that permits an answer, and
a machine running real weights met every question with a question. A second
backend written the same way would have been a second copy and a second drift.

Nothing here imports a model library, so it is importable wherever Python is.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import json


def context_lines(request):
    """What the engine knows, as plain lines for the prompt.

    Every field the engine sends is rendered. A field the engine troubles to
    send and the prompt discards is worse than one that was never sent, because
    the omission is invisible from both ends: the engine's record shows it was
    supplied and the model never saw it. This prompt dropped three of them, and
    two were the whole basis on which a question gets answered.
    """
    lines = []

    # What the machine is. ADIP 9.4.5 names this first among what an engine
    # should supply for a question, and "what model are you" is answerable
    # from nothing else.
    if request.get("machine"):
        lines.append("this machine: " + request["machine"])

    if request.get("styles"):
        lines.append("styles: " + ", ".join(request["styles"]))
    if request.get("materials"):
        lines.append("materials: " + ", ".join(request["materials"]))
    if request.get("variables"):
        # Name, unit and meaning, one per line. The meaning is what lets a
        # request phrased in the domain's own words reach the right variable:
        # "less bitter" does not contain the word "time".
        lines.append("variables:")
        for v in request["variables"]:
            if v.get("unit") and v.get("meaning"):
                lines.append("  {} ({}): {}".format(v["name"], v["unit"], v["meaning"]))
            else:
                lines.append("  " + v["name"])

    # The manufacturer's notes. ADIP 9.4.5: an answer should name what it drew
    # on, which is why each note is given with its identifier.
    if request.get("notes"):
        lines.append("notes, each with the id to cite in drew_on:")
        for note in request["notes"]:
            lines.append("  [{}] {}".format(note.get("id", "?"), note.get("text", "")))

    for cycle in request.get("remembered", []):
        parts = ["cycle " + cycle.get("cycle_id", "?")]
        if cycle.get("style"):
            parts.append("style=" + cycle["style"])
        parts.extend(cycle.get("settings", []))
        if cycle.get("feedback"):
            parts.append("said=" + ",".join(cycle["feedback"]))
        lines.append(" ".join(parts))

    if not request.get("remembered"):
        lines.append("nothing has been produced yet")

    # ADIP 3.3.1: the decisions the engine has already taken and what they were
    # for. A model asked about an earlier correction has nothing else to go on.
    if request.get("history"):
        lines.append("decisions already taken, newest first:")
        for decision in request["history"]:
            line = "  cycle {} corrected {}".format(
                decision.get("cycle_id", "?"), decision.get("cause", "?")
            )
            if decision.get("tool"):
                line += " using " + decision["tool"]
            lines.append(line)

    return "\n".join(lines)


def choice_lines(request):
    """The goals and candidates, as text the model reads.

    Every field the engine sends is rendered. A field the engine troubles to
    send and the prompt discards is worse than one that was never sent, because
    the omission is invisible from both ends: the engine's record shows it was
    supplied and the model never saw it.
    """
    out = []

    for goal in request.get("goals", []):
        # `cause` is the diagnosis this correction answers. Two goals on one
        # variable can want opposite things for different reasons, and the
        # delta alone does not say which.
        line = "goal: move {} by {}".format(goal.get("variable"), goal.get("delta"))
        if goal.get("cause"):
            line += ", to correct {}".format(goal["cause"])
        if goal.get("confidence") is not None:
            line += " (confidence {})".format(goal["confidence"])
        out.append(line)

    out.append("")
    out.append("candidates:")
    for c in request.get("candidates", []):
        # The parameter is what the value sets, and its unit is not the
        # variable's: a grind position is in steps and moves shot time in
        # seconds. A model proposing an absolute setting has to know which of
        # the two it is writing.
        unit = c.get("unit") or "unit"
        parts = [
            "  [{}] {}".format(c.get("index"), c.get("tool")),
            "sets {} in {}".format(c.get("parameter"), unit),
            "moves {} by {} per {}".format(c.get("variable"), c.get("per_unit"), unit),
            "current {}".format(c.get("current")),
        ]
        if c.get("confidence") is not None:
            parts.append("confidence {}".format(c["confidence"]))
        if c.get("also_affects"):
            parts.append("also affects {}".format(", ".join(c["also_affects"])))
        if c.get("description"):
            parts.append(c["description"])
        out.append(", ".join(parts))

    # ADI-P 3.3.1 requires a selector at D1 or D2 be given the history the
    # engine holds, on the stated grounds that a model seeing only what the
    # engine already used to rank the list can disagree with the ranking but
    # never on evidence. What the engine sends today is the identifiers of the
    # last few cycles and not the decisions or their outcomes, so this is the
    # history as far as it goes.
    if request.get("remembered"):
        out.append("")
        out.append("recent cycles, newest first: " + ", ".join(request["remembered"]))

    for key in ("constraints", "rejected"):
        if request.get(key):
            out.append("")
            out.append("{}: {}".format(key, json.dumps(request[key])))

    out.append("")
    out.append("level: {}".format(request.get("level", "d2")))
    return "\n".join(out)


def plan_for(request):
    """How this request is served: its schema, its instruction and its prompt.

    The schema and the instruction are taken from the request. The engine holds
    the contract, narrows the schema to what this machine declares, and sends
    both with every request, so this file does not carry a copy of either.

    That is not tidiness. This backend used to carry its own copy of the schema,
    and the copy had two of the contract's three arms: an intent and a question,
    but no answer. The engine has permitted an answer throughout, ADIP 9.4.5
    describes it, and the engine was already sending the machine identity and
    the manufacturer's notes needed to compose one. The grammar built from the
    local copy made answering impossible to emit, so the machine met every
    question with a question. Nothing detected it, because the engine never
    stated the contract on the wire for the copy to be compared against.

    Separated from main so that it can be exercised without loading a model.
    A selection request carries candidates and a spoken one does not, which is
    the same test reference.py makes.
    """
    schema = request.get("schema")
    instruction = request.get("instruction")
    if schema is None or instruction is None:
        # Refused rather than guessed at. A backend that invents a contract
        # when none arrived is how the copies drifted in the first place.
        missing = "schema" if schema is None else "instruction"
        raise ValueError(
            "the request carries no {}. The engine sends both with every "
            "request; a hand-written request has to supply them too.".format(missing)
        )

    if "candidates" in request:
        return "chooser", schema, instruction, choice_lines(request)

    # The context goes above the instruction, because the engine's instruction
    # refers to what it was "given above": the styles, the variables and their
    # meanings, the notes, and what this machine is.
    #
    # The prompt is returned bare. `main` appends the "/no_think" switch once,
    # for whichever seam is being served, so it is written in one place and
    # cannot be sent twice.
    system = context_lines(request) + "\n\n" + instruction
    return "interpreter", schema, system, request.get("utterance", "")
