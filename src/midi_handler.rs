// src/midi_handler.rs
//
// This file implements MIDI input functionality for the synthesizer, allowing connection
// to MIDI keyboards and other controllers. The implementation uses the midir crate for
// cross-platform MIDI device access and midly for MIDI message parsing.

use crossbeam_channel::{bounded, Receiver, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiInputPort};
use midly::{live::LiveEvent, MidiMessage};
use parking_lot::Mutex; // Using parking_lot::Mutex instead of std::sync::Mutex as per project convention
use std::error::Error;
use std::sync::Arc;

// Import the VoiceManager from our project
use crate::voice_manager::VoiceManager;

/// Represents the types of MIDI events our synthesizer will process.
/// 
/// Currently we're handling the basic note events, but this enum can be extended
/// in the future to handle control changes, pitch bend, etc.
#[derive(Debug, Clone)]
#[allow(dead_code)] // channel-based API kept alongside the direct VoiceManager path
pub enum MidiEvent {
    /// Note On event with note number (0-127) and velocity (0-127)
    NoteOn { note: u8, velocity: u8 },
    
    /// Note Off event with note number (0-127) and velocity (0-127)
    /// Note: Most MIDI keyboards send velocity with Note Off, but we don't use it currently
    NoteOff { note: u8, velocity: u8 },
    
    // Future expansion possibilities:
    // ControlChange { controller: u8, value: u8 },
    // PitchBend { value: i16 },
    // ModWheel { value: u8 },
}

/// Manages MIDI input device connections and routes MIDI messages to the synthesizer.
///
/// The MidiHandler provides two methods of operation:
/// 1. Direct connection to the VoiceManager (preferred for this project)
/// 2. Channel-based communication
///
/// The direct connection is more efficient for our needs since it doesn't require
/// additional message passing between threads.
pub struct MidiHandler {
    /// The MidiInput instance used for scanning available ports.
    /// This is kept separate from the connection to allow rescanning while connected.
    midi_in: Option<MidiInput>,

    /// The active MIDI input connection. When this is Some, we are connected to a device.
    connection: Option<MidiInputConnection<()>>,

    /// List of available MIDI ports with their indices, names, and port objects.
    /// This is populated by the scan_devices() method.
    available_ports: Vec<(usize, String, MidiInputPort)>,

    /// Channel for sending MIDI events to other threads if needed.
    /// This is an alternative to the direct VoiceManager approach.
    sender: Sender<MidiEvent>,

    /// Channel for receiving MIDI events (for the channel-based approach).
    #[allow(dead_code)]
    receiver: Receiver<MidiEvent>,

    /// Reference to the VoiceManager for direct event handling.
    /// When this is set, MIDI events directly trigger voice_manager methods.
    voice_manager: Option<Arc<Mutex<VoiceManager>>>,
}

#[allow(dead_code)] // the channel-based helpers are unused but part of the public surface
impl MidiHandler {
    /// Creates a new MidiHandler and returns the handler along with a receiver
    /// for MIDI events.
    ///
    /// # Returns
    /// 
    /// A tuple containing:
    /// - The MidiHandler instance
    /// - A Receiver<MidiEvent> that can be used to receive MIDI events in another thread
    ///
    /// # Errors
    ///
    /// Returns an error if initializing the MIDI system fails.
    ///
    /// # Example
    ///
    /// ```text
    /// let (mut midi_handler, midi_receiver) = MidiHandler::new().unwrap()
    /// ```
    pub fn new() -> Result<(Self, Receiver<MidiEvent>), Box<dyn Error>> {
        let (sender, receiver) = bounded(128);
        let receiver_clone = receiver.clone();
    
        let midi_in = MidiInput::new("patina_midi_input")?;

        let mut handler = Self {
            midi_in: Some(midi_in),
            connection: None,
            available_ports: Vec::new(),
            sender,
            receiver,
            voice_manager: None,
        };

        // Scan for devices immediately
        handler.scan_devices()?;
        
        // Attempt to auto-connect to IAC Driver if available
        let iac_index = handler.find_iac_driver();
        if let Some(idx) = iac_index {
            println!("Auto-connecting to IAC Driver at index {}", idx);
            // Don't fail if we can't connect - just log it
            if let Err(e) = handler.connect_to_device(idx) {
                eprintln!("Failed to auto-connect to IAC Driver: {}", e);
            }
        } else {
            println!("No IAC Driver found for auto-connection");
        }
        
        Ok((handler, receiver_clone))
    }
    
    /// Finds the index of the IAC Driver in the available ports list
    fn find_iac_driver(&self) -> Option<usize> {
        for (idx, (_, name, _)) in self.available_ports.iter().enumerate() {
            if name.contains("IAC") {
                return Some(idx);
            }
        }
        None
    }

    /// Sets the voice manager for direct MIDI event handling.
    ///
    /// When set, incoming MIDI events will directly trigger methods on the voice manager
    /// without going through the channel. This is the preferred approach for this project
    /// as it simplifies the architecture and avoids extra message passing.
    ///
    /// # Parameters
    ///
    /// * `voice_manager` - An Arc<Mutex<VoiceManager>> reference, which matches how
    ///                     VoiceManager is used throughout the project
    ///
    /// # Example
    ///
    /// ```text
    /// let voice_manager = Arc::new(Mutex::new(VoiceManager::new(sample_rate, 8)));
    /// midi_handler.set_voice_manager(Arc::clone(&voice_manager));
    /// ```
    pub fn set_voice_manager(&mut self, voice_manager: Arc<Mutex<VoiceManager>>) {
        // Store the reference to the voice manager for use in the MIDI callback
        self.voice_manager = Some(voice_manager);
        
        // Note: This works because our voice_manager is already designed to be
        // accessed safely from multiple threads via Arc<Mutex<>>
    }
    
    /// Scans for available MIDI input devices and updates the internal list.
    ///
    /// This method queries the operating system's MIDI system to find all available
    /// MIDI input devices, including physical MIDI keyboards and virtual MIDI ports
    /// created by other software.
    ///
    /// # Returns
    ///
    /// Ok(()) if successful, or an error if scanning fails.
    ///
    /// # Example
    ///
    /// ```text
    /// midi_handler.scan_devices().unwrap();
    /// let devices = midi_handler.get_devices();
    /// for (index, name) in devices {
    ///     println!("{}: {}", index, name);
    /// }
    /// ```
    pub fn scan_devices(&mut self) -> Result<(), Box<dyn Error>> {
        // Create a new MidiInput instance if needed
        if self.midi_in.is_none() {
            self.midi_in = Some(MidiInput::new("patina_midi_input")?);
        }
        
        let midi_in = self.midi_in.as_ref().unwrap();
        self.available_ports.clear();
        
        println!("Available MIDI input devices:");
        
        // Collect all available ports
        for (i, port) in midi_in.ports().into_iter().enumerate() {
            match midi_in.port_name(&port) {
                Ok(name) => {
                    println!("  {}: {}", i, name);
                    self.available_ports.push((i, name, port));
                },
                Err(err) => {
                    eprintln!("Error getting port name: {}", err);
                }
            }
        }
        
        Ok(())
    }
    
    /// Returns a list of available MIDI devices for display in the UI.
    ///
    /// This method converts the internal port information into a simpler format
    /// containing just the index and name of each device, suitable for displaying
    /// in a dropdown menu or list in the UI.
    ///
    /// # Returns
    ///
    /// A Vec of tuples containing (index, name) for each available MIDI device.
    ///
    /// # Example
    ///
    /// ```text
    /// for (index, name) in midi_handler.get_devices() {
    ///     println!("{}: {}", index, name);
    /// }
    /// ```
    pub fn get_devices(&self) -> Vec<(usize, String)> {
        // Map the internal port list to a simpler format for UI display
        // This extracts just the index and name, omitting the technical MidiInputPort object
        self.available_ports
            .iter()
            .map(|(idx, name, _)| (*idx, name.clone()))
            .collect()
    }
    
    /// Connects to a MIDI input device by its index in the available devices list.
    ///
    /// This method establishes a connection to the selected MIDI device and sets up
    /// a callback to handle incoming MIDI messages. When a MIDI message is received,
    /// it will be parsed and either:
    /// 1. Directly handled by calling methods on the VoiceManager (if set), or
    /// 2. Sent through the channel for processing elsewhere
    ///
    /// # Parameters
    ///
    /// * `index` - The index of the device to connect to, from the list returned
    ///             by get_devices()
    ///
    /// # Returns
    ///
    /// Ok(()) if the connection was successful, or an error if it failed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The index is out of bounds
    /// - Creating a new MidiInput fails
    /// - Connecting to the port fails
    ///
    /// # Example
    ///
    /// ```text
    /// midi_handler.scan_devices().unwrap();
    /// if !midi_handler.get_devices().is_empty() {
    ///     midi_handler.connect_to_device(0).unwrap(); // Connect to the first device
    /// }
    /// ```
    fn connect_to_device(&mut self, index: usize) -> Result<(), Box<dyn Error>> {
        // Disconnect any existing connection first
        self.disconnect();
        
        // Verify the index is valid
        if index >= self.available_ports.len() {
            return Err("Invalid MIDI device index".into());
        }
        
        // Clone the port and name for the selected device
        let (idx, name, port) = &self.available_ports[index];
        let port = port.clone();
        let port_name = name.clone();
        
        println!("Attempting to connect to MIDI device #{}: {}", idx, port_name);
        
        // We need to create a new MidiInput for the connection
        let mut midi_in = MidiInput::new("patina_midi_connection")?;
        midi_in.ignore(Ignore::None);
        
        // Clone sender and voice_manager for the closure
        let sender = self.sender.clone();
        let voice_manager = self.voice_manager.clone();
        
        // Add debug print in the callback to confirm we're receiving MIDI messages
        let connection = midi_in.connect(
            &port,
            "patina",
            move |_timestamp, message, _| {
                // This closure is called for each incoming MIDI message
                
                // Try to parse the raw MIDI bytes using midly
                if let Ok(event) = LiveEvent::parse(message) {
                    // Process standard MIDI channel messages
                    if let LiveEvent::Midi { channel: _, message } = event {
                        match message {
                            // Handle Note On messages
                            MidiMessage::NoteOn { key, vel } => {
                                let note = key.as_int();
                                let velocity = vel.as_int();
                                
                                // MIDI spec: Note On with velocity 0 is equivalent to Note Off
                                if velocity > 0 {
                                    // This is a genuine Note On message
                                    if let Some(vm) = &voice_manager {
                                        // Direct approach: call note_on() on the VoiceManager
                                        vm.lock().note_on(note, velocity as f32 / 127.0);
                                    } else {
                                        // Channel approach: send a NoteOn event through the channel
                                        let _ = sender.send(MidiEvent::NoteOn { 
                                            note, 
                                            velocity 
                                        });
                                    }
                                } else {
                                    // This is a Note Off message disguised as Note On with velocity 0
                                    if let Some(vm) = &voice_manager {
                                        vm.lock().note_off(note);
                                    } else {
                                        let _ = sender.send(MidiEvent::NoteOff { 
                                            note, 
                                            velocity: 0 
                                        });
                                    }
                                }
                            },
                            // Handle explicit Note Off messages
                            MidiMessage::NoteOff { key, vel: _ } => {
                                let note = key.as_int();
                                
                                if let Some(vm) = &voice_manager {
                                    vm.lock().note_off(note);
                                } else {
                                    let _ = sender.send(MidiEvent::NoteOff { 
                                        note, 
                                        velocity: 0 // We don't currently use Note Off velocity
                                    });
                                }
                            },
                            // Pitch wheel: midly gives -1..1, standard range +/-2 semitones
                            MidiMessage::PitchBend { bend } => {
                                if let Some(vm) = &voice_manager {
                                    vm.lock().set_pitch_bend(bend.as_f32() * 2.0);
                                }
                            },
                            MidiMessage::Controller { controller, value } => {
                                if let Some(vm) = &voice_manager {
                                    // The full chart lives in Param::from_cc —
                                    // every automatable parameter answers to a
                                    // controller, scaled like its knob
                                    if let Some(param) = crate::song::Param::from_cc(controller.as_int()) {
                                        let t = value.as_int() as f32 / 127.0;
                                        param.apply(&mut vm.lock(), param.midi_value(t));
                                    }
                                }
                            },
                            // Program change flips the whole instrument to a
                            // factory patch, keyboard register included
                            MidiMessage::ProgramChange { program } => {
                                if let Some(vm) = &voice_manager {
                                    let bank = crate::patch::FACTORY;
                                    let idx = program.as_int() as usize;
                                    if let Some((name, text)) = bank.get(idx) {
                                        if let Err(e) = crate::patch::apply(&mut vm.lock(), text) {
                                            eprintln!("Program change to '{}' failed: {}", name, e);
                                        } else {
                                            println!("Program change: {}", name);
                                        }
                                    }
                                }
                            },
                            _ => {} // Ignore other message types for now
                        }
                    }
                }
            },
            (),
        )?;
        
        println!("Connected to MIDI device: {}", port_name);
        self.connection = Some(connection);
        
        Ok(())
    }
    
    /// Disconnects from the current MIDI device if connected.
    ///
    /// This method safely closes the current MIDI connection and releases resources.
    /// It's safe to call even if no device is currently connected.
    ///
    /// # Example
    ///
    /// ```text
    /// midi_handler.disconnect();
    /// ```
    pub fn disconnect(&mut self) {
        // Take the connection option, which moves ownership out of self.connection
        // and leaves None in its place
        if let Some(conn) = self.connection.take() {
            // Dropping the connection object closes the connection
            drop(conn);
            println!("Disconnected from MIDI device");
        }
        // If there was no connection, this method does nothing
    }
    
    /// Processes pending MIDI events from the channel.
    ///
    /// This method should be called regularly (e.g., from the audio thread) if
    /// using the channel-based approach rather than direct voice manager access.
    /// It processes all pending MIDI events without blocking.
    ///
    /// Note: This method is only needed if NOT using the direct voice_manager
    /// approach via set_voice_manager(). With our project structure, the direct
    /// approach is preferred.
    ///
    /// # Parameters
    ///
    /// * `voice_manager` - A mutable reference to the VoiceManager to handle the events
    ///
    /// # Returns
    ///
    /// Ok(()) if successful, or an error if processing fails
    ///
    /// # Example
    ///
    /// ```text
    /// // In your audio processing callback:
    /// midi_handler.process_events(&mut voice_manager).unwrap();
    /// ```
    pub fn process_events(&self, voice_manager: &mut VoiceManager) -> Result<(), Box<dyn Error>> {
        // Try to receive all pending MIDI events without blocking
        // This we don't stall the audio thread if the channel is empty
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                MidiEvent::NoteOn { note, velocity } => {
                    voice_manager.note_on(note, velocity as f32 / 127.0);
                },
                MidiEvent::NoteOff { note, velocity: _ } => {
                    voice_manager.note_off(note);
                },
                // Handle other event types here as they're added
            }
        }
        
        Ok(())
    }
    
    /// Checks if currently connected to a MIDI device.
    ///
    /// # Returns
    ///
    /// `true` if connected to a device, `false` otherwise
    ///
    /// # Example
    ///
    /// ```text
    /// if midi_handler.is_connected() {
    ///     println!("Connected to a MIDI device");
    /// } else {
    ///     println!("Not connected to any MIDI device");
    /// }
    /// ```
    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }
}

// Implement Drop to so resources are cleaned up properly when the MidiHandler is dropped
impl Drop for MidiHandler {
    fn drop(&mut self) {
        // so we disconnect from any MIDI devices to avoid resource leaks
        self.disconnect();
    }
}
