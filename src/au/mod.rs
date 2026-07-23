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

// The custom egui panel (au/cocoa.rs) rides the `editor` feature, which the
// default `au` build turns on. It survives Logic's out-of-process view host
// by software-rendering through AppKit's drawRect: cycle; see au/cocoa.rs
// for the two subtleties that make that work. Building without `editor`
// drops the custom view and hosts fall back to their generic parameter view.
#[cfg(feature = "editor")]
mod cocoa;
mod ffi;
#[cfg(feature = "editor")]
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
    /// Scratch for a null-mData buffer that wants more than one channel.
    /// It MUST NOT be out_l/out_r: interleaving into out_l while reading
    /// out_l overwrites samples the loop has not consumed yet.
    interleaved: Vec<f32>,
}

struct UnitState {
    sample_rate: f64,
    max_frames: u32,
    /// Present exactly while the unit is initialized. There is deliberately
    /// no separate `initialized` flag: the two could drift, and the render
    /// path would then have to `expect()` an engine that a flag promised —
    /// a panic straight out of a host callback.
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

    /// Store a host-supplied value, forced into the range we advertise.
    ///
    /// This is the ONLY door values come in through (SetParameter,
    /// ScheduleParameters, ClassInfo restore), and a host can push anything
    /// down it: an out-of-range automation curve, or a NaN parsed out of a
    /// corrupted preset blob. NaN reaching a recursive filter's state makes
    /// every later sample NaN for the life of the instance — the plugin
    /// goes permanently silent and the host looks broken — so it never gets
    /// stored in the first place.
    fn set_value(&self, id: usize, v: f32) {
        let def = &self.defs[id];
        let clean = if v.is_nan() {
            def.default_value()
        } else {
            let (min, max) = param_range(def);
            v.clamp(min, max)
        };
        self.values[id].store(clean.to_bits(), Ordering::Relaxed);
    }

    /// Push current parameter values into the engine (render thread, or
    /// Initialize while the state lock is held).
    fn apply_params(defs: &[ParamDef], values: &[AtomicU32], engine: &mut Engine) {
        for (i, def) in defs.iter().enumerate() {
            let value = f32::from_bits(values[i].load(Ordering::Relaxed));
            match def {
                ParamDef::Float(fd) => {
                    if !fd.guarded || value != engine.applied[i] {
                        fd.param.apply(&mut engine.vm, value);
                        engine.applied[i] = value;
                    }
                }
                // Selectors swap voice banks / circuit models — strictly
                // change-only. Param::apply maps the raw value (the selector
                // index as an f32) to the engine enum position.
                ParamDef::Choice(cd) => {
                    if value != engine.applied[i] {
                        cd.param.apply(&mut engine.vm, value);
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
            // CFDataGetBytePtr can be null (empty CFData), and from_raw_parts
            // with a null pointer is UB even for a zero length.
            let ptr = if !data.is_null() && CFGetTypeID(data) == CFDataGetTypeID() {
                CFDataGetBytePtr(data)
            } else {
                std::ptr::null()
            };
            if !ptr.is_null() {
                let len = CFDataGetLength(data).max(0) as usize;
                let bytes = std::slice::from_raw_parts(ptr, len);
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

/// The range we advertise to the host for a table entry — selectors span
/// their variant indices. Used both to fill AudioUnitParameterInfo and to
/// clamp what comes back in, so the two can't disagree.
fn param_range(def: &ParamDef) -> (f32, f32) {
    match def {
        ParamDef::Float(fd) => (fd.min, fd.max),
        ParamDef::Choice(cd) => (0.0, cd.variants.len().saturating_sub(1) as f32),
    }
}

fn fill_parameter_info(def: &ParamDef, info: &mut AudioUnitParameterInfo) {
    let (min, max) = param_range(def);
    let (name, default) = match def {
        ParamDef::Float(fd) => (fd.name, fd.default),
        ParamDef::Choice(cd) => (cd.name, cd.default as f32),
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
        #[cfg(feature = "editor")]
        kAudioUnitProperty_CocoaUI => global_only(size_of::<AudioUnitCocoaViewInfo>(), false),
        #[cfg(feature = "editor")]
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

/// Gate for properties whose value is a freshly created CoreFoundation
/// object (or an in/out struct the host owns): answers the size query and
/// rejects an undersized buffer BEFORE anything is allocated.
///
/// This has to happen first. Hosts routinely call GetProperty with a null
/// outData just to learn the size, and `write_out` returns at that point —
/// so building the CFDictionary/CFString/CFArray/CFURL up front dropped a
/// +1 reference on the floor on every such call. An undersized buffer was
/// worse still: half a pointer copied out AND the object leaked.
///
/// `Some(status)` means "already answered, return it"; `None` means the
/// caller may go ahead and produce the value.
unsafe fn owned_value_gate(
    size: usize,
    out_data: *mut c_void,
    io_size: *mut u32,
) -> Option<OSStatus> {
    if io_size.is_null() {
        return Some(kAudio_ParamError);
    }
    if out_data.is_null() {
        *io_size = size as u32;
        return Some(noErr);
    }
    if (*io_size as usize) < size {
        return Some(kAudio_ParamError);
    }
    None
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
    if st.engine.is_some() {
        return noErr;
    }
    let max_frames = st.max_frames as usize;
    let mut engine = Engine {
        vm: VoiceManager::new(st.sample_rate as f32, NUM_VOICES),
        applied: vec![f32::NAN; unit.defs.len()],
        out_l: vec![0.0; max_frames],
        out_r: vec![0.0; max_frames],
        silence: vec![0.0; max_frames],
        interleaved: vec![0.0; max_frames * 2],
    };
    AuUnit::apply_params(&unit.defs, &unit.values, &mut engine);
    st.engine = Some(engine);
    noErr
}

unsafe extern "C" fn au_uninitialize(this: *mut c_void) -> OSStatus {
    let Ok(unit) = unit(this) else { return noErr };
    // Tear the engine down outside the lock: freeing the voice bank and the
    // reverb tails is not something the render thread should ever wait on.
    let engine = unit.state.lock().engine.take();
    drop(engine);
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
    // Properties whose value is a +1 CoreFoundation object (or an in/out
    // struct written in place) must settle size queries and undersized
    // buffers before they build anything — see `owned_value_gate`.
    macro_rules! gate {
        ($t:ty) => {
            if let Some(status) = owned_value_gate(size_of::<$t>(), out_data, io_size) {
                return status;
            }
        };
    }
    match prop {
        kAudioUnitProperty_ClassInfo => {
            gate!(CFPropertyListRef);
            write_out(unit.save_state(), out_data, io_size)
        }
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
            gate!(AudioUnitParameterInfo);
            let mut info: AudioUnitParameterInfo = std::mem::zeroed();
            fill_parameter_info(&unit.defs[elem as usize], &mut info);
            write_out(info, out_data, io_size)
        }
        kAudioUnitProperty_ParameterValueStrings => {
            gate!(CFArrayRef);
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
            gate!(AUPreset);
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
        #[cfg(feature = "editor")]
        kAudioUnitProperty_CocoaUI => {
            gate!(AudioUnitCocoaViewInfo);
            match cocoa::cocoa_view_info() {
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
            }
        }
        // The in-process handshake with our own Cocoa view (see au/cocoa.rs).
        #[cfg(feature = "editor")]
        cocoa::PROP_PATINA_UNIT => {
            let handshake = [unit as *const AuUnit as u64, std::process::id() as u64];
            write_out(handshake, out_data, io_size)
        }
        // In/out queries: the host passes the struct in outData with its
        // input fields filled and we complete the out field in place.
        kAudioUnitProperty_ParameterStringFromValue => {
            gate!(AudioUnitParameterStringFromValue);
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
            gate!(AudioUnitParameterValueFromString);
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
            // Refuse rates the engine cannot be built at rather than
            // running its fixed-Hz resonators above Nyquist, where they
            // diverge (see crate::MIN_SAMPLE_RATE)
            if !crate::supported_sample_rate(rate) {
                return kAudioUnitErr_FormatNotSupported;
            }
            {
                let mut st = unit.state.lock();
                if st.engine.is_some() {
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
                || !crate::supported_sample_rate(asbd.mSampleRate)
            {
                return kAudioUnitErr_FormatNotSupported;
            }
            {
                let mut st = unit.state.lock();
                if st.engine.is_some() {
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
                if st.engine.is_some() {
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
    // Reset arrives on the main thread (Logic sends it on every transport
    // stop and locate) while the audio thread is still rendering and
    // contending for this very lock. Building the replacement voice bank and
    // freeing the old one are both allocator work, so they happen OUTSIDE
    // the lock; only the swap is done under it.
    let sample_rate = unit.state.lock().sample_rate;
    let fresh = VoiceManager::new(sample_rate as f32, NUM_VOICES);
    let mut retired = None;
    {
        let mut st = unit.state.lock();
        if let Some(engine) = st.engine.as_mut() {
            retired = Some(std::mem::replace(&mut engine.vm, fresh));
            engine.applied.fill(f32::NAN);
            AuUnit::apply_params(&unit.defs, &unit.values, engine);
        }
    }
    drop(retired);
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

    // ioActionFlags is optional; the notify callbacks each get their own
    // copy so a host cannot have one of them rewrite what the next sees.
    let base_flags: AudioUnitRenderActionFlags =
        if io_action_flags.is_null() { 0 } else { *io_action_flags };
    for_each_render_notify(unit, |proc_, data| {
        let mut f = base_flags | kAudioUnitRenderAction_PreRender;
        proc_(data, &mut f, in_time_stamp, in_bus, in_frames, io_data);
    });

    let status = {
        let mut st = unit.state.lock();
        let max_frames = st.max_frames;
        match st.engine.as_mut() {
            None => Err(kAudioUnitErr_Uninitialized),
            Some(_) if in_frames > max_frames => Err(kAudioUnitErr_TooManyFramesToProcess),
            Some(engine) => {
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
                        // Host asked us to supply the memory. A one-channel
                        // buffer can point straight at the matching mix; a
                        // multi-channel one must NOT, because the interleave
                        // below reads out_l/out_r while writing the buffer.
                        let backing = if channels == 1 {
                            match bi {
                                0 => &mut engine.out_l,
                                1 => &mut engine.out_r,
                                _ => &mut engine.silence,
                            }
                        } else {
                            &mut engine.interleaved
                        };
                        if backing.len() < samples {
                            backing.resize(samples, 0.0);
                        }
                        buf.mData = backing.as_mut_ptr() as *mut c_void;
                    }
                    buf.mDataByteSize = (samples * 4) as u32;
                    // Raw pointers, not slices: in the mono null-mData case the
                    // destination IS one of the source vectors, and holding a
                    // &mut [f32] and a &[f32] over the same bytes is UB even
                    // when the copy is skipped.
                    let dst = buf.mData as *mut f32;
                    match channels {
                        // Non-interleaved: buffer 0 = left, buffer 1 = right,
                        // anything further is silence.
                        1 => {
                            let src = match bi {
                                0 => engine.out_l.as_ptr(),
                                1 => engine.out_r.as_ptr(),
                                _ => engine.silence.as_ptr(),
                            };
                            // A null-data buffer may already BE the source;
                            // only copy when they differ.
                            if dst as *const f32 != src {
                                std::ptr::copy_nonoverlapping(src, dst, frames);
                            }
                        }
                        // Interleaved stereo in one buffer.
                        2 => {
                            for i in 0..frames {
                                *dst.add(2 * i) = engine.out_l[i];
                                *dst.add(2 * i + 1) = engine.out_r[i];
                            }
                        }
                        _ => std::ptr::write_bytes(dst, 0, samples),
                    }
                }
                Ok(())
            }
        }
    };

    let result = match status {
        Ok(()) => noErr,
        Err(e) => {
            unit.state.lock().last_render_error = e;
            e
        }
    };

    for_each_render_notify(unit, |proc_, data| {
        let mut f = base_flags
            | kAudioUnitRenderAction_PostRender
            | if result != noErr { kAudioUnitRenderAction_PostRenderError } else { 0 };
        proc_(data, &mut f, in_time_stamp, in_bus, in_frames, io_data);
    });
    result
}

/// Call `f` for every registered render-notify callback.
///
/// Two constraints meet here, and both are load-bearing on the audio thread:
/// the host must not be called back with our lock held (any callback that
/// touches the unit would deadlock), and the render path must not allocate
/// (the old `collect()` into a Vec ran a malloc per block for every host
/// that registers a notify — Logic does). So the list is copied out through
/// a fixed stack window, in as many passes as it takes; nothing is dropped
/// and nothing is heap-allocated.
unsafe fn for_each_render_notify(
    unit: &AuUnit,
    mut f: impl FnMut(AURenderCallback, *mut c_void),
) {
    const WINDOW: usize = 8;
    let mut start = 0usize;
    loop {
        let mut window: [Option<(AURenderCallback, usize)>; WINDOW] = [None; WINDOW];
        let mut count = 0usize;
        {
            let list = unit.notifies.lock();
            for entry in list.iter().skip(start).take(WINDOW) {
                window[count] = Some((entry.proc_, entry.data as usize));
                count += 1;
            }
        }
        for slot in window.iter().take(count) {
            if let Some((proc_, data)) = *slot {
                f(proc_, data as *mut c_void);
            }
        }
        if count < WINDOW {
            return;
        }
        start += WINDOW;
    }
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

// ---------------------------------------------------------------------------
// Host-ABI regression tests
//
// These drive the component the way a host does — through the very dispatch
// functions `au_lookup` hands out — because every bug this file has shipped
// came from a call shape we had not exercised.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    /// An opened unit; dropping it Closes, like the host does.
    struct TestUnit(*mut c_void);

    impl TestUnit {
        fn open() -> Self {
            let iface = PatinaAUFactory(ptr::null());
            let this = iface as *mut c_void;
            unsafe { assert_eq!(au_open(this, 0xBEEF as *mut c_void), noErr) };
            TestUnit(this)
        }

        fn initialized() -> Self {
            let u = Self::open();
            unsafe { assert_eq!(au_initialize(u.0), noErr) };
            u
        }

        fn unit(&self) -> &AuUnit {
            unsafe { unit(self.0).unwrap() }
        }
    }

    impl Drop for TestUnit {
        fn drop(&mut self) {
            unsafe { au_close(self.0) };
        }
    }

    fn zero_timestamp() -> AudioTimeStamp {
        unsafe { std::mem::zeroed() }
    }

    /// One interleaved-stereo buffer whose storage the unit has to supply.
    ///
    /// This is the shape that used to corrupt the mix: the null buffer was
    /// backed by `out_l`, so interleaving into it clobbered `out_l[1..]`
    /// before the loop had read those samples — and aliased a `&mut [f32]`
    /// onto a live `&[f32]` while doing it.
    #[test]
    fn interleaved_null_buffer_does_not_alias_the_mix() {
        const FRAMES: u32 = 64;
        let u = TestUnit::initialized();
        unsafe {
            // A held note, run past the amp attack, so the block is not silence.
            assert_eq!(au_midi_event(u.0, 0x90, 69, 100, 0), noErr);
            let ts = zero_timestamp();
            let mut scratch = vec![0.0f32; 2 * FRAMES as usize];
            for _ in 0..60 {
                let mut abl = AudioBufferList {
                    mNumberBuffers: 1,
                    mBuffers: [AudioBuffer {
                        mNumberChannels: 2,
                        mDataByteSize: (scratch.len() * 4) as u32,
                        mData: scratch.as_mut_ptr() as *mut c_void,
                    }],
                };
                assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, FRAMES, &mut abl), noErr);
            }

            let mut abl = AudioBufferList {
                mNumberBuffers: 1,
                mBuffers: [AudioBuffer {
                    mNumberChannels: 2,
                    mDataByteSize: 0,
                    mData: ptr::null_mut(),
                }],
            };
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, FRAMES, &mut abl), noErr);

            let out = abl.mBuffers[0].mData as *const f32;
            assert!(!out.is_null(), "the unit must supply storage for a null mData buffer");
            assert_eq!(abl.mBuffers[0].mDataByteSize, FRAMES * 2 * 4);

            let st = u.unit().state.lock();
            let engine = st.engine.as_ref().unwrap();
            assert_ne!(
                out,
                engine.out_l.as_ptr(),
                "interleaved output must not be written over the mix it reads from"
            );
            assert_ne!(out, engine.out_r.as_ptr());
            let mut any_signal = false;
            for i in 0..FRAMES as usize {
                assert_eq!(*out.add(2 * i), engine.out_l[i], "left sample {i}");
                assert_eq!(*out.add(2 * i + 1), engine.out_r[i], "right sample {i}");
                any_signal |= engine.out_l[i] != 0.0 || engine.out_r[i] != 0.0;
            }
            assert!(any_signal, "the test is toothless if the block is silent");
        }
    }

    /// Mono null buffers still get pointed straight at the mixes (no copy),
    /// one per channel, and report the byte count they were filled with.
    #[test]
    fn mono_null_buffers_are_backed_by_the_mixes() {
        const FRAMES: u32 = 32;
        let u = TestUnit::initialized();
        unsafe {
            // A two-buffer AudioBufferList is a flexible array member; build
            // the real variable-length layout by hand.
            let mut storage =
                vec![0u8; size_of::<AudioBufferList>() + size_of::<AudioBuffer>()];
            let list = storage.as_mut_ptr() as *mut AudioBufferList;
            (*list).mNumberBuffers = 2;
            for buf in (*list).buffers_mut() {
                buf.mNumberChannels = 1;
                buf.mDataByteSize = 0;
                buf.mData = ptr::null_mut();
            }

            let ts = zero_timestamp();
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, FRAMES, list), noErr);

            let out = (*list).buffers_mut();
            assert_eq!(out[0].mDataByteSize, FRAMES * 4);
            assert_eq!(out[1].mDataByteSize, FRAMES * 4);
            let st = u.unit().state.lock();
            let engine = st.engine.as_ref().unwrap();
            assert_eq!(out[0].mData as *const f32, engine.out_l.as_ptr());
            assert_eq!(out[1].mData as *const f32, engine.out_r.as_ptr());
        }
    }

    /// Hosts query a property's size by passing a null outData. For a
    /// property whose value is a freshly created +1 CoreFoundation object
    /// that must NOT build the object (the reference would be dropped on the
    /// floor), and an undersized buffer must fail rather than receive half
    /// a pointer.
    #[test]
    fn owned_properties_survive_size_queries_and_short_buffers() {
        let u = TestUnit::open();
        let owned: [(u32, u32, usize); 5] = [
            (kAudioUnitProperty_ClassInfo, 0, size_of::<CFPropertyListRef>()),
            (kAudioUnitProperty_ParameterInfo, 0, size_of::<AudioUnitParameterInfo>()),
            (kAudioUnitProperty_ParameterValueStrings, 0, size_of::<CFArrayRef>()),
            (kAudioUnitProperty_PresentPreset, 0, size_of::<AUPreset>()),
            (
                kAudioUnitProperty_ParameterStringFromValue,
                0,
                size_of::<AudioUnitParameterStringFromValue>(),
            ),
        ];
        unsafe {
            for (prop, elem, want) in owned {
                let mut size = 0u32;
                assert_eq!(
                    au_get_property(
                        u.0,
                        prop,
                        kAudioUnitScope_Global,
                        elem,
                        ptr::null_mut(),
                        &mut size
                    ),
                    noErr,
                    "size query for property {prop}"
                );
                assert_eq!(size as usize, want, "size for property {prop}");

                let mut buf = [0u8; 256];
                let mut small = (want - 1) as u32;
                assert_eq!(
                    au_get_property(
                        u.0,
                        prop,
                        kAudioUnitScope_Global,
                        elem,
                        buf.as_mut_ptr() as *mut c_void,
                        &mut small
                    ),
                    kAudio_ParamError,
                    "short buffer for property {prop}"
                );
                assert!(buf.iter().all(|b| *b == 0), "property {prop} half-wrote a value");
            }
        }
    }

    /// A null ioDataSize used to be dereferenced outright by the two in/out
    /// parameter-string properties — an immediate crash inside the host.
    #[test]
    fn null_io_size_is_rejected_not_dereferenced() {
        let u = TestUnit::open();
        unsafe {
            for prop in [
                kAudioUnitProperty_ParameterStringFromValue,
                kAudioUnitProperty_ParameterValueFromString,
                kAudioUnitProperty_ClassInfo,
                kAudioUnitProperty_ParameterInfo,
                #[cfg(feature = "editor")]
                kAudioUnitProperty_CocoaUI,
            ] {
                assert_eq!(
                    au_get_property(
                        u.0,
                        prop,
                        kAudioUnitScope_Global,
                        0,
                        ptr::null_mut(),
                        ptr::null_mut()
                    ),
                    kAudio_ParamError,
                    "property {prop}"
                );
            }
        }
    }

    /// Hosts do address parameter ids we never advertised (stale automation
    /// in an old project, a rescan race). None of it may index out of bounds.
    #[test]
    fn out_of_range_parameter_ids_are_refused() {
        let u = TestUnit::open();
        let count = u.unit().defs.len() as u32;
        unsafe {
            let mut v = 0.0f32;
            for id in [count, count + 1, u32::MAX] {
                assert_eq!(
                    au_get_parameter(u.0, id, kAudioUnitScope_Global, 0, &mut v),
                    kAudioUnitErr_InvalidParameter
                );
                assert_eq!(
                    au_set_parameter(u.0, id, kAudioUnitScope_Global, 0, 0.5, 0),
                    kAudioUnitErr_InvalidParameter
                );
                let event = AudioUnitParameterEvent {
                    scope: kAudioUnitScope_Global,
                    element: 0,
                    parameter: id,
                    eventType: kParameterEvent_Immediate,
                    eventValues: [0, 0.5f32.to_bits(), 0, 0],
                };
                assert_eq!(au_schedule_parameters(u.0, &event, 1), kAudioUnitErr_InvalidParameter);
                // The string conversions take the id from host-supplied
                // struct fields, not from the element.
                let mut query = AudioUnitParameterStringFromValue {
                    inParamID: id,
                    inValue: ptr::null(),
                    outString: ptr::null(),
                };
                let mut size = size_of::<AudioUnitParameterStringFromValue>() as u32;
                assert_eq!(
                    au_get_property(
                        u.0,
                        kAudioUnitProperty_ParameterStringFromValue,
                        kAudioUnitScope_Global,
                        0,
                        &mut query as *mut _ as *mut c_void,
                        &mut size
                    ),
                    kAudioUnitErr_InvalidParameter
                );
            }
            assert_eq!(
                au_set_parameter(u.0, 0, kAudioUnitScope_Output, 0, 0.5, 0),
                kAudioUnitErr_InvalidScope
            );
        }
    }

    /// NaN and out-of-range values a host can push at us must never land in
    /// the surface: one NaN through a recursive filter silences the instance
    /// for the rest of its life.
    #[test]
    fn hostile_parameter_values_are_clamped_at_the_door() {
        let u = TestUnit::open();
        let unit = u.unit();
        unsafe {
            for (id, def) in unit.defs.iter().enumerate() {
                let (min, max) = param_range(def);
                for hostile in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e30, 1e30] {
                    assert_eq!(
                        au_set_parameter(u.0, id as u32, kAudioUnitScope_Global, 0, hostile, 0),
                        noErr
                    );
                    let v = unit.value(id);
                    assert!(
                        v.is_finite() && v >= min && v <= max,
                        "`{}` accepted {hostile} -> {v}",
                        def.id()
                    );
                }
            }
        }
    }

    /// ClassInfo is what Logic writes into project files: every value has to
    /// come back, and a dictionary that is not ours must be refused.
    #[test]
    fn class_info_round_trips_and_rejects_foreign_state() {
        let u = TestUnit::open();
        let unit = u.unit();
        let expected: Vec<f32> = unit
            .defs
            .iter()
            .enumerate()
            .map(|(i, def)| {
                let (min, max) = param_range(def);
                unit.set_value(i, min + (max - min) * ((i % 7) as f32 / 6.0));
                unit.value(i)
            })
            .collect();

        unsafe {
            let saved = unit.save_state();
            assert!(!saved.is_null());
            for i in 0..unit.defs.len() {
                unit.set_value(i, param_range(&unit.defs[i]).0);
            }
            assert_eq!(unit.restore_state(saved), noErr);
            for (i, want) in expected.iter().enumerate() {
                assert_eq!(unit.value(i), *want, "`{}` did not round-trip", unit.defs[i].id());
            }
            CFRelease(saved);

            let alien = CFDictionaryCreateMutable(
                kCFAllocatorDefault,
                0,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            assert_eq!(unit.restore_state(alien), kAudioUnitErr_InvalidPropertyValue);
            CFRelease(alien);
            assert_eq!(unit.restore_state(ptr::null()), kAudioUnitErr_InvalidPropertyValue);
        }
    }

    /// A corrupted or hand-edited .aupreset restores without ever storing an
    /// unusable value.
    #[test]
    fn corrupt_preset_blob_cannot_poison_the_surface() {
        let u = TestUnit::open();
        let unit = u.unit();
        let mut blob = String::new();
        for def in unit.defs.iter() {
            blob.push_str(def.id());
            blob.push_str(" NaN\n");
        }
        blob.push_str("no_such_param 1.0\nmalformed-line\n\n");
        unsafe {
            let dict = CFDictionaryCreateMutable(
                kCFAllocatorDefault,
                0,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );
            let put = |key: &str, v: i32| {
                let k = cfstring(key);
                let n = cfnumber_i32(v);
                CFDictionarySetValue(dict, k, n);
                CFRelease(k);
                CFRelease(n);
            };
            put("type", AU_TYPE as i32);
            put("subtype", AU_SUBTYPE as i32);
            put("manufacturer", AU_MANUFACTURER as i32);
            let k = cfstring("data");
            let d = CFDataCreate(kCFAllocatorDefault, blob.as_ptr(), blob.len() as CFIndex);
            CFDictionarySetValue(dict, k, d);
            CFRelease(k);
            CFRelease(d);

            assert_eq!(unit.restore_state(dict), noErr);
            for (i, def) in unit.defs.iter().enumerate() {
                let (min, max) = param_range(def);
                let v = unit.value(i);
                assert!(v.is_finite() && v >= min && v <= max, "`{}` -> {v}", def.id());
            }
            CFRelease(dict);
        }
    }

    /// Render before Initialize, past the frame limit, and on a bus we do
    /// not have: all refused, none of them touching the host's samples.
    #[test]
    fn render_preconditions_are_enforced() {
        let u = TestUnit::open();
        unsafe {
            let ts = zero_timestamp();
            let mut samples = [1.0f32; 16];
            let mut abl = AudioBufferList {
                mNumberBuffers: 1,
                mBuffers: [AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: 64,
                    mData: samples.as_mut_ptr() as *mut c_void,
                }],
            };
            assert_eq!(
                au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl),
                kAudioUnitErr_Uninitialized
            );
            assert_eq!(au_initialize(u.0), noErr);
            assert_eq!(
                au_render(u.0, ptr::null_mut(), &ts, 1, 16, &mut abl),
                kAudioUnitErr_InvalidElement
            );
            assert_eq!(
                au_render(u.0, ptr::null_mut(), &ts, 0, DEFAULT_MAX_FRAMES + 1, &mut abl),
                kAudioUnitErr_TooManyFramesToProcess
            );
            assert!(samples.iter().all(|s| *s == 1.0), "a refused render wrote samples");
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, 16, ptr::null_mut()), kAudio_ParamError);

            let mut err: OSStatus = 0;
            let mut size = 4u32;
            assert_eq!(
                au_get_property(
                    u.0,
                    kAudioUnitProperty_LastRenderError,
                    kAudioUnitScope_Global,
                    0,
                    &mut err as *mut OSStatus as *mut c_void,
                    &mut size
                ),
                noErr
            );
            assert_eq!(err, kAudioUnitErr_TooManyFramesToProcess);
        }
    }

    /// Every advertised parameter reports a usable AudioUnitParameterInfo —
    /// a terminated name, a finite range, a default inside it. A generic AU
    /// view reads exactly this and nothing else.
    #[test]
    fn parameter_info_is_well_formed_for_every_id() {
        let u = TestUnit::open();
        let count = u.unit().defs.len();
        unsafe {
            for id in 0..count as u32 {
                let mut info: AudioUnitParameterInfo = std::mem::zeroed();
                let mut size = size_of::<AudioUnitParameterInfo>() as u32;
                assert_eq!(
                    au_get_property(
                        u.0,
                        kAudioUnitProperty_ParameterInfo,
                        kAudioUnitScope_Global,
                        id,
                        &mut info as *mut _ as *mut c_void,
                        &mut size
                    ),
                    noErr,
                    "parameter {id}"
                );
                assert!(info.name.iter().any(|c| *c == 0), "parameter {id} name not terminated");
                assert!(!info.cfNameString.is_null());
                CFRelease(info.cfNameString);
                assert!(info.minValue.is_finite() && info.maxValue.is_finite());
                assert!(info.minValue <= info.defaultValue && info.defaultValue <= info.maxValue);
                assert!(info.flags & kAudioUnitParameterFlag_IsReadable != 0);
            }
            let mut size = 0u32;
            assert_eq!(
                au_get_property_info(
                    u.0,
                    kAudioUnitProperty_ParameterInfo,
                    kAudioUnitScope_Global,
                    count as u32,
                    &mut size,
                    ptr::null_mut()
                ),
                kAudioUnitErr_InvalidParameter
            );
        }
    }

    unsafe extern "C" fn counting_notify(
        refcon: *mut c_void,
        flags: *mut AudioUnitRenderActionFlags,
        _ts: *const AudioTimeStamp,
        _bus: u32,
        _frames: u32,
        _data: *mut AudioBufferList,
    ) -> OSStatus {
        let counters = &mut *(refcon as *mut [u32; 2]);
        if *flags & kAudioUnitRenderAction_PreRender != 0 {
            counters[0] += 1;
        }
        if *flags & kAudioUnitRenderAction_PostRender != 0 {
            counters[1] += 1;
        }
        noErr
    }

    /// The render thread fans notifications out through a fixed stack window
    /// so it never allocates. More registrations than the window must still
    /// each fire exactly once, pre and post — a silently truncated fan-out
    /// would cost the host its metering and latency reporting.
    #[test]
    fn every_render_notify_fires_once_pre_and_post() {
        const N: usize = 20; // deliberately more than the stack window
        let u = TestUnit::initialized();
        let mut counters = vec![[0u32; 2]; N];
        unsafe {
            for i in 0..N {
                assert_eq!(
                    au_add_render_notify(
                        u.0,
                        counting_notify,
                        counters.as_mut_ptr().add(i) as *mut c_void
                    ),
                    noErr
                );
            }
            let ts = zero_timestamp();
            let mut samples = [0.0f32; 16];
            let mut abl = AudioBufferList {
                mNumberBuffers: 1,
                mBuffers: [AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: 64,
                    mData: samples.as_mut_ptr() as *mut c_void,
                }],
            };
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl), noErr);
            for (i, c) in counters.iter().enumerate() {
                assert_eq!(*c, [1, 1], "notify {i}");
            }

            // Removing one takes it — and only it — out of the fan-out.
            assert_eq!(
                au_remove_render_notify(u.0, counting_notify, counters.as_mut_ptr() as *mut c_void),
                noErr
            );
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl), noErr);
            assert_eq!(counters[0], [1, 1], "removed notify still firing");
            for (i, c) in counters.iter().enumerate().skip(1) {
                assert_eq!(*c, [2, 2], "notify {i}");
            }
        }
    }

    /// Initialize / Reset / Uninitialize in every order a host can send
    /// them, with the render path exercised in between.
    #[test]
    fn lifecycle_calls_are_idempotent() {
        let u = TestUnit::open();
        unsafe {
            let ts = zero_timestamp();
            let mut samples = [0.0f32; 16];
            let mut abl = AudioBufferList {
                mNumberBuffers: 1,
                mBuffers: [AudioBuffer {
                    mNumberChannels: 1,
                    mDataByteSize: 64,
                    mData: samples.as_mut_ptr() as *mut c_void,
                }],
            };
            // Reset before Initialize is legal and must not build an engine.
            assert_eq!(au_reset(u.0, kAudioUnitScope_Global, 0), noErr);
            assert!(u.unit().state.lock().engine.is_none());

            assert_eq!(au_initialize(u.0), noErr);
            assert_eq!(au_initialize(u.0), noErr, "double Initialize");
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl), noErr);
            assert_eq!(au_reset(u.0, kAudioUnitScope_Global, 0), noErr);
            assert_eq!(au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl), noErr);

            assert_eq!(au_uninitialize(u.0), noErr);
            assert_eq!(au_uninitialize(u.0), noErr, "double Uninitialize");
            assert_eq!(
                au_render(u.0, ptr::null_mut(), &ts, 0, 16, &mut abl),
                kAudioUnitErr_Uninitialized
            );
            // ...and the sample rate is settable again once uninitialized.
            let rate = 96_000.0f64;
            assert_eq!(
                au_set_property(
                    u.0,
                    kAudioUnitProperty_SampleRate,
                    kAudioUnitScope_Global,
                    0,
                    &rate as *const f64 as *const c_void,
                    8
                ),
                noErr
            );
            assert_eq!(au_initialize(u.0), noErr);
            assert_eq!(
                au_set_property(
                    u.0,
                    kAudioUnitProperty_SampleRate,
                    kAudioUnitScope_Global,
                    0,
                    &rate as *const f64 as *const c_void,
                    8
                ),
                kAudioUnitErr_Initialized
            );
        }
    }

    /// MIDI arrives on the render thread from hosts that do not validate it;
    /// every status/data combination has to be survivable.
    #[test]
    fn midi_events_never_panic() {
        let u = TestUnit::initialized();
        unsafe {
            for status in 0u32..=0xFFu32 {
                for data1 in [0u32, 1, 64, 120, 123, 127, 255, u32::MAX] {
                    for data2 in [0u32, 1, 63, 64, 127, 255, u32::MAX] {
                        assert_eq!(au_midi_event(u.0, status, data1, data2, 0), noErr);
                    }
                }
            }
            assert_eq!(au_sysex(u.0, ptr::null(), 0), noErr);

            let params =
                MusicDeviceNoteParams { argCount: 2, mPitch: f32::NAN, mVelocity: 1e30 };
            let mut note: NoteInstanceID = 0;
            assert_eq!(au_start_note(u.0, 0, 0, &mut note, 0, &params), noErr);
            assert_eq!(au_stop_note(u.0, 0, u32::MAX, 0), noErr);
            assert_eq!(
                au_start_note(u.0, 0, 0, ptr::null_mut(), 0, ptr::null()),
                kAudio_ParamError
            );
        }
    }

    /// The rate band the AU advertises must be the band the engine is
    /// actually stable at — the per-module
    /// `..._across_the_supported_rate_band` tests sweep exactly this
    /// range. If the engine's floor moves, this boundary moves with it;
    /// it is not a second opinion about what a host may ask for.
    #[test]
    fn the_accepted_rate_band_is_the_engines_band() {
        use crate::supported_sample_rate as ok;
        // Every rate a host actually offers
        for r in [
            8000.0, 11025.0, 16000.0, 22050.0, 32000.0, 44100.0, 48000.0, 88200.0,
            96000.0, 176400.0, 192000.0, 384000.0,
        ] {
            assert!(ok(r), "{r} Hz is a standard host rate and must be accepted");
        }
        // ...and nothing outside the band the circuits survive
        for r in [
            0.0, 1.0, 100.0, 1000.0, 4000.0, 7999.0, 1_000_000.0, f64::NAN,
            f64::INFINITY, -48000.0,
        ] {
            assert!(!ok(r), "{r} Hz must be refused, not rendered as garbage");
        }
        assert_eq!(crate::MIN_SAMPLE_RATE, 8000.0);
    }
}
