// src/midi_file.rs
//
// Standard MIDI File import: `patina --play tune.mid [--patch NAME]`. A .mid
// becomes an ordinary Song — the same timed NoteOn/NoteOff/Param events a
// .song file parses to — so live play, --render, --render-stems and
// --export-events all take MIDI files through song::load_song. The live-input
// conventions of src/midi_handler.rs carry over: GM channel 10 is the 909
// board (one-shots, no note-offs), a velocity-0 note-on is a note-off, CCs go
// through Param::from_cc, and the pitch wheel spans +/-2 semitones.

use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

use crate::song::{EventKind, Param, Song, SongEvent};
use crate::voice_manager::ParamValues;

/// GM channel 10, 0-indexed.
const GM_DRUMS: u8 = 9;

/// Read a .mid file; every melodic MIDI channel plays `patch`
/// (`patches/NAME.patch`), or Init when none is named.
pub fn load(path: &str, patch: Option<&str>) -> Result<Song, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read MIDI file '{}': {}", path, e))?;
    let text = match patch {
        Some(name) => crate::song::read_patch_file(name)?,
        None => crate::patch::FACTORY[0].1.to_string(),
    };
    let voice = crate::song::params_from_patch(&text)?;
    let mut song = parse_midi(&bytes, voice).map_err(|e| format!("MIDI file '{}': {}", path, e))?;
    if patch.is_some() {
        song.events.splice(0..0, bus_settings(&text)?);
    }
    Ok(song)
}

/// One named patch plays the whole file, so it is the whole instrument —
/// what clicking it on the panel does: the bus half (tape, rooms, chorus,
/// width...) lands at the downbeat along with the voice. Which half a
/// parameter belongs to is the song parser's own rule (`reaches_track`);
/// volume stays out, since songs never take a patch's level.
fn bus_settings(patch: &str) -> Result<Vec<SongEvent>, String> {
    Ok(crate::song::patch_lines(patch)?
        .into_iter()
        .filter(|&(p, _)| p != Param::Volume && !p.reaches_track(1))
        .map(|(param, value)| SongEvent {
            time: 0.0,
            kind: EventKind::Param {
                param,
                value,
                channel: 0,
            },
        })
        .collect())
}

/// Parse SMF bytes (type 0 or 1) into a Song whose melodic channels each
/// get a private song channel seeded with `voice`.
pub fn parse_midi(bytes: &[u8], voice: ParamValues) -> Result<Song, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a readable MIDI file: {}", e))?;
    if smf.header.format == Format::Sequential {
        return Err("type 2 (sequential) MIDI files are not supported".into());
    }

    // Merge every track into one absolute-tick stream. Tracks go in file
    // order and the sort is stable, so equal ticks keep file order: an off
    // written before a re-strike of the same key still lands first.
    let mut merged: Vec<(u64, u8, MidiMessage)> = Vec::new();
    let mut tempos: Vec<(u64, u32)> = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Midi { channel, message } => {
                    merged.push((tick, channel.as_int(), message))
                }
                TrackEventKind::Meta(MetaMessage::Tempo(us)) => tempos.push((tick, us.as_int())),
                _ => {}
            }
        }
    }
    merged.sort_by_key(|e| e.0);
    let clock = TickClock::new(smf.header.timing, tempos)?;

    let mut map = ChannelMap {
        voice,
        ids: [None; 16],
        channels: Vec::new(),
        tracks: Vec::new(),
    };
    let mut events = Vec::with_capacity(merged.len());
    for (tick, ch, message) in merged {
        let drums = ch == GM_DRUMS;
        let kind = match message {
            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => EventKind::NoteOn {
                note: key.as_int(),
                velocity: vel.as_int() as f32 / 127.0,
                channel: map.get(ch),
            },
            // MIDI spec: Note On with velocity 0 is a Note Off. Drum voices
            // are one-shots; the 909 trigger has no falling edge.
            MidiMessage::NoteOn { .. } | MidiMessage::NoteOff { .. } if drums => continue,
            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                EventKind::NoteOff {
                    note: key.as_int(),
                    channel: map.get(ch),
                }
            }
            MidiMessage::Controller { controller, value } => {
                let Some(param) = Param::from_cc(controller.as_int()) else {
                    continue;
                };
                EventKind::Param {
                    param,
                    value: param.midi_value(value.as_int() as f32 / 127.0),
                    channel: map.get(ch),
                }
            }
            // Pitch wheel: midly gives -1..1, standard range +/-2 semitones.
            MidiMessage::PitchBend { bend } => EventKind::Param {
                param: Param::PitchBendSemis,
                value: bend.as_f32() * 2.0,
                channel: map.get(ch),
            },
            // Program changes are left to --patch: the file picks notes,
            // the player picks the voice.
            _ => continue,
        };
        events.push(SongEvent {
            time: clock.seconds(tick),
            kind,
        });
    }

    Ok(Song {
        events,
        tail_seconds: crate::song::DEFAULT_TAIL_SECONDS,
        channels: map.channels,
        vox_wav: None,
        vox_pitch: None,
        samplers: Vec::new(),
        tracks: map.tracks,
    })
}

/// MIDI channel -> song channel, allocated on first use: channel 10 is the
/// drum board, every other channel a private song channel (like a bare
/// `track` in a .song) carrying the chosen voice.
struct ChannelMap {
    voice: ParamValues,
    ids: [Option<u16>; 16],
    channels: Vec<ParamValues>,
    tracks: Vec<(String, u16)>,
}

impl ChannelMap {
    fn get(&mut self, ch: u8) -> u16 {
        if let Some(id) = self.ids[ch as usize] {
            return id;
        }
        let (name, id) = if ch == GM_DRUMS {
            ("drums".to_string(), crate::drums::DRUM_CHANNEL)
        } else {
            self.channels.push(self.voice);
            (format!("ch{}", ch + 1), self.channels.len() as u16)
        };
        self.tracks.push((name, id));
        self.ids[ch as usize] = Some(id);
        id
    }
}

/// Absolute tick -> seconds. Metrical time integrates the tempo map
/// exactly (120 BPM until the first Tempo event, from any track); SMPTE
/// timecode ticks are a fixed fraction of a second.
struct TickClock {
    /// (start tick, seconds at start, seconds per tick), ascending
    segments: Vec<(u64, f64, f64)>,
}

impl TickClock {
    fn new(timing: Timing, mut tempos: Vec<(u64, u32)>) -> Result<Self, String> {
        let per_beat = match timing {
            Timing::Metrical(ppq) if ppq.as_int() > 0 => ppq.as_int() as f64,
            Timing::Metrical(_) => return Err("the header declares 0 ticks per beat".into()),
            Timing::Timecode(fps, sub) if sub > 0 => {
                let per_tick = 1.0 / (fps.as_f32() as f64 * sub as f64);
                return Ok(Self {
                    segments: vec![(0, 0.0, per_tick)],
                });
            }
            Timing::Timecode(..) => return Err("the header declares 0 ticks per frame".into()),
        };
        // Stable: of several tempos on one tick, the last written wins
        tempos.sort_by_key(|t| t.0);
        let mut segments = vec![(0u64, 0.0f64, 0.5 / per_beat)];
        for (tick, us_per_beat) in tempos {
            let per_tick = us_per_beat as f64 * 1e-6 / per_beat;
            let &(t0, s0, rate) = segments.last().unwrap();
            if tick == t0 {
                segments.last_mut().unwrap().2 = per_tick;
            } else {
                segments.push((tick, s0 + (tick - t0) as f64 * rate, per_tick));
            }
        }
        Ok(Self { segments })
    }

    fn seconds(&self, tick: u64) -> f64 {
        let i = self.segments.partition_point(|s| s.0 <= tick) - 1;
        let (t0, s0, rate) = self.segments[i];
        s0 + (tick - t0) as f64 * rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midly::num::{u15, u28, u4, u7};
    use midly::{Fps, Header, PitchBend, TrackEvent};

    type Ev = (u32, TrackEventKind<'static>);

    /// Assemble an SMF with midly's writer: (delta ticks, event) per track.
    fn smf(format: Format, timing: Timing, tracks: Vec<Vec<Ev>>) -> Vec<u8> {
        let mut file = Smf::new(Header::new(format, timing));
        for track in tracks {
            let mut t: Vec<TrackEvent> = track
                .into_iter()
                .map(|(delta, kind)| TrackEvent {
                    delta: u28::from(delta),
                    kind,
                })
                .collect();
            t.push(TrackEvent {
                delta: u28::from(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            });
            file.tracks.push(t);
        }
        let mut bytes = Vec::new();
        file.write_std(&mut bytes).unwrap();
        bytes
    }

    fn metrical(ppq: u16) -> Timing {
        Timing::Metrical(u15::from(ppq))
    }

    fn msg(ch: u8, message: MidiMessage) -> TrackEventKind<'static> {
        TrackEventKind::Midi {
            channel: u4::from(ch),
            message,
        }
    }

    fn on(ch: u8, key: u8, vel: u8) -> TrackEventKind<'static> {
        let (key, vel) = (u7::from(key), u7::from(vel));
        msg(ch, MidiMessage::NoteOn { key, vel })
    }

    fn off(ch: u8, key: u8) -> TrackEventKind<'static> {
        let (key, vel) = (u7::from(key), u7::from(0));
        msg(ch, MidiMessage::NoteOff { key, vel })
    }

    fn tempo(us_per_beat: u32) -> TrackEventKind<'static> {
        TrackEventKind::Meta(MetaMessage::Tempo(us_per_beat.into()))
    }

    fn parse(bytes: &[u8]) -> Song {
        parse_midi(bytes, ParamValues::default()).unwrap()
    }

    fn times(song: &Song) -> Vec<f64> {
        song.events.iter().map(|e| e.time).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn ticks_become_seconds_across_a_tempo_change() {
        // 480 ppq: 120 BPM for two beats, then 240 BPM (the tempo lives on
        // the conductor track, the notes on another)
        let bytes = smf(
            Format::Parallel,
            metrical(480),
            vec![
                vec![(0, tempo(500_000)), (960, tempo(250_000))],
                vec![(480, on(0, 60, 100)), (960, off(0, 60))],
            ],
        );
        let t = times(&parse(&bytes));
        // beat 1 at 0.5 s; beat 3 = 1.0 s + one 240-BPM beat (0.25 s)
        assert!(close(t[0], 0.5) && close(t[1], 1.25), "{t:?}");

        // No Tempo event: 120 BPM
        let bytes = smf(
            Format::SingleTrack,
            metrical(96),
            vec![vec![(192, on(0, 60, 100))]],
        );
        assert!(close(parse(&bytes).events[0].time, 1.0));
    }

    #[test]
    fn timecode_ticks_are_fixed_length() {
        // 25 fps x 40 subframes = 1000 ticks a second; tempo is moot
        let bytes = smf(
            Format::SingleTrack,
            Timing::Timecode(Fps::Fps25, 40),
            vec![vec![(0, tempo(250_000)), (500, on(0, 60, 100))]],
        );
        assert!(close(parse(&bytes).events[0].time, 0.5));
    }

    #[test]
    fn velocity_zero_note_on_is_a_note_off() {
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![(0, on(0, 64, 127)), (480, on(0, 64, 0))]],
        );
        let song = parse(&bytes);
        assert!(matches!(
            song.events[0].kind,
            EventKind::NoteOn { note: 64, channel: 1, velocity } if velocity == 1.0
        ));
        assert!(matches!(
            song.events[1].kind,
            EventKind::NoteOff {
                note: 64,
                channel: 1
            }
        ));
    }

    #[test]
    fn controllers_map_through_the_cc_chart() {
        let cc = |c: u8, v: u8| {
            let (controller, value) = (u7::from(c), u7::from(v));
            msg(2, MidiMessage::Controller { controller, value })
        };
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![(0, cc(1, 127)), (0, cc(64, 0))]],
        );
        let song = parse(&bytes);
        let got: Vec<(Param, f32, u16)> = song
            .events
            .iter()
            .map(|e| match e.kind {
                EventKind::Param {
                    param,
                    value,
                    channel,
                } => (param, value, channel),
                _ => panic!("expected a Param event"),
            })
            .collect();
        assert!(matches!(got[0], (Param::ModWheel, v, 1) if v == 1.0));
        assert!(matches!(got[1], (Param::SustainPedal, v, 1) if v == 0.0));
        assert_eq!(song.tracks, vec![("ch3".to_string(), 1)]);
    }

    #[test]
    fn pitch_bend_spans_two_semitones() {
        let bend = |b: f32| {
            let bend = PitchBend::from_f32(b);
            msg(0, MidiMessage::PitchBend { bend })
        };
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![(0, bend(0.5)), (10, bend(-1.0))]],
        );
        let song = parse(&bytes);
        let semis: Vec<f32> = song
            .events
            .iter()
            .map(|e| match e.kind {
                EventKind::Param {
                    param: Param::PitchBendSemis,
                    value,
                    ..
                } => value,
                _ => panic!("expected a bend"),
            })
            .collect();
        assert_eq!(semis, vec![1.0, -2.0]);
    }

    #[test]
    fn channel_ten_plays_the_drum_board() {
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![
                (0, on(9, 36, 100)),
                (240, off(9, 36)),
                (0, on(9, 38, 0)),
            ]],
        );
        let song = parse(&bytes);
        // one trigger; offs (and vel-0 ons) are dropped for one-shots
        assert_eq!(song.events.len(), 1);
        assert!(matches!(
            song.events[0].kind,
            EventKind::NoteOn { note: 36, channel, .. } if channel == crate::drums::DRUM_CHANNEL
        ));
        assert!(song.channels.is_empty(), "drums need no voice channel");
        assert_eq!(
            song.tracks,
            vec![("drums".to_string(), crate::drums::DRUM_CHANNEL)]
        );
    }

    #[test]
    fn same_tick_off_then_on_keeps_file_order() {
        // A repeated key: the first note's off and the re-strike share a tick
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![
                (0, on(0, 60, 100)),
                (480, off(0, 60)),
                (0, on(0, 60, 90)),
                (480, off(0, 60)),
            ]],
        );
        let song = parse(&bytes);
        let at_half: Vec<&EventKind> = song
            .events
            .iter()
            .filter(|e| close(e.time, 0.5))
            .map(|e| &e.kind)
            .collect();
        assert!(matches!(at_half[0], EventKind::NoteOff { note: 60, .. }));
        assert!(matches!(at_half[1], EventKind::NoteOn { note: 60, .. }));
    }

    #[test]
    fn type_1_tracks_merge_onto_their_own_channels() {
        let bytes = smf(
            Format::Parallel,
            metrical(480),
            vec![
                vec![(0, tempo(1_000_000))], // 60 BPM: a beat is a second
                vec![(480, on(4, 48, 100)), (960, off(4, 48))],
                vec![(0, on(1, 72, 100)), (960, off(1, 72))],
            ],
        );
        let song = parse(&bytes);
        let t = times(&song);
        assert!(t.windows(2).all(|w| w[0] <= w[1]), "unsorted: {t:?}");
        assert_eq!(t, vec![0.0, 1.0, 2.0, 3.0]);
        // Channels are allocated in order of first sound
        assert_eq!(
            song.tracks,
            vec![("ch2".to_string(), 1), ("ch5".to_string(), 2)]
        );
        assert_eq!(song.channels.len(), 2);
        assert!(matches!(
            song.events[1].kind,
            EventKind::NoteOn {
                note: 48,
                channel: 2,
                ..
            }
        ));
    }

    #[test]
    fn load_song_imports_by_extension() {
        let dir = std::env::temp_dir().join("patina_midi_import_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.MID");
        let bytes = smf(
            Format::SingleTrack,
            metrical(480),
            vec![vec![(0, on(0, 60, 100)), (480, off(0, 60))]],
        );
        std::fs::write(&path, bytes).unwrap();
        let song = crate::song::load_song(path.to_str().unwrap()).unwrap();
        assert_eq!(song.events.len(), 2);
        assert_eq!(song.tail_seconds, crate::song::DEFAULT_TAIL_SECONDS);
        // A .song picks its voices per track; a patch override is refused
        let text = dir.join("t.song");
        std::fs::write(&text, "track a\nC4\n").unwrap();
        assert!(crate::song::load_song_with_patch(text.to_str().unwrap(), Some("init")).is_err());
    }

    /// A named patch is the whole instrument: its bus half (chorus, tape...)
    /// fires once at the downbeat on the panel channel, while its voice half
    /// and its level stay with the track.
    #[test]
    fn a_patch_brings_its_bus_settings() {
        let bus = bus_settings("chorus_mode 2\ntape_age 0.5\ncutoff 640\nvolume 0.3").unwrap();
        let find = |want: Param| {
            bus.iter().find_map(|e| match e.kind {
                EventKind::Param { param, value, channel } if param == want => {
                    assert_eq!((e.time, channel), (0.0, 0));
                    Some(value)
                }
                _ => None,
            })
        };
        assert_eq!(find(Param::ChorusModeSel), Some(2.0));
        assert_eq!(find(Param::TapeAge), Some(0.5));
        assert_eq!(find(Param::Cutoff), None, "voice settings ride the track channel");
        assert_eq!(find(Param::Volume), None, "songs never take a patch's level");
    }
}
