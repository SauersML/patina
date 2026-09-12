# Synth performance

## Live window

The standalone panel schedules animation and meter refreshes every 33 ms
while playing in the foreground, or every 100 ms while idle/unfocused.
Keyboard and mouse events still request immediate frames. A minimized
window skips textures, shaders and panel layout, checking once a second.
Held window keys are released on minimize.

One nonblocking engine snapshot supplies parameters, keyboard lights, drum
meters and the scope. Waveform drawing and RMS calculation run after the
lock is released. The snapshot reuses an 8 KiB scope array, and glass-panel
rectangle storage retains its capacity between frames. Visual smoothing
uses elapsed time so reducing repaint frequency does not stretch fades.

MIDI connects with the engine already assigned and delivers notes directly.
The old alternate event queue and its unused dependency are removed: an
auto-connected port could capture `None` before engine assignment, fill the
undrained queue, and stall after 128 messages. Each open input now has a
fixed 128-note table. Rescans compare port identities, including identically
named keyboards, and fetch names only for newly connected inputs. Removed
ports close before their held notes are released.

In the audio engine, the high-pass ladder reuses its exact coefficient once
the smoothed cutoff stops changing. Drive makeup and voice-box gain slew
coefficients are calculated when configured, instead of per sample. Newton
iteration counters exist only in test builds.

Run the live heap regression independently of the desktop/audio device:

```sh
cargo run --release --no-default-features -j 1 --example live_budget
```

This includes the first callback and key press, voice stealing, sustain,
drums, pitch bend, filter automation, both ladder circuits and release tails.
On this Mac, the 10-voice engine retained **683,916 heap bytes at 48 kHz**
(668 KiB), or **1,338,768 bytes at 96 kHz** (1.28 MiB). Both workloads made
**zero playback allocations and zero heap growth**, before and after this
change. Before/after audio fingerprints matched at both rates. These are
engine heap figures, not total desktop memory or measured device latency;
window, graphics-driver and MIDI-backend costs are excluded. The example's
startup timer measures engine construction only.

A release-app launch smoke check opened the 48 kHz stereo headphone output
and detected the connected Hammer 88. A two-second macOS `top` sample while
idle reported about **124 MiB app memory and 7.4% of one CPU core**. Other
work heavily loaded the machine, so this is an observed footprint, not a
controlled before/after desktop comparison. The temporary test instance was
closed afterward. End-to-end key-to-speaker latency was not measured.

A separate two-run CPU comparison against `de90092`, in opposite run orders,
used the same optimization settings described below. Sparse playback fell
from 2,677 to 2,357 CPU ns/frame (12% lower); the live chord was effectively
unchanged (4,330 to 4,351). Idle measurements varied from 683 to 867 ns/frame,
so they do not establish an improvement. The main live-window saving comes
from less frequent scheduled UI work, which this headless CPU benchmark
does not measure.

## Offline bounces

WAV and stem rendering stream frames directly from the engine into a 64 KiB
write buffer. Normalization scales that same file in 64 KiB chunks after its
peak is known, so it does not render a second, different noise performance.
The loudness meter keeps four 100 ms energy sums and one energy value per
400 ms window (75% overlap). Its history grows by approximately 80 bytes per
second of audio, instead of keeping every weighted sample.

`render_offline` and `render_offline_solo` return `OfflineRender`, an exact-size,
fused iterator. Callers that need random access must explicitly collect it.
RIFF bounces exceeding the format's size limit are rejected before opening
the output file.

The audio engine avoids channel routing for idle voice cards and computes
modulation once per channel. The transistor filters use an 11/10 rational
approximation of `tanh`, with absolute f32 error below 1e-6. The tape oxide
computes its rational `coth` directly, removing a redundant division.
Oversampling, Newton tolerances, envelope behavior, and effect tails retain
their existing settings.

## Reproduce

Run these separately, with one build job:

```sh
cargo run --release --no-default-features -j 1 --example performance
cargo build --release --no-default-features -j 1 --example render_memory
/usr/bin/time -l target/release/examples/render_memory 30 /tmp/patina-bounce.wav
```

On Linux use `/usr/bin/time -v` for memory statistics. The CPU benchmark
reports process CPU time as well as wall time; use CPU time when other work
is competing for the machine. Remove the generated WAV after measuring.

The streaming tests compare sample values across normalization chunk
boundaries and compare loudness against a separate full-buffer BS.1770
implementation, including both gates, silence, gain changes, and incomplete
windows. Numerical sweeps check the transistor and oxide curves. The song
tail test checks iterator length, event dispatch, and repeated exhaustion.

## Measurements

Measured on this Apple Silicon Mac against synced commit `c3959b2`, with
Rust 1.93, identical cached dependencies, optimization level 3, and one
codegen unit. Each workload generates 96,000 measured frames after warm-up.
The table averages two runs in opposite orders. Other sessions were active;
wall time was heavily affected by contention, and CPU measurements also
varied between runs.

| Workload | Before CPU ns/frame | After CPU ns/frame | Change |
| --- | ---: | ---: | ---: |
| Idle live engine, 16 cards | 1001 | 743 | 26% lower |
| Live chord, 8 active cards | 4175 | 3606 | 14% lower |
| Sparse bounce, 4 active / 64 cards | 3142 | 2139 | 32% lower |
| Dense bounce, 32 active / 64 cards | 13867 | 14114 | 2% higher |

The dense result is smaller than the between-run variation, so this
measurement does not establish an improvement for that workload.

A normalized 30-second bounce measured **24,510,464 bytes before and
2,195,456 bytes after** in maximum resident set size: approximately **91%
less RAM** (23.4 MiB to 2.1 MiB). Both WAVs contained 1,440,000 stereo frames
and reported the same peak, RMS, and integrated loudness. This workload has
one short note and a long tail, isolating the cost of retaining the bounce.

## Validation

The earlier full core run completed with **222 passed, 3 failed, 3 ignored**.
The same three failures occurred in the unchanged baseline:

- `oscillator::tests::moog_tracking_error_is_additive_hertz`
- `song::tests::tempo_lane_can_accelerate_continuously_into_audio_rate`
- `voice_manager::tests::pitch_bend_shifts_frequency`

All new numerical, loudness, WAV, and iterator checks passed, as did the
existing filter, tape, and whole-engine stability checks. To keep local
resource use bounded, the engine and tests were compiled directly with
cached dependencies, one codegen worker, and low process priority. The final
test build peaked near 202 MiB resident memory; the serial test run near
11 MiB. That validation round did not rerun desktop or native plugin builds.

The live DSP follow-up passed **41 targeted tests, with 1 ignored**. This
includes bit-for-bit high-pass output comparisons through sweeps and settled
controls at 8, 44.1, 96 and 192 kHz. The default desktop `cargo check` passed,
and passed again with the final manifest/lockfile. Its compiler process group
peaked at 212 MiB resident memory under the task's resource guard. Checks
ran offline, with one low-priority build job and incremental/debug output
disabled.

Both native MIDI regressions passed in the app crate: malformed/drum input
does not hold keyboard notes, and 3,072 repeated note messages reach the
engine without queueing or stalling. The app test build reused the compiled
release dependencies and peaked at 222 MiB resident memory.
