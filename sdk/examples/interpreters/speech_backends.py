#!/usr/bin/env python3
"""Transcriber and speaker backends using real models.

Two seams, one file, selected by the first argument:

    python speech_backends.py transcribe models/transcriber/moonshine-base
    python speech_backends.py speak models/speaker/en_GB-alba-medium.onnx

    pip install useful-moonshine-onnx piper-tts

A third form loads no model:

    python speech_backends.py transcribe --explain

It reports how each request would be served, including the confidence this
audio would earn at several transcript lengths, which is what makes the
derivation below checkable where the weights are not present.

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

VERIFIED against Moonshine base and Piper en_GB-alba-medium on an x86-64 host.
Speech is synthesised at about twelve times real time; a short utterance is
transcribed in about two seconds.

Two API corrections were needed, noted at each site: Piper no longer provides
synthesize_stream_raw, and Moonshine adds the batch dimension itself, so it
must be given a flat array rather than one already shaped [batch, samples].

The confidence derivation below remains a stand-in. It was not calibrated
against recordings of real speech in a kitchen, which is where a transcript
becomes unreliable. Its properties are now checked by
crates/adi-cli/tests/shipped_speech.rs: room noise stays below a usable
threshold however many words come back, a clear sentence rises above one, a
transcript with no words scores zero, and confidence does not fall as a
recording gets clearer. Those are the properties a calibration would have to
preserve; they are not themselves a calibration.

Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
"""

import base64
import json
import math
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


def rms(samples):
    """Root mean square of samples already scaled to -1 to 1."""
    if len(samples) == 0:
        return 0.0
    return math.sqrt(sum(float(s) * float(s) for s in samples) / len(samples))


def confidence(samples, text):
    """A stand-in until a better signal is available.

    Built from two things that are cheap and correlate with whether a
    transcript can be trusted: whether there was enough signal to hear, and
    whether anything was recognised at all. It is deliberately conservative,
    because the cost of a confident wrong transcript is an unwanted physical
    action and the cost of an unnecessary question is a person repeating
    themselves.

    Written in plain arithmetic rather than through numpy so that it can be
    checked without the ONNX runtime present. It is the one number in this
    file that decides whether a machine acts or asks, and it was the one part
    that nothing exercised.
    """
    if not text or not text.strip():
        return 0.0

    level_rms = rms(samples)

    # Below this is room noise rather than speech.
    if level_rms < 0.005:
        return 0.1

    level = min(1.0, level_rms / 0.05)
    # A single word is more often a misrecognition of noise than a sentence is.
    length = min(1.0, len(text.split()) / 3.0)

    return round(min(0.97, 0.5 * level + 0.5 * length), 3)



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

        # Flat, not reshaped to [batch, samples]. The library adds the batch
        # dimension itself, so handing it a two-dimensional array produced a
        # three-dimensional one and failed its own shape assertion. The
        # confidence estimate below works on either shape.
        audio = np.array(samples, dtype=np.float32) / 32768.0

        text = self.moonshine.transcribe(audio, self.model_path)
        text = text[0] if isinstance(text, (list, tuple)) else str(text)

        return {"text": text.strip(), "confidence": confidence(audio, text)}



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

        # synthesize yields AudioChunk objects, each carrying the samples in
        # several forms. The raw signed 16-bit bytes are taken directly rather
        # than converted from floats, because that is the format the wire
        # expects and converting twice loses nothing but gains nothing either.
        #
        # An earlier version called synthesize_stream_raw, which this library
        # no longer provides. The sample rate is read from the chunk rather
        # than from the configuration, since a chunk states the rate it was
        # actually produced at.
        raw = bytearray()
        rate = self.rate
        for chunk in self.voice.synthesize(text):
            raw.extend(chunk.audio_int16_bytes)
            rate = chunk.sample_rate

        samples = struct.unpack(f"<{len(raw) // 2}h", bytes(raw))
        return {"audio": encode(samples), "sample_rate": rate}


# ---------------------------------------------------------------------- main


def scored(samples, text):
    """What the transcriber would return for this audio and this transcript.

    `handle` answers a recording with no samples at zero before the rule is
    reached, so the rule alone would report a tenth for audio that was never
    heard. Reporting what is served matters more here than reporting what the
    rule computes.
    """
    if not samples:
        return 0.0
    return confidence(samples, text)


def explain(mode, stream):
    """Report how each request would be served, without loading a model.

    `--explain` exists so that this file can be tested where the weights
    cannot be. Moonshine and Piper are hundreds of megabytes and CI has
    neither, which is why nothing exercised this file at all while the
    interpreter beside it had eleven test files.

    For the transcriber it reports the confidence the rule would assign to
    this audio at three transcript lengths. That is the number deciding
    whether a machine acts on what it heard or asks again, and reporting it
    without weights is what makes it checkable: near-silence must stay below
    a usable threshold however many words come back, and clear speech must
    rise above one.

    It says nothing about transcription accuracy or voice quality. Those need
    the models.
    """
    for line in stream:
        line = line.strip()
        if not line:
            continue

        request = json.loads(line)

        if mode == "transcribe":
            samples = decode(request.get("audio", ""))
            scaled = [s / 32768.0 for s in samples]
            plan = {
                "seam": "transcriber",
                "sample_rate": TRANSCRIBER_RATE,
                "given_rate": request.get("sample_rate"),
                "samples": len(samples),
                "rms": round(rms(scaled), 6),
                # The rule applied at the transcript lengths it distinguishes,
                # through the same empty-audio guard the served path takes, so
                # that what is reported here is what would be returned.
                "confidence": {
                    "nothing_recognised": scored(scaled, ""),
                    "one_word": scored(scaled, "espresso"),
                    "a_sentence": scored(scaled, "make me an espresso please"),
                },
            }
        else:
            text = request.get("text", "")
            plan = {
                "seam": "speaker",
                "text": text,
                # An empty request is answered with silence rather than a
                # failure, so the explanation is still available as text.
                "silence": not text.strip(),
            }

        sys.stdout.write(json.dumps(plan) + "\n")
        sys.stdout.flush()

    return 0


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("transcribe", "speak"):
        sys.stderr.write("usage: speech_backends.py transcribe|speak <model>\n")
        return 2

    if "--explain" in sys.argv[2:]:
        return explain(sys.argv[1], sys.stdin)

    if len(sys.argv) < 3:
        sys.stderr.write(
            "usage: speech_backends.py transcribe|speak <model>\n"
            "       speech_backends.py transcribe|speak --explain   (no model)\n"
        )
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
