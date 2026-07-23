// Native Audio Unit (AUv2) front end — no wrapper library, just the
// AudioComponent ABI implemented directly. Logic Pro, GarageBand, and every
// other AU host talk to this through the factory function at the bottom;
// the parameter surface comes from the shared table in src/host_params.rs,
// so the AU exposes exactly what the CLAP/VST3 plugin exposes.
//
// The component is a MusicDevice ('aumu'): zero input buses, one stereo
// output bus, MIDI in via MusicDeviceMIDIEvent/StartNote. State save and
// restore (kAudioUnitProperty_ClassInfo) round-trips every parameter, which
// is what Logic stores in project files and .aupreset documents.
//
// Bundle with: scripts/bundle-au.sh  (validates with `auval -v aumu Ptna Saur`)

// The Apple constant vocabulary is kept verbatim (kAudioUnit...) so it can
// be grepped against the SDK headers.
#![allow(non_upper_case_globals)]

mod cocoa;
mod ffi;
mod raster;

use ffi::*;
use parking_lot::Mutex;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::host_params::{
    self, ChoiceDef, Display, ParamDef, NUM_VOICES, PITCH_BEND_SEMITONES,
};
use crate::voice_manager::VoiceManager;

pub const AU_TYPE: u32 = fourcc(b"aumu");
pub const AU_SUBTYPE: u32 = fourcc(b"Ptna");
pub const AU_MANUFACTURER: u32 = fourcc(b"Saur");

/// Spring + plate reverb decay; what we report for kAudioUnitProperty_TailTime.
const TAIL_SECONDS: f64 = 4.0;
/// CoreAudio's kAUDefaultMaxFramesPerSlice.
const DEFAULT_MAX_FRAMES: u32 = 1156;

// ---------------------------------------------------------------------------
// Instance state
// ---------------------------------------------------------------------------

struct Listener {
    prop: AudioUnitPropertyID,
    proc_: AudioUnitPropertyListenerProc,
    data: *mut c_void,
}
unsafe impl Send for Listener {}

struct RenderNotify {
    proc_: AURenderCallback,
    data: *mut c_void,
}
unsafe impl Send for RenderNotify {}

/// Everything the render thread owns once the unit is initialized.
struct Engine {
    vm: VoiceManager,
    /// Last value pushed through each table entry; NaN forces the first
    /// application (same guarded-setter scheme as the CLAP front end).
    applied: Vec<f32>,
    /// Backing storage handed to hosts that pass null mData pointers.
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    silence: Vec<f32>,
}

struct UnitState {
    sample_rate: f64,
    max_frames: u32,
    initialized: bool,
    engine: Option<Engine>,
    preset_number: i32,
    preset_name: String,
    last_render_error: OSStatus,
    offline_render: u32,
    should_allocate: u32,
    host_callbacks: Vec<u8>,
}

struct AuUnit {
    /// The host-side AudioComponentInstance, passed back to property listeners.
    comp_instance: usize,
    defs: Vec<ParamDef>,
    /// Current value of each table entry (f32 bits), indexed like `defs`.
    /// The entry index IS the AudioUnitParameterID.
    values: Vec<AtomicU32>,
    state: Mutex<UnitState>,
    listeners: Mutex<Vec<Listener>>,
    notifies: Mutex<Vec<RenderNotify>>,
}

unsafe impl Send for AuUnit {}
unsafe impl Sync for AuUnit {}

impl AuUnit {
    fn new(comp_instance: AudioComponentInstance) -> Self {
        let defs = host_params::param_defs();
        let values =
            defs.iter().map(|d| AtomicU32::new(d.default_value().to_bits())).collect();
        Self {
            comp_instance: comp_instance as usize,
            defs,
            values,
            state: Mutex::new(UnitState {
                sample_rate: 44100.0,
                max_frames: DEFAULT_MAX_FRAMES,
                initialized: false,
                engine: None,
                preset_number: -1,
                preset_name: "Untitled".to_string(),
                last_render_error: noErr,
                offline_render: 0,
                should_allocate: 1,
                host_callbacks: Vec::new(),
            }),
            listeners: Mutex::new(Vec::new()),
            notifies: Mutex::new(Vec::new()),
        }
    }

    fn value(&self, id: usize) -> f32 {
        f32::from_bits(self.values[id].load(Ordering::Relaxed))
    }

    fn set_value(&self, id: usize, v: f32) {
        self.values[id].store(v.to_bits(), Ordering::Relaxed);
    }

    /// Push current parameter values into the engine (render thread, or
    /// Initialize while the state lock is held).
    fn apply_params(defs: &[ParamDef], values: &[AtomicU32], engine: &mut Engine) {
        for (i, def) in defs.iter().enumerate() {
            let value = f32::from_bits(values[i].load(Ordering::Relaxed));
            match def {
                ParamDef::Float(fd) => {
                    if !fd.guarded || value != engine.applied[i] {
                        (fd.apply)(&mut engine.vm, value);
                        engine.applied[i] = value;
                    }
                }
                // Selectors swap voice banks — strictly change-only
                ParamDef::Choice(cd) => {
                    if value != engine.applied[i] {
                        (cd.apply)(&mut engine.vm, value.round().max(0.0) as usize);
                        engine.applied[i] = value;
                    }
                }
            }
        }
    }

    /// Notify property listeners registered for `prop`. Never called with
    /// any internal lock held.
    fn notify_property(&self, prop: AudioUnitPropertyID, scope: AudioUnitScope, elem: u32) {
        let procs: Vec<(AudioUnitPropertyListenerProc, usize)> = self
            .listeners
            .lock()
            .iter()
            .filter(|l| l.prop == prop)
            .map(|l| (l.proc_, l.data as usize))
            .collect();
        for (proc_, data) in procs {
            unsafe {
                proc_(data as *mut c_void, self.comp_instance as *mut c_void, prop, scope, elem)
            };
        }
    }

    // --- ClassInfo (preset/state) serialization -----------------------------

    /// One line per table entry: `<id> <value>`. `{:?}` on f32 round-trips
    /// exactly through str::parse.
    fn serialize_params(&self) -> String {
        let mut out = String::new();
        for (i, def) in self.defs.iter().enumerate() {
            out.push_str(def.id());
            out.push(' ');
            out.push_str(&format!("{:?}\n", self.value(i)));
        }
        out
    }

    fn save_state(&self) -> CFMutableDictionaryRef {
        let (number, name) = {
            let st = self.state.lock();
            (st.preset_number, st.preset_name.clone())
        };
        unsafe {
            let dict = CFDictionaryCreateMutable(
                kCFAllocatorDefault,
                0,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            let put_num = |key: &str, v: i32| {
                let k = cfstring(key);
                let n = cfnumber_i32(v);
                CFDictionarySetValue(dict, k, n);
                CFRelease(k);
                CFRelease(n);
            };
            put_num("version", 0);
            put_num("type", AU_TYPE as i32);
            put_num("subtype", AU_SUBTYPE as i32);
            put_num("manufacturer", AU_MANUFACTURER as i32);
            put_num("preset-number", number);

            let k = cfstring("name");
            let v = cfstring(&name);
            CFDictionarySetValue(dict, k, v);
            CFRelease(k);
            CFRelease(v);

            let blob = self.serialize_params();
            let k = cfstring("data");
            let d = CFDataCreate(kCFAllocatorDefault, blob.as_ptr(), blob.len() as CFIndex);
            CFDictionarySetValue(dict, k, d);
            CFRelease(k);
            CFRelease(d);

            dict
        }
    }

    fn restore_state(&self, plist: CFPropertyListRef) -> OSStatus {
        unsafe {
            if plist.is_null() || CFGetTypeID(plist) != CFDictionaryGetTypeID() {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let get = |key: &str| -> CFTypeRef {
                let k = cfstring(key);
                let v = CFDictionaryGetValue(plist, k);
                CFRelease(k);
                v
            };
            let matches = |key: &str, expect: u32| {
                cfnumber_to_i32(get(key)) == Some(expect as i32)
            };
            if !matches("type", AU_TYPE)
                || !matches("subtype", AU_SUBTYPE)
                || !matches("manufacturer", AU_MANUFACTURER)
            {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // A preset always describes the complete surface: reset to
            // defaults, then lay the saved values on top.
            for (i, def) in self.defs.iter().enumerate() {
                self.set_value(i, def.default_value());
            }
            let data = get("data");
            if !data.is_null() && CFGetTypeID(data) == CFDataGetTypeID() {
                let bytes = std::slice::from_raw_parts(
                    CFDataGetBytePtr(data),
                    CFDataGetLength(data) as usize,
                );
                if let Ok(text) = std::str::from_utf8(bytes) {
                    for line in text.lines() {
                        let mut parts = line.splitn(2, ' ');
                        let (Some(id), Some(raw)) = (parts.next(), parts.next()) else {
                            continue;
                        };
                        let Ok(v) = raw.trim().parse::<f32>() else { continue };
                        if let Some(idx) = self.defs.iter().position(|d| d.id() == id) {
                            self.set_value(idx, v);
                        }
                    }
                }
            }

            {
                let mut st = self.state.lock();
                if let Some(n) = cfnumber_to_i32(get("preset-number")) {
                    st.preset_number = n;
                }
                let name = get("name");
                if !name.is_null() && CFGetTypeID(name) == CFStringGetTypeID() {
                    st.preset_name = cfstring_to_string(name);
                }
            }
            noErr
        }
    }
}

// ---------------------------------------------------------------------------
// Property plumbing
// ---------------------------------------------------------------------------

/// Scope/element check for the unit's fixed topology: one global element,
/// zero inputs, one stereo output.
fn check_element(scope: AudioUnitScope, elem: u32) -> OSStatus {
    match scope {
        kAudioUnitScope_Global | kAudioUnitScope_Output => {
            if elem == 0 {
                noErr
            } else {
                kAudioUnitErr_InvalidElement
            }
        }
        kAudioUnitScope_Input => kAudioUnitErr_InvalidElement,
        _ => kAudioUnitErr_InvalidScope,
    }
}

fn param_unit_and_flags(def: &ParamDef) -> (u32, u32) {
    let base = kAudioUnitParameterFlag_IsReadable
        | kAudioUnitParameterFlag_IsWritable
        | kAudioUnitParameterFlag_HasCFNameString
        | kAudioUnitParameterFlag_CFNameRelease;
    match def {
        ParamDef::Choice(_) => {
            (kAudioUnitParameterUnit_Indexed, base | kAudioUnitParameterFlag_ValuesHaveStrings)
        }
        ParamDef::Float(fd) => match fd.display {
            Display::Percent => (kAudioUnitParameterUnit_Percent, base),
            Display::Fraction => (kAudioUnitParameterUnit_Generic, base),
            Display::Seconds => {
                (kAudioUnitParameterUnit_Seconds, base | kAudioUnitParameterFlag_DisplayLogarithmic)
            }
            Display::Hertz => {
                (kAudioUnitParameterUnit_Hertz, base | kAudioUnitParameterFlag_DisplayLogarithmic)
            }
            Display::Plain(unit) => match unit.trim() {
                "ct" => (kAudioUnitParameterUnit_Cents, base),
                "oct" => (kAudioUnitParameterUnit_Octaves, base),
                _ => (kAudioUnitParameterUnit_Generic, base),
            },
        },
    }
}

fn fill_parameter_info(def: &ParamDef, info: &mut AudioUnitParameterInfo) {
    let (name, min, max, default) = match def {
        ParamDef::Float(fd) => (fd.name, fd.min, fd.max, fd.default),
        ParamDef::Choice(cd) => {
            (cd.name, 0.0, (cd.variants.len() - 1) as f32, cd.default as f32)
        }
    };
    let bytes = name.as_bytes();
    let n = bytes.len().min(51);
    for (i, b) in bytes[..n].iter().enumerate() {
        info.name[i] = *b as std::ffi::c_char;
    }
    info.name[n] = 0;
    info.cfNameString = cfstring(name);
    let (unit, flags) = param_unit_and_flags(def);
    info.unit = unit;
    info.minValue = min;
    info.maxValue = max;
    info.defaultValue = default;
    info.flags = flags;
}

/// (byte size, writable) for every property we implement.
fn property_info(
    unit: &AuUnit,
    prop: AudioUnitPropertyID,
    scope: AudioUnitScope,
    elem: u32,
) -> Result<(usize, bool), OSStatus> {
    let global_only = |size: usize, writable: bool| {
        if scope != kAudioUnitScope_Global {
            Err(kAudioUnitErr_InvalidScope)
        } else if elem != 0 {
            Err(kAudioUnitErr_InvalidElement)
        } else {
            Ok((size, writable))
        }
    };
    match prop {
        kAudioUnitProperty_ClassInfo => global_only(size_of::<CFPropertyListRef>(), true),
        kAudioUnitProperty_SampleRate => match check_element(scope, elem) {
            noErr => Ok((size_of::<f64>(), true)),
            err => Err(err),
        },
        kAudioUnitProperty_ParameterList => match scope {
            kAudioUnitScope_Global => Ok((unit.defs.len() * 4, false)),
            kAudioUnitScope_Input | kAudioUnitScope_Output => Ok((0, false)),
            _ => Err(kAudioUnitErr_InvalidScope),
        },
        kAudioUnitProperty_ParameterInfo => {
            if scope != kAudioUnitScope_Global {
                Err(kAudioUnitErr_InvalidScope)
            } else if (elem as usize) < unit.defs.len() {
                Ok((size_of::<AudioUnitParameterInfo>(), false))
            } else {
                Err(kAudioUnitErr_InvalidParameter)
            }
        }
        kAudioUnitProperty_ParameterValueStrings => {
            match unit.defs.get(elem as usize) {
                Some(ParamDef::Choice(_)) if scope == kAudioUnitScope_Global => {
                    Ok((size_of::<CFArrayRef>(), false))
                }
                _ => Err(kAudioUnitErr_InvalidProperty),
            }
        }
        kAudioUnitProperty_ParameterStringFromValue => {
            global_only(size_of::<AudioUnitParameterStringFromValue>(), false)
        }
        kAudioUnitProperty_ParameterValueFromString => {
            global_only(size_of::<AudioUnitParameterValueFromString>(), false)
        }
        kAudioUnitProperty_StreamFormat => match check_element(scope, elem) {
            noErr => Ok((size_of::<AudioStreamBasicDescription>(), true)),
            err => Err(err),
        },
        kAudioUnitProperty_ElementCount => Ok((4, false)),
        kAudioUnitProperty_Latency => global_only(size_of::<f64>(), false),
        kAudioUnitProperty_SupportedNumChannels => {
            global_only(size_of::<AUChannelInfo>(), false)
        }
        kAudioUnitProperty_MaximumFramesPerSlice => global_only(4, true),
        kAudioUnitProperty_TailTime => global_only(size_of::<f64>(), false),
        kAudioUnitProperty_LastRenderError => global_only(4, false),
        kAudioUnitProperty_HostCallbacks => {
            let stored = unit.state.lock().host_callbacks.len();
            global_only(stored.max(32), true)
        }
        kAudioUnitProperty_PresentPreset | kAudioUnitProperty_CurrentPreset => {
            global_only(size_of::<AUPreset>(), true)
        }
        kAudioUnitProperty_OfflineRender => global_only(4, true),
        kAudioUnitProperty_ShouldAllocateBuffer => match check_element(scope, elem) {
            noErr => Ok((4, true)),
            err => Err(err),
        },
        kMusicDeviceProperty_InstrumentCount => global_only(4, false),
        kAudioUnitProperty_CocoaUI => global_only(size_of::<AudioUnitCocoaViewInfo>(), false),
        cocoa::PROP_PATINA_UNIT => global_only(size_of::<[u64; 2]>(), false),
        _ => Err(kAudioUnitErr_InvalidProperty),
    }
}

/// Copy `value` out to the host, honoring the size-query convention
/// (null outData) and truncating writes like AUBase does.
unsafe fn write_out<T: Copy>(
    value: T,
    out_data: *mut c_void,
    io_size: *mut u32,
) -> OSStatus {
    write_out_bytes(
        std::slice::from_raw_parts(&value as *const T as *const u8, size_of::<T>()),
        out_data,
        io_size,
    )
}

unsafe fn write_out_bytes(bytes: &[u8], out_data: *mut c_void, io_size: *mut u32) -> OSStatus {
    if io_size.is_null() {
        return kAudio_ParamError;
    }
    if out_data.is_null() {
        *io_size = bytes.len() as u32;
        return noErr;
    }
    let n = (*io_size as usize).min(bytes.len());
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_data as *mut u8, n);
    *io_size = n as u32;
    noErr
}

// ---------------------------------------------------------------------------
// Component dispatch
// ---------------------------------------------------------------------------

/// The block the factory hands to the host. The interface must be the first
/// field: the host calls through `iface` and we recover the wrapper by
/// pointer identity.
#[repr(C)]
struct Wrapper {
    iface: AudioComponentPlugInInterface,
    unit: Option<Box<AuUnit>>,
}

unsafe fn wrapper<'a>(this: *mut c_void) -> &'a mut Wrapper {
    &mut *(this as *mut Wrapper)
}

/// The unit, or the "open never happened" error.
unsafe fn unit<'a>(this: *mut c_void) -> Result<&'a AuUnit, OSStatus> {
    wrapper(this).unit.as_deref().ok_or(kAudioUnitErr_Uninitialized)
}

unsafe extern "C" fn au_open(this: *mut c_void, instance: AudioComponentInstance) -> OSStatus {
    wrapper(this).unit = Some(Box::new(AuUnit::new(instance)));
    noErr
}

unsafe extern "C" fn au_close(this: *mut c_void) -> OSStatus {
    drop(Box::from_raw(this as *mut Wrapper));
    noErr
}

unsafe extern "C" fn au_initialize(this: *mut c_void) -> OSStatus {
    let Ok(unit) = unit(this) else { return kAudioUnitErr_FailedInitialization };
    let mut st = unit.state.lock();
    if st.initialized {
        return noErr;
    }
    let max_frames = st.max_frames as usize;
    let mut engine = Engine {
        vm: VoiceManager::new(st.sample_rate as f32, NUM_VOICES),
        applied: vec![f32::NAN; unit.defs.len()],
        out_l: vec![0.0; max_frames],
        out_r: vec![0.0; max_frames],
        silence: vec![0.0; max_frames],
    };
    AuUnit::apply_params(&unit.defs, &unit.values, &mut engine);
    st.engine = Some(engine);
    st.initialized = true;
    noErr
}

unsafe extern "C" fn au_uninitialize(this: *mut c_void) -> OSStatus {
    let Ok(unit) = unit(this) else { return noErr };
    let mut st = unit.state.lock();
    st.initialized = false;
    st.engine = None;
    noErr
}

unsafe extern "C" fn au_get_property_info(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    scope: AudioUnitScope,
    elem: u32,
    out_size: *mut u32,
    out_writable: *mut Boolean,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    match property_info(unit, prop, scope, elem) {
        Ok((size, writable)) => {
            if !out_size.is_null() {
                *out_size = size as u32;
            }
            if !out_writable.is_null() {
                *out_writable = writable as Boolean;
            }
            noErr
        }
        Err(e) => e,
    }
}

unsafe extern "C" fn au_get_property(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    scope: AudioUnitScope,
    elem: u32,
    out_data: *mut c_void,
    io_size: *mut u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    // Validates scope/element and rejects unknown properties in one place.
    if let Err(e) = property_info(unit, prop, scope, elem) {
        return e;
    }
    match prop {
        kAudioUnitProperty_ClassInfo => write_out(unit.save_state(), out_data, io_size),
        kAudioUnitProperty_SampleRate => {
            write_out(unit.state.lock().sample_rate, out_data, io_size)
        }
        kAudioUnitProperty_ParameterList => {
            let ids: Vec<u32> = if scope == kAudioUnitScope_Global {
                (0..unit.defs.len() as u32).collect()
            } else {
                Vec::new()
            };
            let bytes = std::slice::from_raw_parts(ids.as_ptr() as *const u8, ids.len() * 4);
            write_out_bytes(bytes, out_data, io_size)
        }
        kAudioUnitProperty_ParameterInfo => {
            let mut info: AudioUnitParameterInfo = std::mem::zeroed();
            fill_parameter_info(&unit.defs[elem as usize], &mut info);
            write_out(info, out_data, io_size)
        }
        kAudioUnitProperty_ParameterValueStrings => {
            let ParamDef::Choice(ChoiceDef { variants, .. }) = &unit.defs[elem as usize]
            else {
                return kAudioUnitErr_InvalidProperty;
            };
            let strings: Vec<CFTypeRef> = variants.iter().map(|v| cfstring(v)).collect();
            let array = CFArrayCreate(
                kCFAllocatorDefault,
                strings.as_ptr(),
                strings.len() as CFIndex,
                &kCFTypeArrayCallBacks as *const c_void,
            );
            for s in strings {
                CFRelease(s);
            }
            write_out(array, out_data, io_size)
        }
        kAudioUnitProperty_StreamFormat => {
            let sr = unit.state.lock().sample_rate;
            write_out(AudioStreamBasicDescription::non_interleaved_f32(sr, 2), out_data, io_size)
        }
        kAudioUnitProperty_ElementCount => {
            let count: u32 = match scope {
                kAudioUnitScope_Input => 0,
                _ => 1,
            };
            write_out(count, out_data, io_size)
        }
        kAudioUnitProperty_Latency => write_out(0.0f64, out_data, io_size),
        kAudioUnitProperty_SupportedNumChannels => {
            write_out(AUChannelInfo { inChannels: 0, outChannels: 2 }, out_data, io_size)
        }
        kAudioUnitProperty_MaximumFramesPerSlice => {
            write_out(unit.state.lock().max_frames, out_data, io_size)
        }
        kAudioUnitProperty_TailTime => write_out(TAIL_SECONDS, out_data, io_size),
        kAudioUnitProperty_LastRenderError => {
            write_out(unit.state.lock().last_render_error, out_data, io_size)
        }
        kAudioUnitProperty_HostCallbacks => {
            let mut stored = unit.state.lock().host_callbacks.clone();
            stored.resize(stored.len().max(32), 0);
            write_out_bytes(&stored, out_data, io_size)
        }
        kAudioUnitProperty_PresentPreset | kAudioUnitProperty_CurrentPreset => {
            let (number, name) = {
                let st = unit.state.lock();
                (st.preset_number, st.preset_name.clone())
            };
            // The host owns (and releases) the returned name.
            write_out(
                AUPreset { presetNumber: number, presetName: cfstring(&name) },
                out_data,
                io_size,
            )
        }
        kAudioUnitProperty_OfflineRender => {
            write_out(unit.state.lock().offline_render, out_data, io_size)
        }
        kAudioUnitProperty_ShouldAllocateBuffer => {
            write_out(unit.state.lock().should_allocate, out_data, io_size)
        }
        kMusicDeviceProperty_InstrumentCount => write_out(0u32, out_data, io_size),
        kAudioUnitProperty_CocoaUI => match cocoa::cocoa_view_info() {
            Some((bundle_url, class_name)) => write_out(
                AudioUnitCocoaViewInfo {
                    mCocoaAUViewBundleLocation: bundle_url,
                    mCocoaAUViewClass: [class_name],
                },
                out_data,
                io_size,
            ),
            // No resolvable bundle -> the host uses its generic view
            None => kAudioUnitErr_InvalidProperty,
        },
        // The in-process handshake with our own Cocoa view (see au/cocoa.rs):
        // the pointer is only meaningful inside this process, so it travels
        // with the pid for the view to verify.
        cocoa::PROP_PATINA_UNIT => {
            let handshake = [unit as *const AuUnit as u64, std::process::id() as u64];
            write_out(handshake, out_data, io_size)
        }
        // In/out queries: the host passes the struct in outData with its
        // input fields filled and we complete the out field in place.
        kAudioUnitProperty_ParameterStringFromValue => {
            if out_data.is_null() {
                *io_size = size_of::<AudioUnitParameterStringFromValue>() as u32;
                return noErr;
            }
            let query = &mut *(out_data as *mut AudioUnitParameterStringFromValue);
            let Some(ParamDef::Choice(cd)) = unit.defs.get(query.inParamID as usize) else {
                return kAudioUnitErr_InvalidParameter;
            };
            let value = if query.inValue.is_null() {
                unit.value(query.inParamID as usize)
            } else {
                *query.inValue
            };
            let idx = (value.round().max(0.0) as usize).min(cd.variants.len() - 1);
            query.outString = cfstring(cd.variants[idx]);
            noErr
        }
        kAudioUnitProperty_ParameterValueFromString => {
            if out_data.is_null() {
                *io_size = size_of::<AudioUnitParameterValueFromString>() as u32;
                return noErr;
            }
            let query = &mut *(out_data as *mut AudioUnitParameterValueFromString);
            let Some(ParamDef::Choice(cd)) = unit.defs.get(query.inParamID as usize) else {
                return kAudioUnitErr_InvalidParameter;
            };
            let wanted = cfstring_to_string(query.inString);
            match cd.variants.iter().position(|v| *v == wanted) {
                Some(idx) => {
                    query.outValue = idx as f32;
                    noErr
                }
                None => kAudioUnitErr_InvalidPropertyValue,
            }
        }
        _ => kAudioUnitErr_InvalidProperty,
    }
}

unsafe extern "C" fn au_set_property(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    scope: AudioUnitScope,
    elem: u32,
    in_data: *const c_void,
    in_size: u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let (_, writable) = match property_info(unit, prop, scope, elem) {
        Ok(info) => info,
        Err(e) => return e,
    };
    if !writable {
        return kAudioUnitErr_PropertyNotWritable;
    }
    if in_data.is_null() {
        return kAudio_ParamError;
    }
    match prop {
        kAudioUnitProperty_ClassInfo => {
            if (in_size as usize) < size_of::<CFPropertyListRef>() {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let status = unit.restore_state(*(in_data as *const CFPropertyListRef));
            if status == noErr {
                unit.notify_property(
                    kAudioUnitProperty_PresentPreset,
                    kAudioUnitScope_Global,
                    0,
                );
            }
            status
        }
        kAudioUnitProperty_SampleRate => {
            if (in_size as usize) < size_of::<f64>() {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let rate = *(in_data as *const f64);
            if !(rate.is_finite() && (1.0..=1_000_000.0).contains(&rate)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            {
                let mut st = unit.state.lock();
                if st.initialized {
                    return kAudioUnitErr_Initialized;
                }
                st.sample_rate = rate;
            }
            unit.notify_property(kAudioUnitProperty_SampleRate, scope, elem);
            unit.notify_property(kAudioUnitProperty_StreamFormat, scope, elem);
            noErr
        }
        kAudioUnitProperty_StreamFormat => {
            if (in_size as usize) < size_of::<AudioStreamBasicDescription>() {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let asbd = *(in_data as *const AudioStreamBasicDescription);
            let float_flag = asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0;
            if asbd.mFormatID != kAudioFormatLinearPCM
                || !float_flag
                || asbd.mBitsPerChannel != 32
                || asbd.mChannelsPerFrame != 2
                || !(asbd.mSampleRate.is_finite() && asbd.mSampleRate > 0.0)
            {
                return kAudioUnitErr_FormatNotSupported;
            }
            {
                let mut st = unit.state.lock();
                if st.initialized {
                    return kAudioUnitErr_Initialized;
                }
                st.sample_rate = asbd.mSampleRate;
            }
            unit.notify_property(kAudioUnitProperty_StreamFormat, scope, elem);
            unit.notify_property(kAudioUnitProperty_SampleRate, scope, elem);
            noErr
        }
        kAudioUnitProperty_MaximumFramesPerSlice => {
            if in_size < 4 {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let frames = *(in_data as *const u32);
            if frames == 0 {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            {
                let mut st = unit.state.lock();
                if st.initialized {
                    return kAudioUnitErr_Initialized;
                }
                st.max_frames = frames;
            }
            unit.notify_property(kAudioUnitProperty_MaximumFramesPerSlice, scope, elem);
            noErr
        }
        kAudioUnitProperty_PresentPreset | kAudioUnitProperty_CurrentPreset => {
            if (in_size as usize) < size_of::<AUPreset>() {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            let preset = *(in_data as *const AUPreset);
            // No factory presets, so only user presets (negative numbers)
            // are addressable.
            if preset.presetNumber >= 0 {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            {
                let mut st = unit.state.lock();
                st.preset_number = preset.presetNumber;
                if !preset.presetName.is_null() {
                    st.preset_name = cfstring_to_string(preset.presetName);
                }
            }
            unit.notify_property(kAudioUnitProperty_PresentPreset, kAudioUnitScope_Global, 0);
            noErr
        }
        kAudioUnitProperty_HostCallbacks => {
            let bytes = std::slice::from_raw_parts(in_data as *const u8, in_size as usize);
            unit.state.lock().host_callbacks = bytes.to_vec();
            noErr
        }
        kAudioUnitProperty_OfflineRender => {
            if in_size < 4 {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            unit.state.lock().offline_render = *(in_data as *const u32);
            noErr
        }
        kAudioUnitProperty_ShouldAllocateBuffer => {
            if in_size < 4 {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            unit.state.lock().should_allocate = *(in_data as *const u32);
            noErr
        }
        _ => kAudioUnitErr_InvalidProperty,
    }
}

unsafe extern "C" fn au_add_property_listener(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    proc_: AudioUnitPropertyListenerProc,
    data: *mut c_void,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    unit.listeners.lock().push(Listener { prop, proc_, data });
    noErr
}

unsafe extern "C" fn au_remove_property_listener(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    proc_: AudioUnitPropertyListenerProc,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    unit.listeners
        .lock()
        .retain(|l| !(l.prop == prop && l.proc_ as usize == proc_ as usize));
    noErr
}

unsafe extern "C" fn au_remove_property_listener_with_user_data(
    this: *mut c_void,
    prop: AudioUnitPropertyID,
    proc_: AudioUnitPropertyListenerProc,
    data: *mut c_void,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    unit.listeners.lock().retain(|l| {
        !(l.prop == prop && l.proc_ as usize == proc_ as usize && l.data == data)
    });
    noErr
}

unsafe extern "C" fn au_add_render_notify(
    this: *mut c_void,
    proc_: AURenderCallback,
    data: *mut c_void,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    unit.notifies.lock().push(RenderNotify { proc_, data });
    noErr
}

unsafe extern "C" fn au_remove_render_notify(
    this: *mut c_void,
    proc_: AURenderCallback,
    data: *mut c_void,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    unit.notifies
        .lock()
        .retain(|n| !(n.proc_ as usize == proc_ as usize && n.data == data));
    noErr
}

unsafe extern "C" fn au_get_parameter(
    this: *mut c_void,
    param: AudioUnitParameterID,
    scope: AudioUnitScope,
    _elem: u32,
    out_value: *mut f32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    if scope != kAudioUnitScope_Global {
        return kAudioUnitErr_InvalidScope;
    }
    if (param as usize) >= unit.defs.len() || out_value.is_null() {
        return kAudioUnitErr_InvalidParameter;
    }
    *out_value = unit.value(param as usize);
    noErr
}

unsafe extern "C" fn au_set_parameter(
    this: *mut c_void,
    param: AudioUnitParameterID,
    scope: AudioUnitScope,
    _elem: u32,
    value: f32,
    _buffer_offset: u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    if scope != kAudioUnitScope_Global {
        return kAudioUnitErr_InvalidScope;
    }
    if (param as usize) >= unit.defs.len() {
        return kAudioUnitErr_InvalidParameter;
    }
    unit.set_value(param as usize, value);
    noErr
}

unsafe extern "C" fn au_schedule_parameters(
    this: *mut c_void,
    events: *const AudioUnitParameterEvent,
    num_events: u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    if events.is_null() {
        return kAudio_ParamError;
    }
    for event in std::slice::from_raw_parts(events, num_events as usize) {
        if event.scope != kAudioUnitScope_Global
            || (event.parameter as usize) >= unit.defs.len()
        {
            return kAudioUnitErr_InvalidParameter;
        }
        // Ramps land on their end value; the engine's own smoothing covers
        // the transition.
        unit.set_value(event.parameter as usize, event.target_value());
    }
    noErr
}

unsafe extern "C" fn au_reset(this: *mut c_void, _scope: AudioUnitScope, _elem: u32) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let mut st = unit.state.lock();
    let sample_rate = st.sample_rate;
    if let Some(engine) = st.engine.as_mut() {
        engine.vm = VoiceManager::new(sample_rate as f32, NUM_VOICES);
        engine.applied.fill(f32::NAN);
        AuUnit::apply_params(&unit.defs, &unit.values, engine);
    }
    noErr
}

unsafe extern "C" fn au_render(
    this: *mut c_void,
    io_action_flags: *mut AudioUnitRenderActionFlags,
    in_time_stamp: *const AudioTimeStamp,
    in_bus: u32,
    in_frames: u32,
    io_data: *mut AudioBufferList,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    if io_data.is_null() || in_time_stamp.is_null() {
        return kAudio_ParamError;
    }
    if in_bus != 0 {
        return kAudioUnitErr_InvalidElement;
    }

    let mut local_flags: AudioUnitRenderActionFlags = 0;
    let flags = if io_action_flags.is_null() { &mut local_flags } else { &mut *io_action_flags };

    let notifies: Vec<(AURenderCallback, usize)> =
        unit.notifies.lock().iter().map(|n| (n.proc_, n.data as usize)).collect();
    for (proc_, data) in &notifies {
        let mut f = *flags | kAudioUnitRenderAction_PreRender;
        proc_(*data as *mut c_void, &mut f, in_time_stamp, in_bus, in_frames, io_data);
    }

    let status = {
        let mut st = unit.state.lock();
        if !st.initialized {
            Err(kAudioUnitErr_Uninitialized)
        } else if in_frames > st.max_frames {
            Err(kAudioUnitErr_TooManyFramesToProcess)
        } else {
            let engine = st.engine.as_mut().expect("initialized implies engine");
            AuUnit::apply_params(&unit.defs, &unit.values, engine);

            let frames = in_frames as usize;
            for i in 0..frames {
                let (l, r) = engine.vm.render_next();
                engine.out_l[i] = l;
                engine.out_r[i] = r;
            }

            let buffers = (*io_data).buffers_mut();
            for (bi, buf) in buffers.iter_mut().enumerate() {
                let channels = buf.mNumberChannels.max(1) as usize;
                let samples = frames * channels;
                if buf.mData.is_null() {
                    // Host asked us to supply the memory.
                    let backing = match bi {
                        0 => &mut engine.out_l,
                        1 => &mut engine.out_r,
                        _ => &mut engine.silence,
                    };
                    if backing.len() < samples {
                        backing.resize(samples, 0.0);
                    }
                    buf.mData = backing.as_mut_ptr() as *mut c_void;
                }
                buf.mDataByteSize = (samples * 4) as u32;
                let out = std::slice::from_raw_parts_mut(buf.mData as *mut f32, samples);
                match channels {
                    // Non-interleaved: buffer 0 = left, buffer 1 = right,
                    // anything further is silence.
                    1 => {
                        let src = match bi {
                            0 => &engine.out_l,
                            1 => &engine.out_r,
                            _ => &engine.silence,
                        };
                        // A null-data buffer may already BE the source;
                        // only copy when they differ.
                        if out.as_ptr() != src.as_ptr() {
                            out.copy_from_slice(&src[..frames]);
                        }
                    }
                    // Interleaved stereo in one buffer.
                    2 => {
                        for i in 0..frames {
                            out[2 * i] = engine.out_l[i];
                            out[2 * i + 1] = engine.out_r[i];
                        }
                    }
                    _ => out.fill(0.0),
                }
            }
            Ok(())
        }
    };

    let result = match status {
        Ok(()) => noErr,
        Err(e) => {
            unit.state.lock().last_render_error = e;
            e
        }
    };

    for (proc_, data) in &notifies {
        let mut f = *flags
            | kAudioUnitRenderAction_PostRender
            | if result != noErr { kAudioUnitRenderAction_PostRenderError } else { 0 };
        proc_(*data as *mut c_void, &mut f, in_time_stamp, in_bus, in_frames, io_data);
    }
    result
}

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

unsafe extern "C" fn au_midi_event(
    this: *mut c_void,
    status: u32,
    data1: u32,
    data2: u32,
    _offset_sample_frame: u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let mut st = unit.state.lock();
    let Some(engine) = st.engine.as_mut() else { return noErr };
    let vm = &mut engine.vm;

    let channel = (status & 0x0F) as u8;
    let note = (data1 & 0x7F) as u8;
    match status & 0xF0 {
        0x90 if data2 > 0 => {
            host_params::note_on(vm, channel, note, (data2 & 0x7F) as f32 / 127.0)
        }
        0x80 | 0x90 => host_params::note_off(vm, channel, note),
        0xB0 => match data1 {
            // Mod wheel
            1 => vm.set_mod_wheel((data2 & 0x7F) as f32 / 127.0),
            // Sustain pedal
            64 => vm.set_sustain_pedal(data2 >= 64),
            // All sound off / all notes off
            120 | 123 => {
                vm.set_sustain_pedal(false);
                for n in 0..128u8 {
                    vm.note_off(n);
                }
            }
            _ => (),
        },
        0xE0 => {
            // 14-bit wheel; 0.5 is center, spanning +/-2 semitones
            let raw = ((data2 & 0x7F) << 7 | (data1 & 0x7F)) as f32 / 16383.0;
            vm.set_pitch_bend((raw - 0.5) * 2.0 * PITCH_BEND_SEMITONES);
        }
        _ => (),
    }
    noErr
}

unsafe extern "C" fn au_sysex(_this: *mut c_void, _data: *const u8, _length: u32) -> OSStatus {
    noErr
}

unsafe extern "C" fn au_prepare_instrument(
    _this: *mut c_void,
    _instrument: MusicDeviceInstrumentID,
) -> OSStatus {
    noErr
}

unsafe extern "C" fn au_release_instrument(
    _this: *mut c_void,
    _instrument: MusicDeviceInstrumentID,
) -> OSStatus {
    noErr
}

unsafe extern "C" fn au_start_note(
    this: *mut c_void,
    _instrument: MusicDeviceInstrumentID,
    group: MusicDeviceGroupID,
    out_note_instance: *mut NoteInstanceID,
    _offset_sample_frame: u32,
    params: *const MusicDeviceNoteParams,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    if params.is_null() {
        return kAudio_ParamError;
    }
    let pitch = (*params).mPitch.clamp(0.0, 127.0);
    let velocity = ((*params).mVelocity / 127.0).clamp(0.0, 1.0);
    let note = pitch.round() as u8;
    {
        let mut st = unit.state.lock();
        if let Some(engine) = st.engine.as_mut() {
            host_params::note_on(&mut engine.vm, (group & 0x0F) as u8, note, velocity);
        }
    }
    if !out_note_instance.is_null() {
        *out_note_instance = note as NoteInstanceID;
    }
    noErr
}

unsafe extern "C" fn au_stop_note(
    this: *mut c_void,
    group: MusicDeviceGroupID,
    note_instance: NoteInstanceID,
    _offset_sample_frame: u32,
) -> OSStatus {
    let unit = match unit(this) {
        Ok(u) => u,
        Err(e) => return e,
    };
    let mut st = unit.state.lock();
    if let Some(engine) = st.engine.as_mut() {
        host_params::note_off(&mut engine.vm, (group & 0x0F) as u8, (note_instance & 0x7F) as u8);
    }
    noErr
}

// ---------------------------------------------------------------------------
// Lookup + factory
// ---------------------------------------------------------------------------

macro_rules! method {
    ($f:expr) => {
        Some(std::mem::transmute::<*const (), unsafe extern "C" fn()>($f as *const ()))
    };
}

unsafe extern "C" fn au_lookup(selector: i16) -> AudioComponentMethod {
    match selector {
        kAudioUnitInitializeSelect => method!(au_initialize),
        kAudioUnitUninitializeSelect => method!(au_uninitialize),
        kAudioUnitGetPropertyInfoSelect => method!(au_get_property_info),
        kAudioUnitGetPropertySelect => method!(au_get_property),
        kAudioUnitSetPropertySelect => method!(au_set_property),
        kAudioUnitAddPropertyListenerSelect => method!(au_add_property_listener),
        kAudioUnitRemovePropertyListenerSelect => method!(au_remove_property_listener),
        kAudioUnitRemovePropertyListenerWithUserDataSelect => {
            method!(au_remove_property_listener_with_user_data)
        }
        kAudioUnitAddRenderNotifySelect => method!(au_add_render_notify),
        kAudioUnitRemoveRenderNotifySelect => method!(au_remove_render_notify),
        kAudioUnitGetParameterSelect => method!(au_get_parameter),
        kAudioUnitSetParameterSelect => method!(au_set_parameter),
        kAudioUnitScheduleParametersSelect => method!(au_schedule_parameters),
        kAudioUnitRenderSelect => method!(au_render),
        kAudioUnitResetSelect => method!(au_reset),
        kMusicDeviceMIDIEventSelect => method!(au_midi_event),
        kMusicDeviceSysExSelect => method!(au_sysex),
        kMusicDevicePrepareInstrumentSelect => method!(au_prepare_instrument),
        kMusicDeviceReleaseInstrumentSelect => method!(au_release_instrument),
        kMusicDeviceStartNoteSelect => method!(au_start_note),
        kMusicDeviceStopNoteSelect => method!(au_stop_note),
        _ => None,
    }
}

/// The entry point named by `factoryFunction` in the component's Info.plist.
/// The host frees the returned block by calling Close on it.
#[no_mangle]
pub extern "C" fn PatinaAUFactory(
    _desc: *const AudioComponentDescription,
) -> *mut AudioComponentPlugInInterface {
    let wrapper = Box::new(Wrapper {
        iface: AudioComponentPlugInInterface {
            Open: au_open,
            Close: au_close,
            Lookup: au_lookup,
            reserved: std::ptr::null_mut(),
        },
        unit: None,
    });
    Box::into_raw(wrapper) as *mut AudioComponentPlugInInterface
}
