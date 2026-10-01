#!/usr/bin/env python3
"""Transcriber and speaker backends using real models.

Two seams, one file, selected by the first argument:

    python speech_backends.py transcribe models/transcriber/moonshine-base
    python speech_backends.py speak models/speaker/en_GB-alba-medium.onnx

    pip install useful-moonshine-onnx piper-tts

    seams:
      transcriber:
        backend: process
        command: python
        args: ["examples/interpreters/speech_backends.py", "transcribe",
               "models/transcriber/moonshine-base"]
        sample_rate: 16000
        min_confidence: 0.6

      speaker:
        backend: process
        command: python
        args: ["examples/interpreters/speech_backends.py", "speak",
               "models/speaker/en_GB-alba-medium.onnx"]
        sample_rate: 22050

The protocol, one JSON object per line in each direction:

    transcribe  -> {"audio": "<base64>", "sample_rate": 16000}
                <- {"text": "...", "confidence": 0.0 to 1.0}

    speak       -> {"text": "..."}
                <- {"audio": "<base64>", "sample_rate": 22050}

Audio is base64 of 16-bit signed, little endian, single channel samples.

NOT YET VERIFIED against real models. Written from the library documentation
and not run, because the machine it was written on cannot hold the weights.
Expect to correct something on first use, particularly the confidence
derivation below.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import base64
import json
import struct
import sys

TRANSCRIBER_RATE = 16000


def decode(encoded):
    raw = base64.b64decode(encoded)
    if len(raw) % 2:
        raise ValueError("odd byte count: not 16-bit samples")
    return struct.unpack(f"<{len(raw) // 2}h", raw)


def encode(samples):
    return base64.b64encode(struct.pack(f"<{len(samples)}h", *samples)).decode()


# --------------------------------------------------------------- transcriber


class Transcriber:
    """Moonshine, through its ONNX package.

    ADI-P 9.4.2 requires a confidence with every transcript, and treats one
    below the configured threshold as not heard, at which point the machine
    asks rather than acting. Mishearing "hotter" as "shorter" is a physical
    action on a machine with a heating element.

    Moonshine does not return a confidence directly, so one is derived below.
    That derivation is the least certain part of this file and should be
    checked against real recordings before being relied on: specifically, that
    quiet or unclear speech scores below the threshold and clear speech above
    it. Until then, prefer a conservative threshold.
    """

    def __init__(self, model_path):
        import moonshine_onnx

        self.moonshine = moonshine_onnx
        self.model_path = model_path

    def handle(self, request):
        import numpy as np

        samples = decode(request["audio"])
        if not samples:
            return {"text": "", "confidence": 0.0}

        audio = np.array(samples, dtype=np.float32) / 32768.0
        audio = audio.reshape(1, -1)

        text = self.moonshine.transcribe(audio, self.model_path)
        text = text[0] if isinstance(text, (list, tuple)) else str(text)

        return {"text": text.strip(), "confidence": self.confidence(audio, text)}

    @staticmethod
    def confidence(audio, text):
        """A stand-in until a better signal is available.

        Built from two things that are cheap and correlate with whether a
        transcript can be trusted: whether there was enough signal to hear,
        and whether anything was recognised at all. It is deliberately
        conservative, because the cost of a confident wrong transcript is an
        unwanted physical action and the cost of an unnecessary question is a
        person repeating themselves.
        """
        import numpy as np

        if not text or not text.strip():
            return 0.0

        rms = float(np.sqrt(np.mean(np.square(audio))))

        # Below this is room noise rather than speech.
        if rms < 0.005:
            return 0.1

        level = min(1.0, rms / 0.05)
        # A single word is more often a misrecognition of noise than a
        # sentence is.
        length = min(1.0, len(text.split()) / 3.0)

        return round(min(0.97, 0.5 * level + 0.5 * length), 3)


# ------------------------------------------------------------------- speaker


class Speaker:
    """Piper, through its Python package.

    Piper handles phonemisation internally, which is the part that would
    otherwise require espeak-ng to be installed and located separately.
    """

    def __init__(self, model_path):
        from piper import PiperVoice

        self.voice = PiperVoice.load(model_path)
        self.rate = self.voice.config.sample_rate

    def handle(self, request):
        text = request.get("text", "")
        if not text.strip():
            return {"audio": "", "sample_rate": self.rate}

        samples = []
        for chunk in self.voice.synthesize_stream_raw(text):
            samples.extend(struct.unpack(f"<{len(chunk) // 2}h", chunk))

        return {"audio": encode(samples), "sample_rate": self.rate}


# ---------------------------------------------------------------------- main


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("transcribe", "speak"):
        sys.stderr.write("usage: speech_backends.py transcribe|speak <model>\n")
        return 2

    mode, model_path = sys.argv[1], sys.argv[2]

    try:
        backend = Transcriber(model_path) if mode == "transcribe" else Speaker(model_path)
    except ImportError as error:
        sys.stderr.write(
            f"{error}\n"
            "    pip install useful-moonshine-onnx piper-tts\n"
        )
        return 1

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            reply = backend.handle(json.loads(line))
        except Exception as error:  # noqa: BLE001
            sys.stderr.write(f"{mode} failed: {error}\n")
            # A transcriber that cannot hear reports no confidence, and the
            # engine then asks. A speaker that cannot speak returns silence,
            # and the explanation is still available as text.
            reply = (
                {"text": "", "confidence": 0.0}
                if mode == "transcribe"
                else {"audio": "", "sample_rate": getattr(backend, "rate", 22050)}
            )

        sys.stdout.write(json.dumps(reply) + "\n")
        sys.stdout.flush()

    return 0


if __name__ == "__main__":
    sys.exit(main())
