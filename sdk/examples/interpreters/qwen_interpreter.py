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

The schema and the instruction are not written here. The engine sends both
with every request: it owns the contract, and it narrows the schema to the
styles, materials, variables and units this machine declares. This file reads
them off the request and compiles the grammar from what it was given.

That division is the point. This file used to carry its own copy of the
schema, and the copy had two of the contract's three arms. An intent and a
question were permitted; an answer was not. The engine has permitted an answer
throughout, ADI-P 9.4.5 describes it, and the engine was already sending the
machine identity and the manufacturer's notes an answer is composed from. The
grammar built from the local copy made an answer impossible to emit, so the
machine met every question with a question. Nothing detected it, because the
engine never stated the contract on the wire for the copy to be checked
against. A backend that holds no copy cannot drift from the original.

The schema permits an intent, a question or an answer. The second arm is not a
convenience. Constraining a small model costs accuracy, because capacity spent
satisfying a format is capacity not spent on the task, and a model that cannot
resolve a request would otherwise be forced to invent something valid.
Permitting it to ask instead removes that pressure, and ADI-P 9.4.3 permits it
for the separate reason that the alternative to asking is guessing.

VERIFIED against Qwen3 4B Q4_K_M on an x86-64 host with no accelerator.
Reaches 9 of 18 exactly correct on eval/espresso, and 14 of 18 correct in
choosing whether to act or to ask.

One thing found by running it, worth knowing if you write your own backend.

Qwen3 needs "/no_think". It reasons in a block before answering, and a grammar
admitting only the final JSON leaves it nowhere to do that. Without the switch
its answers barely varied with the input.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import json
import sys

# One copy of the prompt rendering, shared with the other backends here.
from adi_prompt import plan_for

# Imported here rather than lazily so that a missing dependency is reported at
# startup, and not fatally, because --explain needs neither the package nor a
# model. See main.
try:
    from llama_cpp import Llama, LlamaGrammar
except ImportError:
    Llama = None
    LlamaGrammar = None


def explain(stream):
    """Report how each request would be served, without loading a model.

    `--explain` exists so that the routing can be tested where the weights
    cannot be: CI has no 2.5 GB model and never will. It answers the question
    this file once got wrong, which is whether a selection request is
    recognised as one at all, and it needs no llama-cpp and no GGUF.

    Both halves of what the model is sent are reported: the system message,
    which carries the context and the engine's instruction, and the user
    message. Reporting only the second hid a prompt that was dropping the
    machine identity, the manufacturer's notes and the decision history, since
    on the spoken path those all go in the first.

    It says nothing about the quality of the model's judgement. That needs
    weights and is measured by `adi models bench`.
    """
    for line in stream:
        line = line.strip()
        if not line:
            continue

        seam, schema, instruction, prompt = plan_for(json.loads(line))
        sys.stdout.write(
            json.dumps(
                {
                    "seam": seam,
                    "schema": schema,
                    "instruction": instruction,
                    "prompt": prompt,
                }
            )
            + "\n"
        )
        sys.stdout.flush()

    return 0


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--explain":
        return explain(sys.stdin)

    if len(sys.argv) < 2:
        sys.stderr.write(
            "usage: qwen_interpreter.py <model.gguf>\n"
            "       qwen_interpreter.py --explain   (routing only, no model)\n"
        )
        return 2

    if Llama is None:
        sys.stderr.write(
            "llama-cpp-python is not installed.\n"
            "    pip install llama-cpp-python\n"
        )
        return 1

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

        seam = "interpreter"
        try:
            request = json.loads(line)

            # One program, two seams. A selection request carries candidates
            # and a spoken one does not, which is the test reference.py makes.
            seam, schema, instruction, prompt = plan_for(request)

            key = json.dumps(schema, sort_keys=True)
            if key not in grammars:
                grammars[key] = LlamaGrammar.from_json_schema(key)
            grammar = grammars[key]

            messages = [
                {"role": "system", "content": instruction},
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
                {"role": "user", "content": prompt + " /no_think"},
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
            # A backend that crashes takes the seam down. One that answers
            # leaves the machine usable and puts the reason in the record.
            #
            # What a usable answer is differs by seam. A question back is the
            # right reply to a person and meaningless to the engine when it
            # asked which mechanism to move: there, refusing the plan is the
            # safe answer, and no steps is how a plan is refused.
            if seam == "chooser":
                sys.stderr.write("chooser backend error: {}\n".format(error))
                reply = json.dumps({"steps": []})
            else:
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
