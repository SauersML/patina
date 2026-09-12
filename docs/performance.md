# Engine and bounce performance

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

The final core test run completed with **222 passed, 3 failed, 3 ignored**.
The same three failures occurred in the unchanged baseline:

- `oscillator::tests::moog_tracking_error_is_additive_hertz`
- `song::tests::tempo_lane_can_accelerate_continuously_into_audio_rate`
- `voice_manager::tests::pitch_bend_shifts_frequency`

All new numerical, loudness, WAV, and iterator checks passed, as did the
existing filter, tape, and whole-engine stability checks. To keep local
resource use bounded, the engine and tests were compiled directly with
cached dependencies, one codegen worker, and low process priority. The final
test build peaked near 202 MiB resident memory; the serial test run near
11 MiB. App/GUI and native plugin feature builds were not rerun.
