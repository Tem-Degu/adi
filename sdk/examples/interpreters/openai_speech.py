#!/usr/bin/env python3
"""Transcriber and speaker backends using a hosted provider.

The hosted counterpart to `speech_backends.py`, which runs the same two seams
from local weights. Both speak the process protocol: one JSON object per line
in, one JSON object per line out, so the engine cannot tell which of them is
serving a seam and neither can the rest of the SDK.

    export OPENAI_API_KEY=...
    python openai_speech.py transcribe gpt-4o-transcribe
    python openai_speech.py speak gpt-4o-mini-tts

    seams:
      transcriber:
        backend: process
        command: python3
        args: ["examples/interpreters/openai_speech.py", "transcribe",
               "gpt-4o-transcribe"]
        sample_rate: 16000
        min_confidence: 0.6

      speaker:
        backend: process
        command: python3
        args: ["examples/interpreters/openai_speech.py", "speak",
               "gpt-4o-mini-tts"]
        sample_rate: 24000

The key is read from the environment and never from an argument, because an
argument is visible to anyone who can list processes on the machine.

No dependency beyond the standard library. Audio crosses the wire as raw
signed 16-bit little-endian mono samples, base64 encoded, which is what the
engine sends and expects; the conversions to and from what the provider wants
are the whole of this file.

**What a hosted speech seam costs.** Every utterance leaves the building,
which is a product decision and not a protocol one: ADI-P 9.4.2 counts a seam
as the same seam wherever the weights are. It also does not work when the
network is down, and the round trip adds latency that a local model does not.
Against that, it needs no model files, no ONNX runtime and no megabytes of
flash, which is what makes it the sensible way to try the thing before
committing to hardware.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import base64
import io
import json
import math
import os
import struct
import sys
import urllib.request
import uuid
import wave

# One copy of the confidence estimate, shared with the local backend beside
# this file. A hosted transcriber that returned a different number for the
# same audio would make the threshold mean two things.
from speech_backends import confidence as level_confidence

TRANSCRIBE_URL = "https://api.openai.com/v1/audio/transcriptions"
SPEECH_URL = "https://api.openai.com/v1/audio/speech"
TIMEOUT_SECONDS = 120

# The rate the provider's synthesis returns when asked for raw PCM. Stated
# here because the configuration has to agree with it, and a mismatch is
# refused by the engine rather than resampled.
SPEECH_RATE = 24000


def key():
    value = os.environ.get("OPENAI_API_KEY", "").strip()
    if not value:
        sys.exit("OPENAI_API_KEY is not set")
    return value


def wav_bytes(samples, sample_rate):
    """The samples as a WAV file in memory.

    The provider takes a container rather than raw samples, and the engine
    sends raw samples, so one of the two has to wrap them. Doing it here keeps
    the wire format the same for every backend.
    """
    buffer = io.BytesIO()
    with wave.open(buffer, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(sample_rate)
        out.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    return buffer.getvalue()


def post_multipart(url, fields, filename, content, api_key):
    """A multipart form, assembled by hand.

    urllib has no multipart encoder and the alternative is a dependency, for
    what is thirty lines of byte concatenation.
    """
    boundary = uuid.uuid4().hex
    marker = ("--" + boundary).encode()
    body = bytearray()

    for name, value in fields.items():
        body += marker + b"\r\n"
        body += f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode()
        body += str(value).encode() + b"\r\n"

    body += marker + b"\r\n"
    body += (
        f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n'
        "Content-Type: audio/wav\r\n\r\n"
    ).encode()
    body += content + b"\r\n"
    body += marker + b"--\r\n"

    request = urllib.request.Request(
        url,
        data=bytes(body),
        headers={
            "Authorization": "Bearer " + api_key,
            "Content-Type": "multipart/form-data; boundary=" + boundary,
        },
    )
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
        return json.loads(response.read().decode())


def post_json(url, payload, api_key):
    """One request whose reply is bytes rather than JSON."""
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": "Bearer " + api_key,
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=TIMEOUT_SECONDS) as response:
        return response.read()


# --------------------------------------------------------------- transcriber


def spoken_confidence(reply, samples, text):
    """How much of a guess the transcript is.

    ADI-P 9.4.2 requires a confidence with every transcript and treats one
    below the configured threshold as not heard, at which point the machine
    asks instead of acting. Mishearing "hotter" as "shorter" is a physical
    action on a machine with a heating element.

    Where the provider returns log probabilities, they are the better signal
    and are used. Where it does not, this falls back to the same estimate the
    local backend makes from the signal level and the number of words, so the
    threshold means one thing across both.
    """
    logprobs = reply.get("logprobs") or []
    values = [item["logprob"] for item in logprobs if "logprob" in item]

    if values:
        mean = sum(values) / len(values)
        return round(min(0.97, math.exp(mean)), 3)

    return level_confidence([s / 32768.0 for s in samples], text)


def transcribe(request, model, api_key):
    encoded = request.get("audio", "")
    rate = int(request.get("sample_rate", 16000))
    raw = base64.b64decode(encoded) if encoded else b""

    if not raw:
        return {"text": "", "confidence": 0.0}
    if len(raw) % 2:
        raise ValueError("odd byte count: not 16-bit samples")

    samples = struct.unpack(f"<{len(raw) // 2}h", raw)

    reply = post_multipart(
        TRANSCRIBE_URL,
        {
            "model": model,
            "response_format": "json",
            # Asked for, and used where the provider sends it. Where it does
            # not, the fallback in spoken_confidence applies.
            "include[]": "logprobs",
        },
        "utterance.wav",
        wav_bytes(samples, rate),
        api_key,
    )

    text = (reply.get("text") or "").strip()
    return {"text": text, "confidence": spoken_confidence(reply, samples, text)}


# ------------------------------------------------------------------- speaker


def speak(request, model, api_key):
    text = (request.get("text") or "").strip()
    if not text:
        return {"audio": "", "sample_rate": SPEECH_RATE}

    # Raw PCM rather than a compressed format, because the engine takes
    # samples and decoding mp3 would need a dependency to undo something that
    # did not need doing.
    audio = post_json(
        SPEECH_URL,
        {
            "model": model,
            "voice": os.environ.get("OPENAI_TTS_VOICE", "alloy"),
            "input": text,
            "response_format": "pcm",
        },
        api_key,
    )

    if len(audio) % 2:
        audio = audio[:-1]

    return {"audio": base64.b64encode(audio).decode(), "sample_rate": SPEECH_RATE}


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("transcribe", "speak"):
        sys.exit("usage: openai_speech.py transcribe|speak <model>")

    mode = sys.argv[1]
    model = sys.argv[2]
    api_key = key()
    handler = transcribe if mode == "transcribe" else speak

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            reply = handler(json.loads(line), model, api_key)
        except Exception as problem:  # noqa: BLE001
            # A seam that dies takes the session with it. Reporting the
            # failure as an empty result with no confidence lets the engine
            # treat it as not heard, which is what it does with anything it
            # cannot trust.
            sys.stderr.write(f"openai_speech: {problem}\n")
            sys.stderr.flush()
            reply = (
                {"text": "", "confidence": 0.0}
                if mode == "transcribe"
                else {"audio": "", "sample_rate": SPEECH_RATE}
            )

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
