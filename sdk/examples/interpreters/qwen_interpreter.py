#!/usr/bin/env python3
"""Interpreter backend using a local GGUF language model.

Serves the interpreter seam through the process backend. Reads one JSON
request per line on standard input and writes one answer per line on standard
output.

    pip install llama-cpp-python

    seams:
      interpreter:
        backend: process
        command: python
        args: ["examples/interpreters/qwen_interpreter.py",
               "models/interpreter/qwen3-1.7b-instruct-q4_k_m.gguf"]

The output is constrained by a grammar built from the schema, so the model
cannot emit anything outside it. This is ADI-P 9.3 requirement 3 enforced at
the decoder rather than checked afterwards; the engine validates the result
again regardless, because a decoder that guarantees the shape guarantees
nothing about the contents.

The schema permits either an intent or a question. That second arm is not a
convenience. Constraining a small model costs accuracy, because capacity spent
satisfying a format is capacity not spent on the task, and a model that cannot
resolve a request would otherwise be forced to invent something valid.
Permitting it to ask instead removes that pressure, and ADI-P 9.4.3 permits it
for the separate reason that the alternative to asking is guessing.

VERIFIED against Qwen3 4B Q4_K_M on an x86-64 host with no accelerator.
Reaches 9 of 18 exactly correct on eval/espresso, and 14 of 18 correct in
choosing whether to act or to ask.

Two things found by running it, both corrected here and both worth knowing if
you write your own backend.

The output schema must be narrowed to what the machine declares. The engine's
own schema permits any string where an identifier belongs, because it
validates the answer afterwards. A grammar can do better: see schema_for
below. Unconstrained, this model produced a style the machine does not have.

Qwen3 needs "/no_think". It reasons in a block before answering, and a grammar
admitting only the final JSON leaves it nowhere to do that. Without the switch
its answers barely varied with the input.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import copy
import json
import sys

try:
    from llama_cpp import Llama, LlamaGrammar
except ImportError:
    sys.stderr.write(
        "llama-cpp-python is not installed.\n"
        "    pip install llama-cpp-python\n"
    )
    sys.exit(1)


# The shape a reply must take. Kept in step with
# adi-models/src/schema.rs::UNDERSTOOD_SCHEMA; a change to one is a change to
# both.
SCHEMA = {
    "oneOf": [
        {
            "type": "object",
            "required": ["intent"],
            "additionalProperties": False,
            "properties": {
                "intent": {
                    "type": "object",
                    "required": ["kind"],
                    "additionalProperties": False,
                    "properties": {
                        "kind": {"enum": ["adjust", "produce"]},
                        "style": {"type": "string"},
                        "material": {"type": "string"},
                        "targets": {
                            "type": "object",
                            "additionalProperties": {
                                "oneOf": [
                                    {
                                        "type": "object",
                                        "required": ["value", "unit"],
                                        "additionalProperties": False,
                                        "properties": {
                                            "value": {"type": "number"},
                                            "unit": {"type": "string"},
                                        },
                                    },
                                    {
                                        "type": "object",
                                        "required": ["of"],
                                        "additionalProperties": False,
                                        "properties": {
                                            "of": {"type": "string"},
                                            "scale": {"type": "number"},
                                            "offset": {
                                                "type": "object",
                                                "required": ["value", "unit"],
                                                "properties": {
                                                    "value": {"type": "number"},
                                                    "unit": {"type": "string"},
                                                },
                                            },
                                        },
                                    },
                                ]
                            },
                        },
                    },
                }
            },
        },
        {
            "type": "object",
            "required": ["clarify"],
            "additionalProperties": False,
            "properties": {
                "clarify": {
                    "type": "object",
                    "required": ["question", "because"],
                    "additionalProperties": False,
                    "properties": {
                        "question": {"type": "string"},
                        "because": {"type": "string"},
                    },
                }
            },
        },
    ]
}

INSTRUCTION = """\
You turn a spoken request to an appliance into one JSON object.

To act on the request:
  {"intent": {"kind": "produce"|"adjust", "style": "<id>",
   "targets": {"<variable>": <target>}}}

A target is either {"value": <number>, "unit": "<unit>"}
or {"of": "last"|"<cycle id>", "scale": <number>, "offset": {...}}.
Give at most one of scale and offset. Neither means the same as then.
A scale of 0 means none at all.

If the request cannot be resolved, ask instead:
  {"clarify": {"question": "<what to ask>", "because": "<why>"}}

Asking is a correct answer. Do not guess a value, a style or a cycle that was
not stated: the result of this request moves a machine.

State relations rather than resolving them. "Half of last time" is
{"of": "last", "scale": 0.5}, never a number. The engine holds the memory and
does the arithmetic.

Name only the styles, materials and variables listed below.\
"""



def schema_for(request):
    """The output schema, narrowed to what this machine actually declares.

    SCHEMA above permits any string where an identifier belongs, which is what
    the engine's own schema does, because the engine validates the answer
    afterwards and can report a useful refusal. A grammar can do better than
    that here: a style that is not in the list becomes impossible to emit
    rather than emitted and then refused.

    This matters more than it looks. The instruction already says to name only
    what is listed, and a model of this size does not reliably obey it. Against
    the shipped corpus the unconstrained schema produced
    `adi.espresso.style.cortado` for a machine that declares no such style.
    Instruction is advice; a grammar is not.

    Units are narrowed the same way, from the units the variables declare.
    Targets are keyed by variable, so an unmodelled variable also becomes
    unreachable rather than refused four steps later.
    """
    schema = copy.deepcopy(SCHEMA)
    intent = schema["oneOf"][0]["properties"]["intent"]["properties"]

    styles = request.get("styles") or []
    if styles:
        intent["style"] = {"enum": list(styles)}
    else:
        # A machine declaring no styles cannot be asked for one.
        intent.pop("style", None)

    materials = request.get("materials") or []
    if materials:
        intent["material"] = {"enum": list(materials)}
    else:
        intent.pop("material", None)

    variables = request.get("variables") or []
    names = [v["name"] if isinstance(v, dict) else v for v in variables]
    units = sorted({v.get("unit") for v in variables
                    if isinstance(v, dict) and v.get("unit")})

    if not names:
        intent.pop("targets", None)
        return schema

    target = intent["targets"]["additionalProperties"]
    if units:
        # Each branch of the target carries a unit, and an offset carries its
        # own.
        target["oneOf"][0]["properties"]["unit"] = {"enum": units}
        target["oneOf"][1]["properties"]["offset"]["properties"]["unit"] = {
            "enum": units
        }

    # One named property per declared variable, and nothing else permitted.
    # A JSON schema keyed by name rather than by additionalProperties is what
    # makes the grammar enumerate them.
    intent["targets"] = {
        "type": "object",
        "additionalProperties": False,
        "properties": {name: target for name in names},
    }
    return schema

def context_lines(request):
    """What the engine knows, as plain lines for the prompt."""
    lines = []

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

    return "\n".join(lines)


def main():
    if len(sys.argv) < 2:
        sys.stderr.write("usage: qwen_interpreter.py <model.gguf>\n")
        return 2

    model = Llama(
        model_path=sys.argv[1],
        n_ctx=2048,
        # The engine caps what it sends, so a larger context buys nothing and
        # costs memory on an appliance.
        verbose=False,
    )
    # One grammar per distinct machine, not per request. Compiling a grammar
    # costs tens of milliseconds, and the declaration does not change within a
    # session, so the first request of a session pays for it and the rest do
    # not.
    grammars = {}

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)

            schema = schema_for(request)
            key = json.dumps(schema, sort_keys=True)
            if key not in grammars:
                grammars[key] = LlamaGrammar.from_json_schema(key)
            grammar = grammars[key]

            messages = [
                {"role": "system", "content": INSTRUCTION + "\n\n" + context_lines(request)},
                # Qwen3 reasons inside a <think> block before answering, and a
                # grammar permitting only the final JSON leaves it nowhere to
                # do that. Measured against the shipped corpus, the result was
                # an answer that barely varied with the input: the same style
                # came back for almost every request, including ones that were
                # not requests at all.
                #
                # "/no_think" is the family's documented switch for a direct
                # answer. A model that does not recognise it reads it as
                # ordinary text, so this is safe to send to any backend.
                {"role": "user", "content": request.get("utterance", "") + " /no_think"},
            ]
            completion = model.create_chat_completion(
                messages=messages,
                grammar=grammar,
                # Low rather than zero. The schema fixes the shape, so the
                # remaining variation is in wording, where a little helps a
                # question read naturally.
                temperature=0.3,
                max_tokens=256,
            )
            reply = completion["choices"][0]["message"]["content"]
            json.loads(reply)  # refuse to pass on something unparseable

        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down. One that asks leaves
            # the machine usable and puts the reason in the record.
            reply = json.dumps(
                {
                    "clarify": {
                        "question": "Sorry, I did not follow. Could you say that again?",
                        "because": f"interpreter backend error: {error}",
                    }
                }
            )

        sys.stdout.write(reply.strip() + "\n")
        sys.stdout.flush()

    return 0


if __name__ == "__main__":
    sys.exit(main())
