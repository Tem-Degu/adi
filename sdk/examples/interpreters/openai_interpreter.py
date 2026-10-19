#!/usr/bin/env python3
"""Interpreter and chooser backend using a hosted model.

Serves both seams through the process backend, like the local one, and reads
the contract off the request as the local one does. The whole of this file is
transport: the engine states what a reply must satisfy and sends it, so a
backend on a different provider is a different way of posting a prompt and
nothing more.

    export OPENAI_API_KEY=...
    python openai_interpreter.py gpt-5.5

    seams:
      interpreter:
        backend: process
        command: python3
        args: ["examples/interpreters/openai_interpreter.py", "gpt-5.5"]

The key is read from the environment and never from an argument, because an
argument is visible to anyone who can list processes on the machine.

No dependency beyond the standard library. The protocol is one JSON object per
line in each direction, and HTTPS is in urllib, so a hosted seam needs nothing
installed. ADI-P 9.4.2 counts this as the same seam wherever the weights are.

**What this configuration does not do.** The local backend compiles the schema
into a grammar, so a reply outside it cannot be generated. ADI-P 9.3
requirement 3 is enforced at the decoder there. Here the schema is sent to the
provider and applied by them, and where they decline it the schema is stated in
the prompt instead. The engine validates every reply regardless, which is the
check that actually holds the boundary, so a hosted seam is sound but it is not
evidence that decoder-level enforcement works. Measure the local path for that.

A hosted seam also sends the utterance, the machine's declaration and the
manufacturer's notes to a third party, and it does not work when the network is
down. Both are product decisions rather than protocol ones. ADI-P 9.4.2 permits
either.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import json
import os
import sys
import urllib.request

# One copy of the prompt rendering, shared with the other backends here.
from adi_prompt import plan_for

ENDPOINT = "https://api.openai.com/v1/chat/completions"
TIMEOUT_SECONDS = 60


def post(payload, key):
    """One request to the provider, with the reply parsed."""
    body = json.dumps(payload).encode()
    request = urllib.request.Request(
        ENDPOINT,
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": "Bearer " + key,
        },
    )
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
        return json.loads(response.read().decode())


def provider_can_enforce(schema):
    """Whether this provider will apply this schema itself.

    It requires an object at the top level and no choice between alternatives
    there. The selection schema is a plain object and qualifies. The output
    schema offers an intent, a question or an answer as a `oneOf`, so it does
    not, and asking anyway spends a round trip to be told so on every single
    utterance.

    The alternative would be to flatten the engine's schema into one object
    with three optional members, which the provider would accept. That is not
    done, because "exactly one of these three" is the contract, a flattened
    object permits two at once or none, and a guarantee the local decoder
    enforces should not be given up to suit one provider's dialect.
    """
    if schema.get("type") != "object":
        return False
    return not any(key in schema for key in ("oneOf", "anyOf", "allOf", "not"))


def ask(model, key, schema, instruction, prompt):
    """Put the request to the model and return the text it replied with.

    Where the provider will apply the schema, it does. Where it will not, the
    schema is stated in the prompt and the reply is required to be JSON, which
    is weaker: the engine refuses an invalid reply on the way back either way,
    so what changes is how often that happens and not whether an invalid reply
    could be acted on.
    """
    if provider_can_enforce(schema):
        return post(
            {
                "model": model,
                "messages": [
                    {"role": "system", "content": instruction},
                    {"role": "user", "content": prompt},
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "understood",
                        "schema": schema,
                        "strict": False,
                    },
                },
            },
            key,
        )["choices"][0]["message"]["content"]

    spelled_out = (
        instruction
        + "\n\nYour reply must satisfy this JSON Schema exactly, and must match"
        + " exactly one of its alternatives:\n"
        + json.dumps(schema)
    )
    return post(
        {
            "model": model,
            "messages": [
                {"role": "system", "content": spelled_out},
                {"role": "user", "content": prompt},
            ],
            "response_format": {"type": "json_object"},
        },
        key,
    )["choices"][0]["message"]["content"]


def main():
    if len(sys.argv) < 2:
        sys.stderr.write(
            "usage: openai_interpreter.py <model>\n"
            "       OPENAI_API_KEY must be set in the environment\n"
        )
        return 2

    model = sys.argv[1]
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        sys.stderr.write(
            "OPENAI_API_KEY is not set. It is read from the environment rather\n"
            "than from an argument, because an argument is visible to anyone\n"
            "who can list processes.\n"
        )
        return 1

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        seam = "interpreter"
        try:
            request = json.loads(line)
            seam, schema, instruction, prompt = plan_for(request)
            reply = ask(model, key, schema, instruction, prompt)
            json.loads(reply)  # refuse to pass on something unparseable

        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down. One that answers
            # leaves the machine usable and puts the reason in the record.
            #
            # What a usable answer is differs by seam. A question back is the
            # right reply to a person and meaningless to the engine when it
            # asked which mechanism to move: there, refusing the plan is the
            # safe answer, and no steps is how a plan is refused.
            sys.stderr.write("{} backend error: {}\n".format(seam, error))
            if seam == "chooser":
                reply = json.dumps({"steps": []})
            else:
                reply = json.dumps(
                    {
                        "clarify": {
                            "question": "Sorry, I did not follow. Could you say that again?",
                            "because": "interpreter backend error: {}".format(error),
                        }
                    }
                )

        sys.stdout.write(reply.strip() + "\n")
        sys.stdout.flush()

    return 0


if __name__ == "__main__":
    sys.exit(main())
