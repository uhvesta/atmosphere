#!/usr/bin/env python3
"""Makes Humata 2's two notification sounds from sine partials.

Run it again only to change the sounds; the WAV files it writes are checked
in next to it and bundled into Humata 2.app/Contents/Resources by app.bzl.

    python3 apps/humata2/app/sounds/make_sounds.py [out_dir]

Both are soft (peak about -12 and -16 dBFS), quick to start, gently decaying and
under 0.4 s (Fable, NOTIFY2). The Inbox sound is two rising notes, an agent waiting on you.
The To-do sound is one lower, rounder note, something to look at later.
Only the standard library is used, so the output is the same on every run.
"""

import math
import os
import struct
import sys
import wave

RATE = 44100
PEAK = 10 ** (-12 / 20)  # about -12 dBFS


def note(freq, start, length, partials, attack=0.006, decay=6.0):
    """One soft note: sine partials (multiple, level), quick attack, exp decay."""
    out = []
    for i in range(int(length * RATE)):
        t = i / RATE
        rise = min(1.0, t / attack)
        fall = math.exp(-decay * t)
        # A raised-cosine tail so the note ends at silence, never a click.
        tail = 1.0
        left = length - t
        if left < 0.04:
            tail = 0.5 - 0.5 * math.cos(math.pi * left / 0.04)
        level = rise * fall * tail
        value = sum(amp * math.sin(2 * math.pi * freq * mult * t) for mult, amp in partials)
        out.append((start + i, level * value))
    return out


def render(notes, length, peak=PEAK):
    samples = [0.0] * int(length * RATE)
    for part in notes:
        for at, value in part:
            if at < len(samples):
                samples[at] += value
    top = max(abs(s) for s in samples) or 1.0
    return [s / top * peak for s in samples]


def write(path, samples):
    with wave.open(path, "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(b"".join(struct.pack("<h", int(s * 32767)) for s in samples))


# A warm sine with a little octave and twelfth: bell-like but not bright.
SOFT = [(1.0, 1.0), (2.0, 0.18), (3.0, 0.05)]


def inbox():
    # E5 then A5, 70 ms apart: a small rising "someone is waiting".
    first = note(659.26, 0, 0.28, SOFT, decay=9.0)
    second = note(880.0, int(0.07 * RATE), 0.31, SOFT, decay=8.0)
    return render([first, second], 0.38)


def todo():
    # One rounder G4 with a faint fifth: a gentle "noted".
    # Lower and softer than the Inbox sound (about -16 dBFS).
    one = note(392.0, 0, 0.30, [(1.0, 1.0), (1.5, 0.12), (2.0, 0.10)], attack=0.010, decay=9.0)
    return render([one], 0.30, peak=10 ** (-16 / 20))


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    write(os.path.join(out_dir, "humata2-inbox.wav"), inbox())
    write(os.path.join(out_dir, "humata2-todo.wav"), todo())


if __name__ == "__main__":
    main()
