# Rust Analog Synth Design Document

## 1. Overview and Goals

This document outlines the design for a Rust-based analog synthesizer simulator. Our primary goals are:

1. Create a modular, extensible system that can grow from a basic synthesizer to a complex one.
2. Ensure clean separation of concerns between audio processing, UI, and MIDI handling.
3. Maintain real-time performance for audio processing.
4. Allow for easy addition of new features and components.
5. Leverage Rust's strengths in safety, concurrency, and performance.
6. Accurately emulate the sound and behavior of analog synthesizer hardware.

## 2. Architecture Design

We will use a layered architecture with clear interfaces between components:

```
+-------------------+
|    User Interface |
+-------------------+
          |
+-------------------+
|    MIDI Handler   |
+-------------------+
          |
+-------------------+
|    Synth Engine   |
+-------------------+
    |            |
+-------+    +-------+
| Voice |    |Effects|
+-------+    +-------+
    |
+---------------------------+
| Oscillator | Filter | LFO |
+---------------------------+
    |
+-------------------+
|     Envelope      |
+-------------------+
```

## 3. Component Breakdown

1. **Synth Engine**: Coordinates all audio processing.
   - Manages voices using an object pool
   - Applies global effects
   - Handles audio device interaction

2. **Voice**: Represents a single synthesizer voice.
   - Contains Oscillator, Envelope, Filter, and LFO
   - Implemented as a struct with no runtime polymorphism
   - Add slight detuning between voices for a richer sound
   - Implement voice stealing algorithm for polyphony

3. **Oscillator**: Generates raw waveforms.
   - Implemented as a struct with methods for different waveform types
   - Uses atomic types for thread-safe parameter updates
   - Implement analog-style waveform generation with subtle imperfections
   - Add hard sync capability
   - Implement FM (Frequency Modulation) between oscillators
   - Add phase modulation capabilities

4. **Envelope**: Modulates amplitude over time (ADSR).
   - Implemented as a struct with methods for each stage
   - Implement non-linear ADSR curves to mimic analog behavior
   - Add voltage-controlled amplifier (VCA) emulation

5. **Filter**: Shapes the tone of the voice.
   - Uses traits for different filter types
   - Implement multiple filter types (Lowpass, Highpass, Bandpass)
   - Add resonance control
   - Implement self-oscillation capability
   - Add filter overdrive/saturation
   - Emulate analog filter non-linearities and instabilities

6. **LFO (Low Frequency Oscillator)**:
   - Implement various LFO shapes (sine, triangle, square, random)
   - Allow LFO to modulate various parameters (pitch, filter cutoff, etc.)

7. **Effects**: Applies post-processing effects.
   - Implements a trait object-based plugin system for flexibility
   - Add analog-style chorus
   - Implement tape-style delay
   - Add reverb emulation

8. **MIDI Handler**: Processes MIDI input.
   - Uses a custom enum for type-safe MIDI events

9. **User Interface**: Provides control over synth parameters.
   - Implemented using egui for immediate mode GUI
   - Communicates with audio thread via Arc<Mutex<>>

10. **Real-time Audio Processing:**
    - Use lock-free data structures and atomic types for parameter updates
    - Minimize allocations in the audio callback
    - Implement oversampling for alias reduction

## 4. Rust-Specific Design Considerations

1. **Thread Safety and Communication**:
   - Use `Arc<Mutex<>>` for shared state that requires complex operations
   - Implement a lock-free ring buffer for audio thread communication
   - Use atomics for simple shared state (e.g., global volume)

2. **Memory Management**:
   - Implement an object pool for voice allocation to avoid runtime allocations
   - Use `const` generics for fixed-size audio buffers

3. **Zero-Cost Abstractions**:
   - Define traits for Oscillator, Envelope, and Filter interfaces
   - Use static dispatch where possible for performance

4. **Type Safety**:
   - Implement newtype patterns for units like Frequency and Amplitude
   - Use a type-safe builder pattern for synth configuration

5. **Error Handling**:
   - Define a custom `SynthError` enum that encapsulates all possible errors
   - Use `Result` types consistently, avoiding panics in audio threads

6. **Concurrency Patterns**:
   - Use channels for non-real-time communication between threads
   - Explore lock-free data structures for real-time parameter updates

7. **Optimization**:
   - Utilize SIMD instructions for audio processing where applicable
   - Avoid allocations and blocking operations in the audio thread

8. **SIMD Optimization**:
   - Use Rust's SIMD intrinsics for parallel processing of audio samples

9. **Setter Idempotence (house rule)**:
   - Song automation re-asserts parameter values every block, so **every
     setter that allocates, rebuilds filters, re-randomizes, or otherwise
     resets state MUST early-return when the value is unchanged**. A
     setter that only stores a float may skip the guard.
   - Violations are subtle and audible: `Talker::set_clarity` once rebuilt
     its output filters per call and the cascade never rang up (the voice
     lost its top end); `Chorus::set_mode` rebuilt BBD voices per set
     event. Guarded examples: `Talker::set_clarity`, `Chorus::set_mode` /
     `set_rate` / `set_depth`, `Tape::set_drive` / `set_age`,
     `VoxBox::set_mode`.
   - The next structural parameter you add will hit this trap: write the
     guard first, and a test that re-asserting the same value leaves
     internal state untouched (see `chorus::tests::reasserting_the_same_mode_is_a_no_op`).

## 5. Key Rust Patterns and Features to Utilize

1. **Traits**: For defining common interfaces (e.g., `Oscillator`, `Effect`)
2. **Enums**: For representing different types (e.g., waveforms, MIDI events)
3. **Generics** and **PhantomData**: For type-safe configurations
4. **Atomic Types**: For lock-free sharing of simple values
5. **Channels**: For communication between non-audio threads
6. **Unsafe Code**: Carefully used for performance-critical sections, with clear safety documentation
7. **Macros**: For generating repetitive code (e.g., parameter setters)
8. **Type State Pattern**: For enforcing correct usage of the synthesizer API
9. **Const Generics**: For compile-time configuration of oversampling rates and buffer sizes

## 6. Analog-Specific Considerations

1. **Oscillator Drift**:
   - Implement subtle frequency drift to emulate temperature changes in analog circuits
   - Use noise generators to add slight instability to oscillator pitch

2. **Filter Behavior**:
   - Model non-linear behavior of filter cutoff and resonance
   - Implement filter saturation for overdrive effects
   - Ladder filter

3. **Envelope Non-Linearity**:
   - Model the non-linear charging and discharging of capacitors in analog ADSR circuits

4. **Component Tolerances**:
   - Simulate variations in component values to add subtle differences between voices

5. **Analog Warmth**:
   - Implement soft clipping and saturation throughout the signal path
   - Add subtle harmonic distortion to mimic analog circuitry

6. **Noise and Imperfections**:
   - Add low-level noise to various stages of the signal path
   - Implement "zipper noise" reduction for parameter changes

7. **Power Supply Simulation**:
   - Model subtle fluctuations in power supply to affect overall sound

8. **Anti-Aliasing**:
   - Implement oversampling and appropriate anti-aliasing filters

9. **Analog-Style Modulation**:
   - Implement cross-modulation between oscillators
   - Allow audio-rate modulation of filter parameters

## 7. Performance Optimizations

1. Implement vectorized operations using SIMD for core DSP functions
2. Optimize memory access patterns for cache-friendly processing
3. Implement multi-threading for parallel voice processing

## 8. Future Considerations

1. Explore async Rust for non-audio tasks if beneficial
2. Consider FFI for integrating with existing audio or MIDI libraries
3. Investigate using WebAssembly for a web-based version of the synthesizer
4. Implement a flexible audio graph using traits and generics