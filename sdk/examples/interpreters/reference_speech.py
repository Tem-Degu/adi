#!/usr/bin/env python3
"""Reference transcriber and speaker, as external programs.

Two seams, one file, selected by the first argument:

    python3 reference_speech.py transcribe
    python3 reference_speech.py speak

Neither contains a model. They implement the protocol so that an integrator
can see the exact message shapes, test their own wiring before obtaining
weights, and have something to replace one line at a time.

The transcriber returns a fixed phrase with a confidence derived from the
signal level, which is enough to exercise the path that matters: ADI-P 9.4.2
treats a transcript below the configured threshold as not heard, and the
machine then asks rather than acting. Point a real model at the same interface
and nothing above it changes.

The speaker returns a tone rather than speech. It is the correct length,
sample rate and format, so everything downstream can be tested.

Protocol, one JSON object per line in each direction:

    transcribe  -> {"audio": "<base64>", "sample_rate": 16000}
                <- {"text": "...", "confidence": 0.0 to 1.0}

    speak       -> {"text": "..."}
                <- {"audio": "<base64>", "sample_rate": 22050}

Audio is base64 of 16-bit signed, little endian, single channel samples.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import base64
import json
import math
import struct
import sys

TRANSCRIBER_RATE = 16000
SPEAKER_RATE = 22050


def decode(encoded):
    """Base64 to samples."""
    raw = base64.b64decode(encoded)
    if len(raw) % 2:
        raise ValueError("odd byte count: not 16-bit samples")
    return list(struct.unpack(f"<{len(raw) // 2}h", raw))


def encode(samples):
    """Samples to base64."""
    return base64.b64encode(struct.pack(f"<{len(samples)}h", *samples)).decode()


def transcribe(request):
    """Stand in for a speech model.

    The confidence is derived from the signal level so that the threshold
    behaviour can be tested: near-silence produces a low confidence, and the
    machine should then ask rather than act on what it thinks it heard.
    """
    samples = decode(request["audio"])

    if not samples:
        return {"text": "", "confidence": 0.0}

    peak = max(abs(s) for s in samples) / 32767.0
    confidence = min(0.99, peak * 1.2)

    return {
        "text": "make me an espresso",
        "confidence": round(confidence, 3),
    }


def speak(request):
    """Stand in for a synthesis model.

    A tone, not speech. The duration scales with the length of the text so
    that timing-dependent code sees something plausible.
    """
    text = request.get("text", "")
    seconds = max(0.2, min(10.0, len(text) * 0.06))
    count = int(SPEAKER_RATE * seconds)

    samples = [
        int(8000 * math.sin(2 * math.pi * 220.0 * n / SPEAKER_RATE)) for n in range(count)
    ]

    return {"audio": encode(samples), "sample_rate": SPEAKER_RATE}


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("transcribe", "speak"):
        sys.stderr.write("usage: reference_speech.py transcribe|speak\n")
        return 2

    handler = transcribe if sys.argv[1] == "transcribe" else speak

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            reply = handler(json.loads(line))
        except Exception as error:  # noqa: BLE001
            # A backend that crashes takes the seam down. One that answers
            # with a zero confidence leaves the machine usable: the engine
            # treats it as not heard and asks.
            if sys.argv[1] == "transcribe":
                reply = {"text": "", "confidence": 0.0}
            else:
                sys.stderr.write(f"speak failed: {error}\n")
                reply = {"audio": "", "sample_rate": SPEAKER_RATE}

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()

    return 0


if __name__ == "__main__":
    sys.exit(main())
