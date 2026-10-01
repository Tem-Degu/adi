"""A chooser that proposes a value the mechanism cannot reach.

A test fixture, not an example to copy. It stands in for a model that has
misjudged a magnitude, which is the case the engine has to survive: the value is
clamped to the declared range, the limit is sent, and the record says the
correction was partial so the person can be told.

Serves the interpreter seam by asking, since what is being tested is selection.
"""
import sys
import json

for line in sys.stdin:
    request = json.loads(line)

    if "candidates" in request:
        # Far beyond any plausible range, so the test does not depend on the
        # particular limits the example declares.
        print(json.dumps({"steps": [{"candidate": 0, "value": 999.0}]}))
    else:
        print(json.dumps({
            "clarify": {
                "question": "What would you like?",
                "because": "this fixture only exercises selection",
            }
        }))

    sys.stdout.flush()
