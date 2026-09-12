// src/midi_handler.rs
//
// Hardware and virtual MIDI input for the standalone app. Every MIDI input
// port the OS knows about is opened and its messages drive the VoiceManager
// directly from the MIDI thread — the same thread-safety contract the audio
// callback already relies on (parking_lot::Mutex around the VoiceManager).
//
// Ports are re-scanned on a slow tick from the UI thread so a keyboard plugged
// in after launch starts playing without a restart, and a keyboard pulled
// mid-chord has its notes released instead of ringing forever.
//
// There is no "unconnected" state to get wrong: the handler cannot be built
// without a VoiceManager, and it connects to everything it can see.

use midir::{Ignore, MidiInput, MidiInputConnection};
use midly::{live::LiveEvent, MidiMessage};
use parking_lot::Mutex;
use std::error::Error;
use std::sync::Arc;

use crate::voice_manager::VoiceManager;

struct PortState {
    keys: [bool; 128],
    pedal_down: bool,
}

impl Default for PortState {
    fn default() -> Self {
        Self {
            keys: [false; 128],
            pedal_down: false,
        }
    }
}

/// A live connection keyed by the backend's port identity. Two keyboards
/// with the same display name are still independent connections.
struct OpenPort {
    name: String,
    port: midir::MidiInputPort,
    // Dropping the connection closes the port.
    _connection: MidiInputConnection<()>,
    /// Key and pedal state needed to release this input on unplug.
    state: Arc<Mutex<PortState>>,
}

/// Opens every MIDI input port and routes its messages into the synth.
pub struct MidiHandler {
    voice_manager: Arc<Mutex<VoiceManager>>,
    /// One long-lived client used only to enumerate ports on each re-scan.
    scanner: MidiInput,
    ports: Vec<OpenPort>,
}

impl MidiHandler {
    /// Builds the handler and connects to every MIDI input currently present.
    ///
    /// Fails only if the OS MIDI system itself cannot be initialised; a port
    /// that refuses to open is logged and skipped so one bad device never
    /// takes the keyboard with it.
    pub fn new(voice_manager: Arc<Mutex<VoiceManager>>) -> Result<Self, Box<dyn Error>> {
        let mut handler = Self {
            voice_manager,
            scanner: MidiInput::new("patina_midi_scan")?,
            ports: Vec::new(),
        };
        handler.refresh();
        if handler.ports.is_empty() {
            println!("No MIDI input devices found — plug a keyboard in and it will connect.");
        }
        Ok(handler)
    }

    /// Re-scans the MIDI inputs: opens ports that appeared, closes ports that
    /// vanished (releasing any notes they were holding), leaves the rest alone.
    /// Cheap enough to call every second from the UI thread.
    pub fn refresh(&mut self) {
        let present = self.scanner.ports();

        // Close what disappeared, and let go of its notes.
        let mut i = 0;
        while i < self.ports.len() {
            if present.contains(&self.ports[i].port) {
                i += 1;
                continue;
            }
            let gone = self.ports.remove(i);
            // Close first, so no callback can start a note after we release it.
            drop(gone._connection);
            release_port(&mut self.voice_manager.lock(), &mut gone.state.lock());
            println!("MIDI input removed: {}", gone.name);
        }

        // Open what appeared.
        for port in present {
            if self.ports.iter().any(|open| open.port == port) {
                continue;
            }
            let name = match self.scanner.port_name(&port) {
                Ok(name) => name,
                Err(e) => {
                    eprintln!("Could not read MIDI input name: {e}");
                    continue;
                }
            };
            match self.open(&name, &port) {
                Ok(open) => {
                    println!("Connected to MIDI input: {}", name);
                    self.ports.push(open);
                }
                Err(e) => eprintln!("Could not open MIDI input '{}': {}", name, e),
            }
        }
    }

    fn open(&self, name: &str, port: &midir::MidiInputPort) -> Result<OpenPort, Box<dyn Error>> {
        let mut midi_in = MidiInput::new("patina_midi_input")?;
        midi_in.ignore(Ignore::None);
        let vm = Arc::clone(&self.voice_manager);
        let state = Arc::new(Mutex::new(PortState::default()));
        let state_in_callback = Arc::clone(&state);
        let connection = midi_in.connect(
            port,
            "patina",
            move |_timestamp, message, _| handle_message(&vm, &state_in_callback, message),
            (),
        )?;
        Ok(OpenPort {
            name: name.to_string(),
            port: port.clone(),
            _connection: connection,
            state,
        })
    }
}

/// Disconnect is also the last pedal-up event for that input. Keys already
/// released under sustain are absent from the key table but still sound.
fn release_port(vm: &mut VoiceManager, state: &mut PortState) {
    if state.pedal_down {
        vm.set_sustain_pedal(false);
    }
    for (note, down) in state.keys.iter().copied().enumerate() {
        if down {
            vm.note_off_channel(note as u8, 0);
        }
    }
    *state = PortState::default();
}

/// One raw MIDI message from any port, applied to the synth.
fn handle_message(vm: &Arc<Mutex<VoiceManager>>, state: &Arc<Mutex<PortState>>, message: &[u8]) {
    let Ok(LiveEvent::Midi { channel, message }) = LiveEvent::parse(message) else {
        return;
    };
    // GM convention: channel 10 (0-indexed 9) is the drum channel — those
    // notes hit the 909 board's trigger inputs instead of the keyboard voices.
    let drums = channel.as_int() == 9;
    match message {
        MidiMessage::NoteOn { key, vel } => {
            let note = key.as_int();
            let velocity = vel.as_int();
            // MIDI spec: Note On with velocity 0 is a Note Off.
            if velocity == 0 {
                if !drums {
                    state.lock().keys[note as usize] = false;
                    vm.lock().note_off(note);
                }
                return;
            }
            let velocity = velocity as f32 / 127.0;
            if drums {
                vm.lock()
                    .note_on_channel(note, velocity, crate::drums::DRUM_CHANNEL);
            } else {
                state.lock().keys[note as usize] = true;
                vm.lock().note_on(note, velocity);
            }
        }
        MidiMessage::NoteOff { key, .. } => {
            // Drum voices are one-shots; the 909 trigger has no falling edge.
            if !drums {
                let note = key.as_int();
                state.lock().keys[note as usize] = false;
                vm.lock().note_off(note);
            }
        }
        // Pitch wheel: midly gives -1..1, standard range +/-2 semitones.
        MidiMessage::PitchBend { bend } => {
            vm.lock().set_pitch_bend(bend.as_f32() * 2.0);
        }
        MidiMessage::Controller { controller, value } => {
            if controller.as_int() == 64 {
                state.lock().pedal_down = value.as_int() >= 64;
            }
            // The full chart lives in Param::from_cc — every automatable
            // parameter answers to a controller, scaled like its knob.
            if let Some(param) = crate::song::Param::from_cc(controller.as_int()) {
                let t = value.as_int() as f32 / 127.0;
                param.apply(&mut vm.lock(), param.midi_value(t));
            }
        }
        // Program change flips the whole instrument to a factory patch,
        // keyboard register included.
        MidiMessage::ProgramChange { program } => {
            let bank = crate::patch::FACTORY;
            if let Some((name, text)) = bank.get(program.as_int() as usize) {
                match crate::patch::apply(&mut vm.lock(), text) {
                    Ok(()) => println!("Program change: {}", name),
                    Err(e) => eprintln!("Program change to '{}' failed: {}", name, e),
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnect_releases_notes_already_lifted_under_sustain() {
        let vm = Arc::new(Mutex::new(VoiceManager::new(48000.0, 10)));
        let state = Arc::new(Mutex::new(PortState::default()));
        vm.lock().set_release(0.005);
        handle_message(&vm, &state, &[0x90, 60, 100]);
        handle_message(&vm, &state, &[0xb0, 64, 127]);
        for _ in 0..128 {
            vm.lock().render_next();
        }
        handle_message(&vm, &state, &[0x80, 60, 0]);
        assert!(!state.lock().keys[60]);
        assert!(vm.lock().held_note_states()[60]);

        release_port(&mut vm.lock(), &mut state.lock());
        assert!(!vm.lock().held_note_states()[60]);
        assert!(!state.lock().pedal_down);
        let mut vm = vm.lock();
        for _ in 0..2048 {
            vm.render_next();
        }
        assert!(vm.voices.iter().all(|v| !v.is_active()));
        // A new keyboard must not inherit a latched pedal either.
        vm.note_on(64, 0.8);
        vm.note_off(64);
        assert!(!vm.held_note_states()[64]);
    }

    #[test]
    fn ordinary_pedal_up_releases_deferred_notes() {
        let vm = Arc::new(Mutex::new(VoiceManager::new(48000.0, 10)));
        let state = Arc::new(Mutex::new(PortState::default()));
        handle_message(&vm, &state, &[0xb0, 64, 127]);
        handle_message(&vm, &state, &[0x90, 60, 100]);
        handle_message(&vm, &state, &[0x90, 60, 0]);
        assert!(vm.lock().held_note_states()[60]);
        handle_message(&vm, &state, &[0xb0, 64, 0]);
        assert!(!vm.lock().held_note_states()[60]);
        assert!(!state.lock().pedal_down);
    }

    #[test]
    fn repeated_live_notes_reach_engine_without_a_queue() {
        let vm = Arc::new(Mutex::new(VoiceManager::new(48000.0, 10)));
        let state = Arc::new(Mutex::new(PortState::default()));
        // Exceed the old undrained queue's 128-message capacity repeatedly.
        for i in 0..1024 {
            let note = 48 + (i % 24) as u8;
            handle_message(&vm, &state, &[0x90, note, 100]);
            handle_message(&vm, &state, &[0x90, note, 100]);
            assert!(vm.lock().held_note_states()[note as usize]);
            assert_eq!(state.lock().keys.iter().filter(|&&down| down).count(), 1);
            let status = if i % 2 == 0 { 0x80 } else { 0x90 };
            handle_message(&vm, &state, &[status, note, 0]);
            assert!(!vm.lock().held_note_states()[note as usize]);
            assert!(!state.lock().keys.iter().any(|&down| down));
        }
    }

    #[test]
    fn drum_and_malformed_messages_do_not_hold_keyboard_notes() {
        let vm = Arc::new(Mutex::new(VoiceManager::new(48000.0, 10)));
        let state = Arc::new(Mutex::new(PortState::default()));
        handle_message(&vm, &state, &[]);
        handle_message(&vm, &state, &[0x90, 60]);
        handle_message(&vm, &state, &[0x99, 36, 100]);
        assert!(!state.lock().keys.iter().any(|&down| down));
        assert!(!vm.lock().held_note_states().iter().any(|&down| down));
        let mut vm = vm.lock();
        for _ in 0..128 {
            vm.render_next();
        }
        assert!(vm.drums.activity().iter().any(|&level| level > 0.0));
    }
}
