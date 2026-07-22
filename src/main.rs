use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SizedSample};
use dasp_sample::FromSample;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use parking_lot::Mutex;
use eframe::egui;

use patina::midi_handler::MidiHandler;
use patina::song;
use patina::ui::SynthUI;
use patina::voice_manager::VoiceManager;

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui.update(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.running.store(false, Ordering::SeqCst);
    }
}

struct SynthApp {
    ui: SynthUI,
    _stream: cpal::Stream,
    running: Arc<AtomicBool>,
}

fn run<T>(device: &cpal::Device, config: &cpal::StreamConfig, song_path: Option<&str>) -> Result<(), Box<dyn std::error::Error>>
where
    T: Sample + SizedSample + FromSample<f32>,
{
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;

    let voice_manager = Arc::new(Mutex::new(VoiceManager::new(sample_rate, 10))); // 10 voices
    let (mut midi_handler, _midi_rx) = MidiHandler::new()?;
    midi_handler.set_voice_manager(Arc::clone(&voice_manager));
    let running = Arc::new(AtomicBool::new(true));
    let vm_clone = Arc::clone(&voice_manager);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &vm_clone)
        },
        |err| eprintln!("an error occurred on stream: {}", err),
        None,
    )?;

    stream.play()?;

    let ui = SynthUI::new(Arc::clone(&voice_manager));

    if let Some(path) = song_path {
        let events = song::load_song(path)?;
        song::spawn_player(events, Arc::clone(&voice_manager));
    }

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1380.0, 980.0])
            .with_min_inner_size([1180.0, 840.0])
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_fullsize_content_view(true),
        ..Default::default()
    };

    eframe::run_native(
        "Patina",
        options,
        Box::new(|cc| {
            // The WGSL sky/glass pipeline lives in egui's callback resources
            let mut ui = ui;
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                patina::aurora_gpu::init(rs);
                ui.set_gpu_available(true);
            }
            Ok(Box::new(SynthApp { ui, _stream: stream, running }))
        }),
    ).map_err(|e| e.to_string())?;

    Ok(())
}

fn write_data<T>(output: &mut [T], channels: usize, voice_manager: &Arc<Mutex<VoiceManager>>)
where
    T: Sample + FromSample<f32>,
{
    // Lock once per callback, not once per frame
    let mut vm = voice_manager.lock();
    for frame in output.chunks_mut(channels) {
        let (left, right) = vm.render_next();
        let left_sample = T::from_sample(left);
        let right_sample = T::from_sample(right);

        for (i, sample) in frame.iter_mut().enumerate() {
            *sample = if i % 2 == 0 { left_sample } else { right_sample };
        }
    }
}

/// Build a one-track song that just says the phrase: each word is a
/// dash-joined ARPAbet syllable spoken on A2 with full intonation —
/// stress digits (`P-AH-T-IY1-N-AH`) place the pitch accents, `.` and
/// `?` end phrases with falls and rises, and the last word falls by
/// default. Mostly dry voice, a whisper of talk-boxed carrier under it.
fn say_song(text: &str) -> String {
    let mut s = String::from(
        "bpm 60\ngate 0.97\n\
         automate vox_dry\n0.9\n\
         automate vox_level\n0.3\n\
         automate vox_vibrato\n0.12\n\
         automate vox_intonation\n0.85\n\
         automate reverb_wet\n0.2\n\
         track say vox vel=0.85\n",
    );
    let words: Vec<&str> = text.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        let mut word = (*word).to_string();
        let phrase_end = word.ends_with('.') || word.ends_with('?');
        if i + 1 == words.len() && !phrase_end {
            word.push('.'); // a statement ends with a fall
        }
        let n = word.split('-').count();
        // A breath between words, a longer one between phrases
        let pause = if phrase_end { 0.45 } else { 0.15 };
        s.push_str(&format!(
            "A2:{:.2}={} R:{} ",
            0.25 + 0.15 * n as f64,
            word,
            pause
        ));
    }
    s.push('\n');
    s
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // `patina --params`: the automatable parameter chart, straight from
    // the single source of truth (song::PARAM_DEFS + Param::range) — the
    // same rows the parser, the CC map, and the sweep test read
    if args.iter().any(|a| a == "--params") {
        println!("{:<16} {:>10} {:>10}  {:<5} {}", "name", "min", "max", "curve", "cc");
        for def in song::PARAM_DEFS {
            let (lo, hi, curve) = def.param.range();
            let curve = match curve {
                song::Curve::Lin => "lin",
                song::Curve::Log => "log",
                song::Curve::Step => "step",
            };
            let cc = def.cc.map(|c| c.to_string()).unwrap_or_default();
            println!("{:<16} {:>10} {:>10}  {:<5} {}", def.name, lo, hi, curve, cc);
        }
        return Ok(());
    }

    // `patina --say "HH-AH-L-OW W-ER-L-D" [--out say.wav]`: render the
    // voice box speaking a phrase, no window, no audio device
    if let Some(text) = args
        .iter()
        .position(|a| a == "--say")
        .and_then(|i| args.get(i + 1))
    {
        let out = args
            .iter()
            .position(|a| a == "--out")
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
            .unwrap_or("renders/say.wav");
        let song = song::parse_song_text(&say_song(text))?;
        patina::render::render_to_wav(&song, out, true)?;
        return Ok(());
    }

    let song_path = args
        .iter()
        .position(|a| a == "--play")
        .and_then(|i| args.get(i + 1))
        .cloned();
    // No --play given: open silent. Songs only play when asked for.
    let song_path = song_path.as_deref();

    // Offline bounce: no window, no audio device, exits when the file is
    // done. `--no-normalize` keeps the engine's exact gain for
    // measurement renders.
    let render_path = args
        .iter()
        .position(|a| a == "--render")
        .and_then(|i| args.get(i + 1))
        .cloned();
    if let Some(out) = render_path.as_deref() {
        let song = song_path.ok_or("--render requires --play <song.song>")?;
        let events = song::load_song(song)?;
        let normalize = !args.iter().any(|a| a == "--no-normalize");
        patina::render::render_to_wav(&events, out, normalize)?;
        return Ok(());
    }

    // The parsed score as JSON: `--play x.song --export-events out.json`
    let export_path = args
        .iter()
        .position(|a| a == "--export-events")
        .and_then(|i| args.get(i + 1))
        .cloned();
    if let Some(out) = export_path.as_deref() {
        let song = song_path.ok_or("--export-events requires --play <song.song>")?;
        let song = song::load_song(song)?;
        patina::render::export_events(&song, out)?;
        return Ok(());
    }

    // One wav per track: `--play x.song --render-stems <dir>`
    let stems_path = args
        .iter()
        .position(|a| a == "--render-stems")
        .and_then(|i| args.get(i + 1))
        .cloned();
    if let Some(dir) = stems_path.as_deref() {
        let song = song_path.ok_or("--render-stems requires --play <song.song>")?;
        let song = song::load_song(song)?;
        patina::render::render_stems(&song, dir)?;
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");

    println!("Output device: {}", device.name()?);

    // Get all supported configs and find the best one to use
    
    // Preferred formats in order (most preferred first)
    let preferred_formats = [
        (SampleFormat::F32, 48000),
        (SampleFormat::I16, 48000),
        (SampleFormat::F32, 44100),
        (SampleFormat::I16, 44100),
    ];
    
    // Walk the preference list in order (outer loop) so the MOST preferred
    // format wins, not just the first device config that matches any of them
    let mut selected_config = None;
    'search: for &(preferred_format, preferred_rate) in &preferred_formats {
        for supported_config in device.supported_output_configs().expect("error querying configs") {
            if supported_config.sample_format() == preferred_format
                && supported_config.min_sample_rate().0 <= preferred_rate
                && supported_config.max_sample_rate().0 >= preferred_rate
            {
                selected_config =
                    Some(supported_config.with_sample_rate(cpal::SampleRate(preferred_rate)));
                break 'search;
            }
        }
    }

    // Use the device's first config if no preferred one is available
    let supported_config = selected_config.unwrap_or_else(|| {
        device
            .supported_output_configs()
            .expect("error querying configs")
            .next()
            .expect("no supported config found")
            .with_max_sample_rate()
    });
    
    println!("Selected output config: {:?}", supported_config);
    
    let sample_format = supported_config.sample_format();
    let config: cpal::StreamConfig = supported_config.into();

    match sample_format {
        SampleFormat::F32 => run::<f32>(&device, &config, song_path)?,
        SampleFormat::I16 => run::<i16>(&device, &config, song_path)?,
        SampleFormat::U16 => run::<u16>(&device, &config, song_path)?,
        SampleFormat::U8 => run::<u8>(&device, &config, song_path)?,
        SampleFormat::I8 => run::<i8>(&device, &config, song_path)?,
        _ => {
            println!("Unsupported sample format: {:?}, trying to use a different format...", sample_format);
            
            // Try to find a supported format
            let mut configs = device.supported_output_configs()
                .expect("error while querying configs");
            
            while let Some(config) = configs.next() {
                let format = config.sample_format();
                if format == SampleFormat::F32 || format == SampleFormat::I16 || 
                   format == SampleFormat::U16 || format == SampleFormat::U8 || 
                   format == SampleFormat::I8 {
                    let stream_config = config.with_max_sample_rate().into();
                    println!("Trying alternative config: {:?}", config);
                    
                    match format {
                        SampleFormat::F32 => return run::<f32>(&device, &stream_config, song_path),
                        SampleFormat::I16 => return run::<i16>(&device, &stream_config, song_path),
                        SampleFormat::U16 => return run::<u16>(&device, &stream_config, song_path),
                        SampleFormat::U8 => return run::<u8>(&device, &stream_config, song_path),
                        SampleFormat::I8 => return run::<i8>(&device, &stream_config, song_path),
                        _ => continue,
                    }
                }
            }
            
            panic!("Could not find any usable audio configuration");
        }
    }

    Ok(())
}
