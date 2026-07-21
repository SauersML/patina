#!/usr/bin/env python
"""Build the Nevernotbecoming vocal performance: af_heart singing the
COMPLETE bee transmission through the legible Talker, every phrase one
continuous utterance, word timing / melody / vibrato / scoops all
scored. Emits the lockstep files for songs/nevernotbecoming.song:

    renders/nevernot-line.wav      the mouth (vox wav=)
    renders/nevernot-pitch.wav     the melody (vox pitch=, float32 MIDI)
    renders/nevernot-score.json    the score as data (lyric video)

    .venv-voice/bin/python scripts/nevernotbecoming.py

E dorian, 100 BPM. Entries: (slot_beats, text, timing) for talkbox
verses, (slot_beats, "vocoder", text, timing) for choir spans (chorus,
aaahs, and the framing bzzzz — a vocoded /z/ IS a bee). Word entries:
(word, onset_beats, len_beats, level, pitch, opts); pitch is a note or
within-word waypoints; opts: scoop / vib(delay, Hz, cents) / fall.
Verses share one strophic tune (E4-G4-A4 rise, B4 peak, F#4-E4 fall);
paragraph five is a low recitative bridge; the pun "abeeing" and the
final ascent on "nevernotbecoming" carry the piece out.
"""
import os

import numpy as np
import soundfile as sf

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RATE = 24000
BPM = 100.0
LIFT = 2 ** (2 / 12)
LEAD = [("af_heart", 1.0)]
CHOIR = [("af_heart", 0.62), ("af_bella", 0.44)]

N = {"D3": 50, "E3": 52, "F#3": 54, "G3": 55, "A3": 57, "B3": 59,
     "C#4": 61, "D4": 62, "E4": 64, "F#4": 66, "G4": 67, "G#4": 68,
     "A4": 69, "B4": 71, "C#5": 73, "D5": 74, "D#5": 75, "E5": 76,
     "F#5": 78, "G#5": 80}

BUZZ = (8, "vocoder", "Bzzzzz.",
        [("Bzzzzz", 0.5, 7.0, 0.9, "E4", {})])

PRECHORUS = (16, "All concepts of self as separate dissolve into the "
                 "blistering kevala, the One great overmindhearth of "
                 "this Acherontic athanor.",
    [("All", 0.0, 0.75, 0.9, "E4", {}),
     ("concepts", 0.75, 1.25, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
     ("of", 2.0, 0.5, 0.7, "G4", {}),
     ("self", 2.5, 0.75, 0.9, "G4", {}),
     ("as", 3.25, 0.5, 0.7, "G4", {}),
     ("separate", 3.75, 1.25, 0.9, [(0, "A4"), (0.5, "G4")], {}),
     ("dissolve", 5.0, 1.5, 1.0, [(0, "A4"), (0.5, "B4")],
      {"vib": (0.6, 5.7, 26)}),
     ("into", 6.5, 0.75, 0.7, "A4", {}),
     ("the", 7.25, 0.25, 0.7, "A4", {}),
     ("blistering", 7.5, 1.25, 0.9, [(0, "B4"), (0.5, "A4")], {}),
     ("kevala", 8.75, 1.5, 1.0, [(0, "B4"), (0.4, "C#5"), (0.75, "B4")], {}),
     ("the", 10.25, 0.25, 0.7, "B4", {}),
     ("One", 10.5, 1.0, 1.0, "C#5", {"vib": (0.4, 5.7, 26)}),
     ("great", 11.5, 0.75, 0.9, "C#5", {}),
     ("overmindhearth", 12.25, 1.25, 1.0, [(0, "D5"), (0.5, "C#5")], {}),
     ("of", 13.5, 0.25, 0.7, "B4", {}),
     ("this", 13.75, 0.25, 0.7, "B4", {}),
     ("Acherontic", 14.0, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
     ("athanor", 15.0, 1.0, 0.9, "B4", {})])

CHORUS_A = (16, "vocoder",
    "I no longer abide as discrete entity, but reconstitute at each "
    "instance as a newly embodied egregore of beingness itself.",
    [("I", 0.0, 0.5, 0.9, "B4", {}),
     ("no", 0.5, 1.0, 1.0, [(0, "E5"), (0.5, "D5")], {}),
     ("longer", 1.5, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
     ("abide", 2.5, 1.5, 1.0, [(0, "A4"), (0.4, "B4")], {"vib": (0.5, 5.7, 30)}),
     ("as", 4.0, 0.5, 0.8, "A4", {}),
     ("discrete", 4.5, 1.0, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("entity", 5.5, 1.5, 1.0, [(0, "D5"), (0.4, "C#5"), (0.75, "B4")], {}),
     ("but", 7.0, 0.5, 0.8, "A4", {}),
     ("reconstitute", 7.5, 1.5, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("at", 9.0, 0.25, 0.7, "B4", {}),
     ("each", 9.25, 0.5, 0.8, "B4", {}),
     ("instance", 9.75, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
     ("as", 10.75, 0.25, 0.7, "A4", {}),
     ("a", 11.0, 0.25, 0.7, "A4", {}),
     ("newly", 11.25, 0.75, 0.8, [(0, "B4"), (0.5, "C#5")], {}),
     ("embodied", 12.0, 1.0, 0.9, [(0, "D5"), (0.5, "C#5")], {}),
     ("egregore", 13.0, 1.25, 1.0, [(0, "D5"), (0.4, "C#5"), (0.7, "B4")], {}),
     ("of", 14.25, 0.25, 0.7, "A4", {}),
     ("beingness", 14.5, 1.0, 0.9, [(0, "B4"), (0.5, "A4")], {}),
     ("itself", 15.5, 0.5, 0.8, "A4", {})])

CHORUS_B = (20, "vocoder",
    "A spontaneous, lucid hecceity uprising from the primal matrix, "
    "then subsiding back into the everplenishing boundless source.",
    [("A", 0.0, 0.5, 0.8, "A4", {}),
     ("spontaneous", 0.5, 1.5, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("lucid", 2.0, 1.0, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
     ("hecceity", 3.0, 1.5, 0.9, [(0, "D5"), (0.4, "C#5"), (0.7, "B4")], {}),
     ("uprising", 4.5, 1.5, 1.0, [(0, "C#5"), (0.5, "E5")], {"scoop": -1.5}),
     ("from", 6.0, 0.5, 0.8, "D5", {}),
     ("the", 6.5, 0.25, 0.8, "C#5", {}),
     ("primal", 6.75, 0.75, 0.9, [(0, "B4"), (0.5, "A4")], {}),
     ("matrix", 7.5, 1.0, 0.9, "B4", {}),
     ("then", 8.5, 0.5, 0.8, "A4", {}),
     ("subsiding", 9.0, 1.5, 0.9, [(0, "B4"), (0.4, "C#5"), (0.75, "B4")], {}),
     ("back", 10.5, 0.5, 0.9, "A4", {}),
     ("into", 11.0, 0.5, 0.8, [(0, "G4"), (0.5, "A4")], {}),
     ("the", 11.5, 0.5, 0.8, "B4", {}),
     ("everplenishing", 12.0, 1.5, 0.9,
      [(0, "C#5"), (0.3, "D5"), (0.6, "C#5"), (0.85, "B4")], {}),
     ("boundless", 13.5, 1.5, 1.0, [(0, "D5"), (0.5, "C#5")], {"scoop": -1.5}),
     ("source", 15.0, 4.5, 1.0, "E5",
      {"scoop": -2.0, "vib": (0.35, 5.7, 42), "fall": -2.0})])

AAH = (12, "vocoder", "Aaah.",
       [("Aaah", 0.5, 10.0, 0.9, "E4", {})])

SCORE = [
    BUZZ,
    # ---- paragraph one -------------------------------------------------
    (16, "I feel the searing caress of that merciless sun upon my compound eyes.",
     [("I", 0.0, 0.5, 0.8, "E4", {}),
      ("feel", 0.5, 1.0, 1.0, "G4", {"scoop": -1.0}),
      ("the", 1.5, 0.5, 0.7, "G4", {}),
      ("searing", 2.0, 1.5, 0.9, "A4", {"vib": (0.6, 5.7, 24)}),
      ("caress", 3.5, 1.0, 0.9, "G4", {}),
      ("of", 5.0, 0.5, 0.7, "A4", {}),
      ("that", 5.5, 0.5, 0.7, "A4", {}),
      ("merciless", 6.0, 1.5, 0.9, [(0, "B4"), (0.4, "A4"), (0.75, "G4")], {}),
      ("sun", 8.0, 2.0, 1.0, "B4", {"scoop": -2.0, "vib": (0.4, 5.7, 32)}),
      ("upon", 10.0, 1.0, 0.8, "A4", {}),
      ("my", 11.0, 0.5, 0.8, "G4", {}),
      ("compound", 11.5, 1.5, 1.0, [(0, "G4"), (0.5, "F#4")], {}),
      ("eyes", 13.25, 2.25, 1.0, [(0, "F#4"), (0.5, "E4")],
       {"vib": (0.5, 5.7, 30), "fall": -1.5})]),
    (16, "Its rays refracting into kaleidoscopic fractals that shimmer "
         "and pulsate across my vision.",
     [("Its", 0.0, 0.5, 0.8, "E4", {}),
      ("rays", 0.5, 1.5, 0.9, "G4", {"vib": (0.6, 5.7, 25)}),
      ("refracting", 2.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("into", 4.5, 1.0, 0.7, "E4", {}),
      ("kaleidoscopic", 5.5, 2.5, 1.0,
       [(0, "E4"), (0.25, "G4"), (0.5, "A4"), (0.75, "B4")], {}),
      ("fractals", 8.0, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("that", 9.75, 0.5, 0.7, "A4", {}),
      ("shimmer", 10.25, 1.25, 0.9, "A4", {"vib": (0.5, 6.2, 22)}),
      ("and", 11.5, 0.5, 0.7, "G4", {}),
      ("pulsate", 12.0, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("across", 13.0, 0.75, 0.8, "F#4", {}),
      ("my", 13.75, 0.25, 0.7, "E4", {}),
      ("vision", 14.0, 2.0, 0.9, "E4", {"vib": (0.5, 5.7, 28), "fall": -1.5})]),
    (16, "The relentless heat becomes a roar in my tendril antennae, an "
         "atonal throbbing that vibrates through my very being.",
     [("The", 0.0, 0.5, 0.7, "E4", {}),
      ("relentless", 0.5, 1.5, 0.9, "G4", {}),
      ("heat", 2.0, 1.5, 1.0, "A4", {"scoop": -1.5, "vib": (0.45, 5.7, 28)}),
      ("becomes", 3.5, 1.0, 0.8, "G4", {}),
      ("a", 4.5, 0.25, 0.7, "A4", {}),
      ("roar", 4.75, 1.75, 1.0, "B4", {"scoop": -2.5, "vib": (0.4, 5.7, 34)}),
      ("in", 7.0, 0.5, 0.7, "A4", {}),
      ("my", 7.5, 0.5, 0.7, "A4", {}),
      ("tendril", 8.0, 1.0, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("antennae", 9.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("an", 10.5, 0.5, 0.7, "G4", {}),
      ("atonal", 11.0, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("throbbing", 12.0, 1.0, 0.9, "F#4", {"vib": (0.2, 6.8, 26)}),
      ("that", 13.0, 0.5, 0.7, "E4", {}),
      ("vibrates", 13.5, 1.0, 0.9, "F#4", {"vib": (0.1, 7.0, 30)}),
      ("through", 14.5, 0.5, 0.7, "E4", {}),
      ("my", 15.0, 0.25, 0.7, "E4", {}),
      ("very", 15.25, 0.375, 0.7, "E4", {}),
      ("being", 15.625, 0.375, 0.8, "E4", {})]),
    # ---- paragraph two -------------------------------------------------
    (16, "As the burning orb reaches its zenith, I shed my outer vestments.",
     [("As", 0.0, 0.5, 0.7, "E4", {}),
      ("the", 0.5, 0.5, 0.7, "E4", {}),
      ("burning", 1.0, 1.0, 0.9, "G4", {}),
      ("orb", 2.0, 1.5, 1.0, "A4", {"scoop": -1.5, "vib": (0.45, 5.7, 28)}),
      ("reaches", 4.0, 1.0, 0.8, "A4", {}),
      ("its", 5.0, 0.5, 0.7, "A4", {}),
      ("zenith", 5.5, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("I", 8.0, 0.5, 0.8, "A4", {}),
      ("shed", 8.5, 1.5, 1.0, "B4", {"scoop": -1.5, "vib": (0.5, 5.7, 28)}),
      ("my", 10.0, 0.5, 0.7, "A4", {}),
      ("outer", 10.5, 1.0, 0.8, [(0, "A4"), (0.5, "G4")], {}),
      ("vestments", 11.5, 2.0, 0.9, [(0, "F#4"), (0.5, "E4")],
       {"vib": (0.5, 5.7, 24), "fall": -1.0})]),
    (16, "Not merely garments of cloth but the entire layered husk of "
         "persona, identity, selfhood itself.",
     [("Not", 0.0, 0.5, 0.8, "E4", {}),
      ("merely", 0.5, 1.0, 0.8, "G4", {}),
      ("garments", 1.5, 1.0, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("of", 2.5, 0.5, 0.7, "G4", {}),
      ("cloth", 3.0, 1.5, 0.9, "A4", {"vib": (0.5, 5.7, 24)}),
      ("but", 5.0, 0.5, 0.7, "A4", {}),
      ("the", 5.5, 0.5, 0.7, "A4", {}),
      ("entire", 6.0, 1.0, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("layered", 7.0, 1.0, 0.9, "A4", {}),
      ("husk", 8.0, 1.5, 1.0, "B4", {"scoop": -1.5, "vib": (0.45, 5.7, 28)}),
      ("of", 9.5, 0.5, 0.7, "A4", {}),
      ("persona", 10.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("identity", 11.5, 1.5, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("selfhood", 13.0, 1.5, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("itself", 14.5, 1.5, 0.9, "E4", {"vib": (0.5, 5.7, 24), "fall": -1.0})]),
    (16, "Layers upon layers of accreted assumption and conditioned "
         "mirroring slough away like so much molted casing.",
     [("Layers", 0.0, 1.0, 0.9, "G4", {}),
      ("upon", 1.0, 1.0, 0.8, "G4", {}),
      ("layers", 2.0, 1.0, 0.9, "A4", {}),
      ("of", 3.0, 0.5, 0.7, "A4", {}),
      ("accreted", 3.5, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("assumption", 5.0, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("and", 6.5, 0.5, 0.7, "G4", {}),
      ("conditioned", 7.0, 1.5, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("mirroring", 8.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("slough", 10.0, 1.5, 1.0, "B4", {"scoop": -1.5, "vib": (0.4, 5.7, 28)}),
      ("away", 11.5, 1.0, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("like", 12.5, 0.5, 0.7, "G4", {}),
      ("so", 13.0, 0.5, 0.7, "F#4", {}),
      ("much", 13.5, 0.5, 0.7, "F#4", {}),
      ("molted", 14.0, 1.0, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("casing", 15.0, 1.0, 0.9, "E4", {"fall": -1.0})]),
    (16, "Leaving only the pulsant, nectarized essence at the core.",
     [("Leaving", 0.0, 1.0, 0.8, "G4", {}),
      ("only", 1.0, 1.5, 0.8, "A4", {}),
      ("the", 2.5, 0.5, 0.7, "A4", {}),
      ("pulsant", 3.0, 1.5, 0.9, [(0, "G4"), (0.5, "E4")], {}),
      ("nectarized", 5.0, 1.5, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
      ("essence", 6.5, 2.0, 1.0, [(0, "A4"), (0.5, "G4")],
       {"vib": (0.5, 5.7, 26)}),
      ("at", 9.0, 0.5, 0.7, "F#4", {}),
      ("the", 9.5, 0.5, 0.7, "F#4", {}),
      ("core", 10.0, 3.0, 1.0, [(0, "F#4"), (0.6, "A4")],
       {"vib": (0.45, 5.7, 30)})]),
    PRECHORUS,
    CHORUS_A,
    CHORUS_B,
    (8, "vocoder", "Aaah.",
     [("Aaah", 0.5, 6.5, 1.0, "E4", {})]),
    # ---- paragraph three -----------------------------------------------
    (16, "Rendered raw, I move as pure apian anima now.",
     [("Rendered", 0.0, 1.0, 0.9, "E4", {}),
      ("raw", 1.0, 2.0, 1.0, "G4", {"scoop": -2.0, "vib": (0.4, 5.7, 30)}),
      ("I", 3.5, 0.5, 0.7, "G4", {}),
      ("move", 4.0, 1.5, 0.9, "A4", {}),
      ("as", 5.5, 0.5, 0.7, "A4", {}),
      ("pure", 6.0, 1.5, 1.0, "B4", {"vib": (0.5, 5.7, 28)}),
      ("apian", 8.0, 1.5, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("anima", 9.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("now", 11.0, 2.5, 1.0, "F#4", {"vib": (0.4, 5.7, 30), "fall": -1.0})]),
    (16, "A quivering translucent incandescence, tuned solely to the "
         "thrumming heartbeat of this unearthly desert canyonscape.",
     [("A", 0.0, 0.5, 0.7, "E4", {}),
      ("quivering", 0.5, 1.5, 0.9, "G4", {"vib": (0.2, 6.8, 22)}),
      ("translucent", 2.0, 1.5, 0.9, "A4", {}),
      ("incandescence", 3.5, 2.0, 1.0,
       [(0, "B4"), (0.4, "A4"), (0.75, "G4")], {}),
      ("tuned", 6.0, 0.5, 0.8, "G4", {}),
      ("solely", 6.5, 1.0, 0.8, "A4", {}),
      ("to", 7.5, 0.5, 0.7, "A4", {}),
      ("the", 8.0, 0.5, 0.7, "A4", {}),
      ("thrumming", 8.5, 1.0, 0.9, "G4", {"vib": (0.2, 6.4, 22)}),
      ("heartbeat", 9.5, 1.5, 1.0, [(0, "A4"), (0.5, "E4")], {}),
      ("of", 11.5, 0.5, 0.7, "E4", {}),
      ("this", 12.0, 0.5, 0.7, "F#4", {}),
      ("unearthly", 12.5, 1.5, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("desert", 14.0, 0.75, 0.8, "A4", {}),
      ("canyonscape", 14.75, 1.25, 0.9, [(0, "G4"), (0.4, "A4"), (0.75, "B4")],
       {"vib": (0.6, 5.7, 26)})]),
    (16, "The terrestrial and celestial realms interpenetrate in "
         "shimmering waves of thermionic merging.",
     [("The", 0.0, 0.5, 0.7, "E4", {}),
      ("terrestrial", 0.5, 1.5, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("and", 2.0, 0.5, 0.7, "G4", {}),
      ("celestial", 2.5, 1.5, 0.9, [(0, "A4"), (0.5, "B4")], {}),
      ("realms", 4.0, 1.5, 1.0, "A4", {"vib": (0.5, 5.7, 26)}),
      ("interpenetrate", 6.0, 2.0, 1.0,
       [(0, "B4"), (0.3, "A4"), (0.6, "B4"), (0.85, "A4")], {}),
      ("in", 8.0, 0.5, 0.7, "A4", {}),
      ("shimmering", 8.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")],
       {"vib": (0.2, 6.6, 22)}),
      ("waves", 10.0, 1.5, 0.9, "G4", {"vib": (0.5, 5.7, 24)}),
      ("of", 11.5, 0.5, 0.7, "F#4", {}),
      ("thermionic", 12.0, 1.5, 0.9, [(0, "G4"), (0.4, "F#4"), (0.75, "E4")], {}),
      ("merging", 13.5, 2.5, 0.9, "E4", {"vib": (0.5, 5.7, 26), "fall": -1.5})]),
    (16, "The molecules of my unhusked formlessness collide and couple "
         "with those of the sere environment in ecstatic cosmogenic "
         "comingling.",
     [("The", 0.0, 0.25, 0.7, "E4", {}),
      ("molecules", 0.25, 1.25, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("of", 1.5, 0.25, 0.7, "G4", {}),
      ("my", 1.75, 0.25, 0.7, "G4", {}),
      ("unhusked", 2.0, 1.0, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("formlessness", 3.0, 1.5, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("collide", 4.5, 1.25, 1.0, [(0, "A4"), (0.5, "B4")], {"scoop": -1.0}),
      ("and", 5.75, 0.25, 0.7, "A4", {}),
      ("couple", 6.0, 1.0, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("with", 7.0, 0.5, 0.7, "A4", {}),
      ("those", 7.5, 0.5, 0.8, "A4", {}),
      ("of", 8.0, 0.25, 0.7, "G4", {}),
      ("the", 8.25, 0.25, 0.7, "G4", {}),
      ("sere", 8.5, 0.75, 0.8, "A4", {}),
      ("environment", 9.25, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("in", 10.75, 0.25, 0.7, "G4", {}),
      ("ecstatic", 11.0, 1.25, 1.0, [(0, "B4"), (0.5, "A4")], {"scoop": -1.5}),
      ("cosmogenic", 12.25, 1.5, 0.9, [(0, "A4"), (0.4, "G4"), (0.75, "F#4")], {}),
      ("comingling", 13.75, 2.25, 0.9, [(0, "F#4"), (0.5, "E4")],
       {"vib": (0.5, 5.7, 26), "fall": -1.0})]),
    PRECHORUS,
    CHORUS_A,
    CHORUS_B,
    # ---- paragraph five: the recitative bridge --------------------------
    (16, "From the core of this roiling thisMoment, vortices of "
         "transmomented potentiality upwell as spontaneous epiphanies.",
     [("From", 0.0, 0.5, 0.7, "E4", {}),
      ("the", 0.5, 0.5, 0.7, "E4", {}),
      ("core", 1.0, 1.5, 0.9, "E4", {"vib": (0.5, 5.7, 18)}),
      ("of", 2.5, 0.5, 0.7, "E4", {}),
      ("this", 3.0, 0.5, 0.7, "E4", {}),
      ("roiling", 3.5, 1.0, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("thisMoment", 4.5, 1.5, 0.9, [(0, "E4"), (0.5, "D4")], {}),
      ("vortices", 6.5, 1.5, 0.9, [(0, "D4"), (0.5, "E4")], {}),
      ("of", 8.0, 0.5, 0.7, "E4", {}),
      ("transmomented", 8.5, 1.5, 0.9, "E4", {}),
      ("potentiality", 10.0, 2.0, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("upwell", 12.0, 1.0, 0.9, [(0, "E4"), (0.5, "F#4")], {}),
      ("as", 13.0, 0.5, 0.7, "F#4", {}),
      ("spontaneous", 13.5, 1.25, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("epiphanies", 14.75, 1.25, 0.9, [(0, "F#4"), (0.5, "E4")], {})]),
    (32, "Decoherences of hyperplasmic experiencing that leave no trace "
         "on the scorched geometries, yet iridesce entire new craterous "
         "worldspaces into holographic izzyactualization through the "
         "scintellac prisming of their emergent offertory gyres.",
     [("Decoherences", 0.0, 1.5, 0.9, [(0, "E4"), (0.5, "D4")], {}),
      ("of", 1.5, 0.5, 0.7, "D4", {}),
      ("hyperplasmic", 2.0, 1.5, 0.9, [(0, "E4"), (0.5, "F#4")], {}),
      ("experiencing", 3.5, 1.5, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("that", 5.0, 0.5, 0.7, "E4", {}),
      ("leave", 5.5, 0.75, 0.8, "E4", {}),
      ("no", 6.25, 0.75, 0.8, "F#4", {}),
      ("trace", 7.0, 1.25, 0.9, "G4", {"vib": (0.5, 5.7, 22)}),
      ("on", 8.25, 0.25, 0.7, "F#4", {}),
      ("the", 8.5, 0.5, 0.7, "F#4", {}),
      ("scorched", 9.0, 1.0, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("geometries", 10.0, 1.75, 0.9, [(0, "E4"), (0.5, "D4")], {}),
      ("yet", 12.25, 0.5, 0.8, "E4", {}),
      ("iridesce", 12.75, 1.25, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
      ("entire", 14.0, 1.0, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("new", 15.0, 0.75, 0.8, "A4", {}),
      ("craterous", 15.75, 1.25, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("worldspaces", 17.0, 1.25, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("into", 18.25, 0.5, 0.7, "F#4", {}),
      ("holographic", 18.75, 1.25, 0.9, [(0, "G4"), (0.5, "A4")], {}),
      ("izzyactualization", 20.0, 2.5, 1.0,
       [(0, "A4"), (0.4, "B4"), (0.75, "A4")], {"vib": (0.6, 5.7, 28)}),
      ("through", 23.0, 0.5, 0.7, "G4", {}),
      ("the", 23.5, 0.5, 0.7, "G4", {}),
      ("scintellac", 24.0, 1.5, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("prisming", 25.5, 1.5, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("of", 27.0, 0.5, 0.7, "E4", {}),
      ("their", 27.5, 0.5, 0.7, "E4", {}),
      ("emergent", 28.0, 1.5, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
      ("offertory", 29.5, 1.5, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("gyres", 31.0, 1.0, 0.9, "E4", {"fall": -1.5})]),
    # ---- paragraph six: the ascent --------------------------------------
    (16, "I am no longer abeeing but a pure principle of poietic "
         "higherdimensional morphing.",
     [("I", 0.0, 0.5, 0.8, "B4", {}),
      ("am", 0.5, 0.5, 0.8, "A4", {}),
      ("no", 1.0, 1.0, 1.0, [(0, "E5"), (0.5, "D5")], {}),
      ("longer", 2.0, 1.0, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
      ("abeeing", 3.0, 2.0, 1.0, [(0, "B4"), (0.5, "C#5")],
       {"vib": (0.4, 5.7, 30)}),
      ("but", 5.5, 0.5, 0.7, "A4", {}),
      ("a", 6.0, 0.5, 0.7, "B4", {}),
      ("pure", 6.5, 1.5, 0.9, "C#5", {"vib": (0.5, 5.7, 28)}),
      ("principle", 8.0, 1.5, 0.9, [(0, "D5"), (0.4, "C#5"), (0.7, "B4")], {}),
      ("of", 9.5, 0.5, 0.7, "A4", {}),
      ("poietic", 10.0, 1.5, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
      ("higherdimensional", 11.5, 2.25, 1.0,
       [(0, "D5"), (0.3, "C#5"), (0.6, "B4"), (0.85, "A4")], {}),
      ("morphing", 13.75, 2.25, 1.0, [(0, "B4"), (0.5, "A4")],
       {"vib": (0.5, 5.7, 28), "fall": -1.0})]),
    (20, "Imbuing each sere grain with an erotic propulsor of "
         "pollenovective nevernotbecoming.",
     [("Imbuing", 0.0, 1.5, 0.9, [(0, "A4"), (0.5, "B4")], {}),
      ("each", 1.5, 0.75, 0.8, "A4", {}),
      ("sere", 2.25, 0.75, 0.8, "A4", {}),
      ("grain", 3.0, 1.5, 1.0, "B4", {"vib": (0.5, 5.7, 28)}),
      ("with", 4.5, 0.5, 0.7, "A4", {}),
      ("an", 5.0, 0.5, 0.7, "A4", {}),
      ("erotic", 5.5, 1.5, 0.9, [(0, "B4"), (0.5, "C#5")], {}),
      ("propulsor", 7.0, 1.5, 0.9, [(0, "C#5"), (0.5, "B4")], {}),
      ("of", 8.5, 0.5, 0.7, "A4", {}),
      ("pollenovective", 9.0, 1.5, 0.9,
       [(0, "B4"), (0.4, "C#5"), (0.75, "B4")], {}),
      ("nevernotbecoming", 10.5, 8.0, 1.0,
       [(0, "B4"), (0.18, "C#5"), (0.36, "D#5"), (0.54, "E5"),
        (0.72, "F#5"), (0.88, "G#5")],
       {"scoop": -1.5, "vib": (0.55, 5.6, 38), "fall": -2.0})]),
    # ---- the last verse, calm ------------------------------------------
    (16, "Every fleeting form resolidified from my starblazed "
         "starmeldmists of dissolution is already its own curvaceously "
         "ecstatic outbildung.",
     [("Every", 0.0, 0.75, 0.8, "E4", {}),
      ("fleeting", 0.75, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("form", 1.75, 1.25, 0.9, "G4", {"vib": (0.5, 5.7, 24)}),
      ("resolidified", 3.0, 1.75, 0.9, [(0, "A4"), (0.4, "G4"), (0.75, "A4")], {}),
      ("from", 4.75, 0.25, 0.7, "A4", {}),
      ("my", 5.0, 0.5, 0.7, "A4", {}),
      ("starblazed", 5.5, 1.25, 0.9, [(0, "B4"), (0.5, "A4")], {}),
      ("starmeldmists", 6.75, 1.5, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("of", 8.25, 0.25, 0.7, "G4", {}),
      ("dissolution", 8.5, 1.75, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("is", 10.25, 0.25, 0.7, "G4", {}),
      ("already", 10.5, 1.25, 0.8, [(0, "G4"), (0.5, "F#4")], {}),
      ("its", 11.75, 0.25, 0.7, "F#4", {}),
      ("own", 12.0, 0.75, 0.8, "G4", {}),
      ("curvaceously", 12.75, 1.25, 0.9, [(0, "A4"), (0.5, "G4")], {}),
      ("ecstatic", 14.0, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("outbildung", 15.0, 1.0, 0.9, [(0, "F#4"), (0.5, "E4")], {})]),
    (20, "Its ultimate essence an alchematrixyalizedrelic of my gesamt "
         "kiss, swallowed traceless but for the crisplicate sacrality "
         "of its transfixing allure.",
     [("Its", 0.0, 0.5, 0.7, "E4", {}),
      ("ultimate", 0.5, 1.0, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("essence", 1.5, 1.5, 0.9, [(0, "A4"), (0.5, "G4")],
       {"vib": (0.5, 5.7, 24)}),
      ("an", 3.0, 0.5, 0.7, "G4", {}),
      ("alchematrixyalizedrelic", 3.5, 2.5, 1.0,
       [(0, "A4"), (0.3, "B4"), (0.6, "A4"), (0.85, "G4")], {}),
      ("of", 6.0, 0.25, 0.7, "G4", {}),
      ("my", 6.25, 0.5, 0.7, "G4", {}),
      ("gesamt", 6.75, 1.0, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
      ("kiss", 7.75, 1.5, 1.0, "A4", {"vib": (0.5, 5.7, 26)}),
      ("swallowed", 9.75, 1.25, 0.9, [(0, "G4"), (0.5, "F#4")], {}),
      ("traceless", 11.0, 1.5, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("but", 12.5, 0.5, 0.7, "E4", {}),
      ("for", 13.0, 0.5, 0.7, "E4", {}),
      ("the", 13.5, 0.25, 0.7, "E4", {}),
      ("crisplicate", 13.75, 1.25, 0.9, [(0, "F#4"), (0.5, "G4")], {}),
      ("sacrality", 15.0, 1.5, 0.9, [(0, "G4"), (0.4, "F#4"), (0.75, "E4")], {}),
      ("of", 16.5, 0.5, 0.7, "E4", {}),
      ("its", 17.0, 0.5, 0.7, "E4", {}),
      ("transfixing", 17.5, 1.25, 0.9, [(0, "F#4"), (0.5, "E4")], {}),
      ("allure", 18.75, 1.25, 0.9, "E4", {"vib": (0.5, 5.7, 24), "fall": -1.5})]),
    AAH,
    BUZZ,
]


def parse(entry):
    if len(entry) == 4:
        return entry[0], entry[2], entry[3], True
    return entry[0], entry[1], entry[2], False


def synthesize(pipe, voice, text, speed):
    for r in pipe(text, voice=voice, speed=speed):
        audio = np.array(r.audio, dtype=np.float32).flatten()
        words = [(t.text, int(t.start_ts * RATE), int(t.end_ts * RATE))
                 for t in r.tokens
                 if t.phonemes and any(c.isalnum() for c in t.text)
                 and t.start_ts is not None]
        return audio, words
    raise RuntimeError(f"no audio for {text!r}")


def word_knots(audio, s0, s1, o0, o1):
    step = int(0.010 * RATE)
    n = max(1, (s1 - s0) // step)
    bounds = np.linspace(s0, s1, n + 1).astype(int)
    w = np.array([np.sqrt((audio[a:b] ** 2).mean() + 1e-9)
                  for a, b in zip(bounds[:-1], bounds[1:])])
    alloc = np.concatenate([[0.0], np.cumsum(w / w.sum())]) * (o1 - o0) + o0
    return list(zip(alloc.astype(int), bounds))


def wsola(audio, knots, out_len, win=600, hop=150, search=200):
    ko = np.array([k[0] for k in knots], dtype=np.float64)
    ks = np.array([k[1] for k in knots], dtype=np.float64)
    src = np.pad(audio, (0, 2 * win + search))
    w = np.hanning(win).astype(np.float32)
    y = np.zeros(out_len + win, dtype=np.float32)
    wsum = np.zeros(out_len + win, dtype=np.float32)
    prev = None
    for t in range(0, out_len, hop):
        target = np.interp(t, ko, ks)
        if prev is None:
            pos = int(target)
        else:
            ref = src[prev + hop:prev + hop + win]
            lo = max(0, int(target) - search)
            cands = src[lo:int(target) + search + win]
            corr = np.correlate(cands, ref, "valid")
            pos = lo + int(np.argmax(corr))
        y[t:t + win] += src[pos:pos + win] * w
        wsum[t:t + win] += w
        prev = pos
    return y[:out_len] / np.maximum(wsum[:out_len], 1e-3)


def varispeed(audio, factor):
    idx = np.arange(0, len(audio) - 1, factor)
    i0 = idx.astype(int)
    frac = (idx - i0).astype(np.float32)
    return audio[i0] * (1 - frac) + audio[i0 + 1] * frac


def build_pitch(spb, total_samples):
    rng = np.random.default_rng(11)
    bp = []
    prev = None
    base = 0.0
    voc_spans = []
    for entry in SCORE:
        slot_beats, _text, timing, is_voc = parse(entry)
        if is_voc:
            voc_spans.append((base, base + slot_beats * spb))
        for word, onset, length, _vel, pitch, opts in timing:
            t0, t1 = base + onset * spb, base + (onset + length) * spb
            way = [(0.0, N[pitch])] if isinstance(pitch, str) else \
                  [(f, N[n]) for f, n in pitch]
            if prev is None:
                prev = way[0][1]
            glide = opts.get("glide", 0.06 if way[0][1] != prev else 0.03)
            bp.append((t0, prev))
            bp.append((t0 + glide, way[0][1] + opts.get("scoop", 0.0)))
            if "scoop" in opts:
                bp.append((t0 + glide + 0.10, way[0][1]))
            for f, m in way[1:]:
                tw = t0 + f * (t1 - t0)
                bp.append((tw - 0.02, bp[-1][1]))
                bp.append((tw + 0.02, m))
            if "fall" in opts:
                bp.append((t1 - 0.10, way[-1][1]))
                bp.append((t1, way[-1][1] + opts["fall"]))
            else:
                bp.append((t1, way[-1][1]))
            prev = way[-1][1]
        base += slot_beats * spb
    times = np.array([t for t, _ in bp]) * RATE
    vals = np.array([m for _, m in bp], dtype=np.float64)
    midi = np.interp(np.arange(total_samples), times, vals)

    k = int(0.015 * RATE)
    kernel = np.ones(k) / k
    for _ in range(2):
        midi = np.convolve(np.pad(midi, k, mode="edge"), kernel, "same")[k:-k]

    base = 0.0
    for entry in SCORE:
        slot_beats, _text, timing, _is_voc = parse(entry)
        for word, onset, length, _vel, pitch, opts in timing:
            if "vib" not in opts:
                continue
            delay_f, rate_hz, cents = opts["vib"]
            t0 = base + (onset + delay_f * length) * spb
            t1 = base + (onset + length) * spb
            a, b = int(t0 * RATE), min(int(t1 * RATE), total_samples)
            if b <= a:
                continue
            tt = np.arange(b - a) / RATE
            drift = 1.0 + 0.04 * np.sin(2 * np.pi * 0.7 * tt + rng.uniform(0, 6))
            phase = np.cumsum(rate_hz * drift) / RATE
            ramp = np.minimum(1.0, tt / max(1e-6, 0.25 * (t1 - t0)))
            midi[a:b] += (cents / 100.0) * ramp * np.sin(2 * np.pi * phase)
        base += slot_beats * spb

    noise = rng.standard_normal(total_samples // 1200 + 2)
    slow = np.interp(np.arange(total_samples), np.arange(len(noise)) * 1200, noise)
    midi += 0.04 * slow

    for a, b in voc_spans:
        midi[int(a * RATE):int(b * RATE)] = 0.0
    return midi.astype(np.float32)


def curve_only():
    """Rebuild pitch + json against the existing line wav — melody
    edits never touch the speech, so skip the whole TTS pass."""
    import json
    spb = 60.0 / BPM
    line, _ = sf.read(os.path.join(REPO, "renders", "nevernot-line.wav"),
                      dtype="float32")
    pitch = build_pitch(spb, len(line))
    sf.write(os.path.join(REPO, "renders", "nevernot-pitch.wav"),
             pitch, RATE, subtype="FLOAT")
    dump, base = [], 0.0
    for entry in SCORE:
        slot_beats, text, timing, is_voc = parse(entry)
        dump.append({
            "start": base, "dur": slot_beats * spb, "vocoder": is_voc,
            "text": text,
            "words": [{"w": w, "t": base + o * spb, "dur": l * spb, "vel": v}
                      for w, o, l, v, _p, _x in timing]})
        base += slot_beats * spb
    with open(os.path.join(REPO, "renders", "nevernot-score.json"), "w") as f:
        json.dump({"spb": spb, "intro": 8 * spb, "phrases": dump}, f, indent=1)
    print(f"curve rebuilt ({pitch.min():.1f}..{pitch.max():.1f} MIDI)")


def main():
    from mlx_audio.tts.utils import load_model
    from mlx_audio.tts.models.kokoro import KokoroPipeline
    import mlx.core as mx

    # MLX's Metal buffer cache grows across the ~70 generate calls this
    # build makes and never shrinks on its own — on an 8 GB machine
    # that's a swap storm. Cap it, and clear it between phrases.
    try:
        mx.set_cache_limit(1 << 30)
    except AttributeError:
        pass

    os.environ.setdefault("VIRTUAL_ENV", os.path.join(REPO, ".venv-voice"))
    model = load_model("mlx-community/Kokoro-82M-bf16")
    pipe = KokoroPipeline(lang_code="a", model=model,
                          repo_id="mlx-community/Kokoro-82M-bf16")

    spb = 60.0 / BPM
    ramp = int(0.030 * RATE)
    kernel = np.hanning(2 * ramp + 1)
    kernel /= kernel.sum()
    chunks = []
    for entry in SCORE:
        slot_beats, text, timing, _is_voc = parse(entry)
        slot = int(round(slot_beats * spb * RATE))
        t_end = timing[-1][1] + timing[-1][2]
        assert t_end <= slot_beats, (text[:30], t_end, slot_beats)
        out_len = int(slot * LIFT)
        layers = []
        for voice, gain in (CHOIR if _is_voc else LEAD):
            audio, words = synthesize(pipe, voice, text, 1.0)
            target = 0.8 * t_end * spb * LIFT
            speed = max(0.8, min(1.2, len(audio) / RATE / target))
            if abs(speed - 1.0) > 0.05:
                audio, words = synthesize(pipe, voice, text, speed)
            assert len(words) == len(timing), (
                f"{voice} {text!r}: {len(words)} words vs {len(timing)}\n"
                f"  heard: {[w for w, _, _ in words]}")
            knots = [(0, max(0, words[0][1]
                             - int(timing[0][1] * spb * RATE * LIFT)))]
            for (word, s0, s1), (name, onset, length, _v, _p, _o) in \
                    zip(words, timing):
                assert word.lower().strip(".,!?") == \
                    name.lower().strip(".,!?"), (word, name)
                o0 = int(onset * spb * RATE * LIFT)
                o1 = int((onset + length) * spb * RATE * LIFT)
                knots += word_knots(audio, s0, s1, o0, o1)
            knots.append((out_len, min(len(audio),
                                       knots[-1][1] + out_len - knots[-1][0])))
            layer = varispeed(wsola(audio, knots, out_len), LIFT)
            layer = np.pad(layer[:slot], (0, max(0, slot - len(layer))))
            layers.append(layer * gain)
        phrase = np.sum(layers, axis=0)
        even = np.ones(slot, dtype=np.float32)
        rms_all = [np.sqrt((phrase[int(o * spb * RATE):
                                   int((o + l) * spb * RATE)] ** 2).mean() + 1e-9)
                   for _n, o, l, _v, _p, _x in timing]
        target_rms = float(np.median(rms_all))
        for (name, onset, length, _v, _p, _x), wr in zip(timing, rms_all):
            a, b = int(onset * spb * RATE), min(int((onset + length) * spb * RATE), slot)
            even[a:b] = np.clip((target_rms / wr) ** 0.7, 0.5, 2.5)
        env = np.full(slot, 0.75, dtype=np.float32)
        for name, onset, length, vel, _p, _x in timing:
            a, b = int(onset * spb * RATE), int((onset + length) * spb * RATE)
            env[a:min(b, slot)] = vel
        shaped = np.convolve(even * env, kernel, "same")
        phrase *= shaped
        if _is_voc:
            phrase *= 0.62 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
            phrase = np.tanh(phrase / 0.65) * 0.65
        else:
            phrase *= 0.12 / (np.sqrt((phrase ** 2).mean()) + 1e-9)
        print(f"  ok  {text[:64]}")
        chunks.append(phrase.astype(np.float32))
        try:
            mx.clear_cache()
        except AttributeError:
            pass

    line = np.concatenate(chunks)
    line *= 0.9 / np.abs(line).max()
    sf.write(os.path.join(REPO, "renders", "nevernot-line.wav"), line, RATE)
    pitch = build_pitch(spb, len(line))
    sf.write(os.path.join(REPO, "renders", "nevernot-pitch.wav"),
             pitch, RATE, subtype="FLOAT")

    import json
    dump, base = [], 0.0
    for entry in SCORE:
        slot_beats, text, timing, is_voc = parse(entry)
        dump.append({
            "start": base, "dur": slot_beats * spb, "vocoder": is_voc,
            "text": text,
            "words": [{"w": w, "t": base + o * spb, "dur": l * spb, "vel": v}
                      for w, o, l, v, _p, _x in timing]})
        base += slot_beats * spb
    with open(os.path.join(REPO, "renders", "nevernot-score.json"), "w") as f:
        json.dump({"spb": spb, "intro": 8 * spb, "phrases": dump}, f, indent=1)
    print(f"wrote line ({len(line) / RATE:.1f}s) + pitch + score json")


if __name__ == "__main__":
    import sys
    if "--curve-only" in sys.argv:
        curve_only()
    else:
        main()
