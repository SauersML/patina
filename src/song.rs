// src/song.rs
//
// A tiny text-based song format and player, used via `patina --play <file>`.
// Notes and parameter automation go through the VoiceManager exactly as if
// they came from the on-screen keyboard, a MIDI device, or the UI sliders.
//
// Format (one directive or a run of event tokens per line, `#` starts a comment):
//
//   bpm 100                  # global tempo (set once, at the top)
//   gate 0.85                # fraction of each note's duration it is held (default 0.9)
//
//   track lead vel=0.9 len=0.5   # start a note track; tracks play in parallel.
//                                # vel = default velocity (0..1)
//                                # len = default token duration in beats (default 1)
//     E5:2 D5 C5 R:4 [C4 E4 G4]:2@0.6  | A4
//
// Note-track tokens:
//   C4  F#3  Eb5  60      note names (C4 = MIDI 60) or raw MIDI numbers
//   [C4 E4 G4]            chord (notes start and stop together)
//   R  or  .              rest
//   :2                    duration suffix, in beats (floats allowed)
//   @0.7                  velocity suffix (0..1)
//   |                     bar line, ignored (readability only)
//
// Automation tracks ramp a synth parameter through breakpoints:
//
//   automate cutoff
//     400 8000:16@exp R:8 400:4@smooth
//
//   The first token must be a plain value (the starting point). After that,
//   V:D@shape means "ramp to V over D beats". R:D / .:D holds the current
//   value. Shapes: lin (default), exp (musical/geometric — right for
//   frequencies), log (fast start), smooth (S-curve), step (jump at the end).
//
// Automatable parameters: volume, cutoff, resonance, drive, saturation,
// attack, decay, sustain, release, reverb_decay, reverb_wet, chorus_rate,
// chorus_depth.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::voice_manager::VoiceManager;

// Automation curves are sampled at this many points per beat
const AUTOMATION_STEPS_PER_BEAT: f64 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    Volume,
    Cutoff,
    Resonance,
    Drive,
    Saturation,
    Attack,
    Decay,
    Sustain,
    Release,
    ReverbDecay,
    ReverbWet,
    ChorusRate,
    ChorusDepth,
}

impl Param {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "volume" => Param::Volume,
            "cutoff" => Param::Cutoff,
            "resonance" => Param::Resonance,
            "drive" => Param::Drive,
            "saturation" => Param::Saturation,
            "attack" => Param::Attack,
            "decay" => Param::Decay,
            "sustain" => Param::Sustain,
            "release" => Param::Release,
            "reverb_decay" => Param::ReverbDecay,
            "reverb_wet" => Param::ReverbWet,
            "chorus_rate" => Param::ChorusRate,
            "chorus_depth" => Param::ChorusDepth,
            _ => return None,
        })
    }

    fn apply(self, vm: &mut VoiceManager, value: f32) {
        match self {
            Param::Volume => vm.set_volume(value),
            Param::Cutoff => vm.set_filter_cutoff(value),
            Param::Resonance => vm.set_filter_resonance(value),
            Param::Drive => vm.set_filter_drive(value),
            Param::Saturation => vm.set_filter_saturation(value),
            Param::Attack => vm.set_attack(value),
            Param::Decay => vm.set_decay(value),
            Param::Sustain => vm.set_sustain(value),
            Param::Release => vm.set_release(value),
            Param::ReverbDecay => vm.set_reverb_decay(value),
            Param::ReverbWet => vm.set_reverb_wet(value),
            Param::ChorusRate => vm.set_chorus_rate(value),
            Param::ChorusDepth => vm.set_chorus_depth(value),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Lin,
    Exp,
    Log,
    Smooth,
    Step,
}

impl Shape {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "lin" => Shape::Lin,
            "exp" => Shape::Exp,
            "log" => Shape::Log,
            "smooth" => Shape::Smooth,
            "step" => Shape::Step,
            _ => return None,
        })
    }

    fn interpolate(self, from: f32, to: f32, t: f32) -> f32 {
        let eased = match self {
            Shape::Lin => t,
            // Geometric interpolation for positive endpoints (perceptually even
            // for frequencies); fall back to an ease-in power curve otherwise
            Shape::Exp => {
                if from > 0.0 && to > 0.0 {
                    return from * (to / from).powf(t);
                }
                t * t
            }
            Shape::Log => 1.0 - (1.0 - t) * (1.0 - t),
            Shape::Smooth => t * t * (3.0 - 2.0 * t),
            Shape::Step => return if t >= 1.0 { to } else { from },
        };
        from + (to - from) * eased
    }
}

#[derive(Debug)]
pub enum EventKind {
    NoteOn { note: u8, velocity: f32 },
    NoteOff { note: u8 },
    Param { param: Param, value: f32 },
}

pub struct SongEvent {
    time: f64, // seconds from song start
    kind: EventKind,
}

pub fn load_song(path: &str) -> Result<Vec<SongEvent>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read song file '{}': {}", path, e))?;
    parse_song(&text)
}

pub fn spawn_player(events: Vec<SongEvent>, voice_manager: Arc<Mutex<VoiceManager>>) {
    thread::spawn(move || {
        // Let the audio stream and window settle before the downbeat
        thread::sleep(Duration::from_millis(1200));
        println!("Song: playing {} events", events.len());

        let start = Instant::now();
        for event in &events {
            let target = Duration::from_secs_f64(event.time);
            if let Some(wait) = target.checked_sub(start.elapsed()) {
                thread::sleep(wait);
            }
            let mut vm = voice_manager.lock();
            match event.kind {
                EventKind::NoteOn { note, velocity } => vm.note_on(note, velocity),
                EventKind::NoteOff { note } => vm.note_off(note),
                EventKind::Param { param, value } => param.apply(&mut vm, value),
            }
        }
        println!("Song: finished");
    });
}

enum TrackMode {
    None,
    Notes { vel: f32, len: f64 },
    Automation { param: Param, current: Option<f32> },
}

// (beats, order-rank, kind); rank makes offs < params < ons at equal times
type RawEvent = (f64, u8, EventKind);

fn parse_song(text: &str) -> Result<Vec<SongEvent>, String> {
    let mut bpm = 120.0_f64;
    let mut gate = 0.9_f64;
    let mut events: Vec<RawEvent> = Vec::new();

    let mut mode = TrackMode::None;
    let mut track_beat = 0.0_f64;

    for (line_no, raw) in text.lines().enumerate() {
        let err = |msg: String| format!("line {}: {}", line_no + 1, msg);
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        let first = line.split_whitespace().next().unwrap();
        match first {
            "bpm" => {
                bpm = line[3..].trim().parse::<f64>().map_err(|_| err("invalid bpm".into()))?;
                if bpm <= 0.0 {
                    return Err(err("bpm must be positive".into()));
                }
            }
            "gate" => {
                gate = line[4..].trim().parse::<f64>().map_err(|_| err("invalid gate".into()))?;
                gate = gate.clamp(0.05, 1.0);
            }
            "track" => {
                track_beat = 0.0;
                let mut vel = 0.8_f32;
                let mut len = 1.0_f64;
                for opt in line.split_whitespace().skip(2) {
                    if let Some(v) = opt.strip_prefix("vel=") {
                        vel = v.parse::<f32>().map_err(|_| err(format!("invalid vel '{}'", v)))?;
                    } else if let Some(v) = opt.strip_prefix("len=") {
                        len = v.parse::<f64>().map_err(|_| err(format!("invalid len '{}'", v)))?;
                    } else {
                        return Err(err(format!("unknown track option '{}'", opt)));
                    }
                }
                mode = TrackMode::Notes { vel, len };
            }
            "automate" => {
                let name = line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| err("automate needs a parameter name".into()))?;
                let param = Param::from_name(name)
                    .ok_or_else(|| err(format!("unknown parameter '{}'", name)))?;
                track_beat = 0.0;
                mode = TrackMode::Automation { param, current: None };
            }
            _ => match &mut mode {
                TrackMode::None => {
                    return Err(err("event tokens before any 'track' or 'automate' line".into()));
                }
                TrackMode::Notes { vel, len } => {
                    let (vel, len) = (*vel, *len);
                    for token in tokenize(line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        let (notes, dur, vel) = parse_note_token(&token, vel, len)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        let off_beat = track_beat + dur * gate;
                        for &note in &notes {
                            events.push((track_beat, 2, EventKind::NoteOn { note, velocity: vel }));
                            events.push((off_beat, 0, EventKind::NoteOff { note }));
                        }
                        track_beat += dur;
                    }
                }
                TrackMode::Automation { param, current } => {
                    let param = *param;
                    for token in tokenize(line).map_err(err)? {
                        if token == "|" {
                            continue;
                        }
                        let seg = parse_automation_token(&token)
                            .map_err(|m| err(format!("token '{}': {}", token, m)))?;
                        match seg {
                            AutoToken::Hold(dur) => track_beat += dur,
                            AutoToken::Set(value) => {
                                events.push((track_beat, 1, EventKind::Param { param, value }));
                                *current = Some(value);
                            }
                            AutoToken::Ramp { to, dur, shape } => {
                                let from = current.ok_or_else(|| {
                                    err(format!(
                                        "token '{}': first token of an automate track must be a plain starting value",
                                        token
                                    ))
                                })?;
                                emit_ramp(&mut events, param, from, to, track_beat, dur, shape);
                                *current = Some(to);
                                track_beat += dur;
                            }
                        }
                    }
                }
            },
        }
    }

    if events.is_empty() {
        return Err("song contains no events".into());
    }

    events.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    let secs_per_beat = 60.0 / bpm;
    Ok(events
        .into_iter()
        .map(|(beats, _, kind)| SongEvent { time: beats * secs_per_beat, kind })
        .collect())
}

fn emit_ramp(
    events: &mut Vec<RawEvent>,
    param: Param,
    from: f32,
    to: f32,
    start_beat: f64,
    dur: f64,
    shape: Shape,
) {
    if matches!(shape, Shape::Step) || from == to {
        events.push((start_beat + dur, 1, EventKind::Param { param, value: to }));
        return;
    }
    let steps = ((dur * AUTOMATION_STEPS_PER_BEAT).ceil() as usize).clamp(1, 4096);
    for k in 1..=steps {
        let t = k as f64 / steps as f64;
        let value = shape.interpolate(from, to, t as f32);
        events.push((start_beat + dur * t, 1, EventKind::Param { param, value }));
    }
}

enum AutoToken {
    Set(f32),
    Hold(f64),
    Ramp { to: f32, dur: f64, shape: Shape },
}

/// Parse one automation token: `V`, `V:D`, `V:D@shape`, or `R:D` / `.:D`.
fn parse_automation_token(token: &str) -> Result<AutoToken, String> {
    let mut s = token;
    let mut shape = Shape::Lin;
    let mut dur: Option<f64> = None;

    if let Some(i) = s.rfind('@') {
        let name = &s[i + 1..];
        shape = Shape::from_name(name).ok_or_else(|| format!("unknown shape '{}'", name))?;
        s = &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        let d = s[i + 1..].parse::<f64>().map_err(|_| "invalid duration".to_string())?;
        if d <= 0.0 {
            return Err("duration must be positive".into());
        }
        dur = Some(d);
        s = &s[..i];
    }

    if s == "." || s.eq_ignore_ascii_case("r") {
        return Ok(AutoToken::Hold(dur.ok_or("hold needs a duration, e.g. R:4")?));
    }

    let value = s.parse::<f32>().map_err(|_| "invalid value".to_string())?;
    match dur {
        Some(dur) => Ok(AutoToken::Ramp { to: value, dur, shape }),
        None => Ok(AutoToken::Set(value)),
    }
}

/// Split a line into tokens, keeping bracketed chords together.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;

    for c in line.chars() {
        match c {
            '[' => {
                depth += 1;
                current.push(c);
            }
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err("unmatched ']'".into());
                }
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if depth != 0 {
        return Err("unmatched '['".into());
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// Parse one note-track token into (notes, duration-in-beats, velocity).
/// An empty notes list is a rest.
fn parse_note_token(token: &str, default_vel: f32, default_len: f64) -> Result<(Vec<u8>, f64, f32), String> {
    let mut s = token;
    let mut vel = default_vel;
    let mut dur = default_len;

    if let Some(i) = s.rfind('@') {
        vel = s[i + 1..].parse::<f32>().map_err(|_| "invalid velocity".to_string())?;
        s = &s[..i];
    }
    if let Some(i) = s.rfind(':') {
        dur = s[i + 1..].parse::<f64>().map_err(|_| "invalid duration".to_string())?;
        s = &s[..i];
    }
    if dur <= 0.0 {
        return Err("duration must be positive".into());
    }

    let notes = if s == "." || s.eq_ignore_ascii_case("r") {
        Vec::new()
    } else if let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        inner
            .split_whitespace()
            .map(parse_note)
            .collect::<Result<Vec<u8>, String>>()?
    } else {
        vec![parse_note(s)?]
    };

    Ok((notes, dur, vel.clamp(0.0, 1.0)))
}

/// Parse a note name like C4, F#3, Eb5 (C4 = MIDI 60), or a raw MIDI number.
fn parse_note(s: &str) -> Result<u8, String> {
    if s.chars().all(|c| c.is_ascii_digit()) {
        let n = s.parse::<u8>().map_err(|_| format!("invalid MIDI number '{}'", s))?;
        if n > 127 {
            return Err(format!("MIDI number {} out of range", n));
        }
        return Ok(n);
    }

    let mut chars = s.chars();
    let letter = chars.next().ok_or("empty note")?;
    let mut semitone: i32 = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        other => return Err(format!("invalid note letter '{}'", other)),
    };

    let rest: String = chars.collect();
    let mut rest = rest.as_str();
    while let Some(r) = rest.strip_prefix('#') {
        semitone += 1;
        rest = r;
    }
    while let Some(r) = rest.strip_prefix('b') {
        semitone -= 1;
        rest = r;
    }

    let octave = rest
        .parse::<i32>()
        .map_err(|_| format!("invalid octave '{}'", rest))?;
    let midi = (octave + 1) * 12 + semitone;
    if !(0..=127).contains(&midi) {
        return Err(format!("note '{}' out of MIDI range", s));
    }
    Ok(midi as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(parse_note("C4").unwrap(), 60);
        assert_eq!(parse_note("A4").unwrap(), 69);
        assert_eq!(parse_note("F#3").unwrap(), 54);
        assert_eq!(parse_note("Eb5").unwrap(), 75);
        assert_eq!(parse_note("60").unwrap(), 60);
        assert!(parse_note("H4").is_err());
        assert!(parse_note("C99").is_err());
    }

    #[test]
    fn note_tokens() {
        let (notes, dur, vel) = parse_note_token("C4:2@0.7", 0.8, 1.0).unwrap();
        assert_eq!(notes, vec![60]);
        assert_eq!(dur, 2.0);
        assert_eq!(vel, 0.7);

        let (notes, dur, _) = parse_note_token("[C4 E4 G4]:0.5", 0.8, 1.0).unwrap();
        assert_eq!(notes, vec![60, 64, 67]);
        assert_eq!(dur, 0.5);

        let (notes, dur, _) = parse_note_token("R:4", 0.8, 1.0).unwrap();
        assert!(notes.is_empty());
        assert_eq!(dur, 4.0);

        // default duration comes from the track's len option
        let (_, dur, _) = parse_note_token("C4", 0.8, 0.5).unwrap();
        assert_eq!(dur, 0.5);
    }

    #[test]
    fn full_song() {
        let events = parse_song("bpm 120\ntrack a vel=0.9\nC4 E4:1 | R:2 [C3 G3]:2\n").unwrap();
        // 4 sounding notes -> 8 events (on + off each)
        assert_eq!(events.len(), 8);
        assert_eq!(events[0].time, 0.0);
        assert!(matches!(events[0].kind, EventKind::NoteOn { note: 60, .. }));
        // chord starts after 1 + 1 + 2 beats = 2.0 s at 120 bpm
        let chord_on = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::NoteOn { .. }) && e.time == 2.0)
            .count();
        assert_eq!(chord_on, 2);
    }

    #[test]
    fn automation() {
        let events =
            parse_song("bpm 60\ntrack a\nC4:8\nautomate cutoff\n400 R:2 8000:4@exp\n").unwrap();
        let params: Vec<(f64, f32)> = events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::Param { param: Param::Cutoff, value } => Some((e.time, value)),
                _ => None,
            })
            .collect();
        // initial set at t=0, then 4 beats * 32 steps of ramp
        assert_eq!(params.len(), 1 + 128);
        assert_eq!(params[0], (0.0, 400.0));
        // ramp starts after the 2-beat hold (t=2s at 60 bpm) and ends at t=6s
        assert!(params[1].0 > 2.0);
        let last = params.last().unwrap();
        assert_eq!(last.0, 6.0);
        assert!((last.1 - 8000.0).abs() < 0.5);
        // geometric ramp is monotonically increasing
        assert!(params.windows(2).all(|w| w[1].1 > w[0].1));
    }

    #[test]
    fn automation_errors() {
        // ramp before a starting value
        assert!(parse_song("automate cutoff\n8000:4@exp\n").is_err());
        // unknown parameter and unknown shape
        assert!(parse_song("automate flanger\n1 2:1\n").is_err());
        assert!(parse_song("automate cutoff\n400 800:4@bounce\n").is_err());
    }

    #[test]
    fn bundled_song_parses() {
        let text = include_str!("../songs/nightdrive.song");
        let events = parse_song(text).unwrap();
        assert!(!events.is_empty());
        for pair in events.windows(2) {
            assert!(pair[0].time <= pair[1].time);
        }
    }

    #[test]
    fn parse_errors() {
        assert!(parse_song("track a\nnot_a_note\n").is_err());
        assert!(parse_song("C4\n").is_err()); // notes before any track
        assert!(parse_song("bpm 100\n").is_err()); // no events at all
    }
}
