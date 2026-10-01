"""Returns an intent the machine cannot serve.

A test fixture. extraction_yield is a modelled variable that no tool in the
example declaration moves, so the engine has no candidate and must refuse. What
is being tested is that the refusal reaches the person in words and the session
survives it.
"""
import sys
import json

for line in sys.stdin:
    request = json.loads(line)
    if "candidates" in request:
        print(json.dumps({"steps": [{"candidate": 0, "value": 20}]}))
    else:
        print(json.dumps({
            "intent": {
                "kind": "adjust",
                "targets": {"extraction_yield": {"value": 22.0, "unit": "pct"}},
            }
        }))
    sys.stdout.flush()
