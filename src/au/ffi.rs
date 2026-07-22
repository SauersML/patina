// Minimal hand-rolled FFI for implementing an AUv2 Audio Unit: the
// AudioComponent plug-in ABI, the AudioUnit property/parameter vocabulary,
// and the handful of CoreFoundation calls ClassInfo serialization needs.
//
// Every constant and struct layout here was transcribed from the macOS SDK
// headers (AudioToolbox/AUComponent.h, AudioUnitProperties.h, MusicDevice.h,
// AudioComponent.h, CoreAudioTypes.h) — do not "correct" values from memory.

#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use std::ffi::{c_char, c_void};

pub type OSStatus = i32;
pub type CFIndex = isize;
pub type CFTypeID = usize;
pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFMutableDictionaryRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFDataRef = *const c_void;
pub type CFNumberRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFPropertyListRef = *const c_void;
pub type Boolean = u8;

pub type AudioUnitPropertyID = u32;
pub type AudioUnitScope = u32;
pub type AudioUnitElement = u32;
pub type AudioUnitParameterID = u32;
pub type AudioUnitParameterValue = f32;
pub type AudioUnitRenderActionFlags = u32;
pub type AudioComponentInstance = *mut c_void;
pub type MusicDeviceInstrumentID = u32;
pub type MusicDeviceGroupID = u32;
pub type NoteInstanceID = u32;

pub const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

// --- Scopes -----------------------------------------------------------------
pub const kAudioUnitScope_Global: u32 = 0;
pub const kAudioUnitScope_Input: u32 = 1;
pub const kAudioUnitScope_Output: u32 = 2;

// --- Selectors (AUComponent.h, MusicDevice.h) --------------------------------
pub const kAudioUnitInitializeSelect: i16 = 0x0001;
pub const kAudioUnitUninitializeSelect: i16 = 0x0002;
pub const kAudioUnitGetPropertyInfoSelect: i16 = 0x0003;
pub const kAudioUnitGetPropertySelect: i16 = 0x0004;
pub const kAudioUnitSetPropertySelect: i16 = 0x0005;
pub const kAudioUnitGetParameterSelect: i16 = 0x0006;
pub const kAudioUnitSetParameterSelect: i16 = 0x0007;
pub const kAudioUnitResetSelect: i16 = 0x0009;
pub const kAudioUnitAddPropertyListenerSelect: i16 = 0x000A;
pub const kAudioUnitRemovePropertyListenerSelect: i16 = 0x000B;
pub const kAudioUnitRenderSelect: i16 = 0x000E;
pub const kAudioUnitAddRenderNotifySelect: i16 = 0x000F;
pub const kAudioUnitRemoveRenderNotifySelect: i16 = 0x0010;
pub const kAudioUnitScheduleParametersSelect: i16 = 0x0011;
pub const kAudioUnitRemovePropertyListenerWithUserDataSelect: i16 = 0x0012;
pub const kMusicDeviceMIDIEventSelect: i16 = 0x0101;
pub const kMusicDeviceSysExSelect: i16 = 0x0102;
pub const kMusicDevicePrepareInstrumentSelect: i16 = 0x0103;
pub const kMusicDeviceReleaseInstrumentSelect: i16 = 0x0104;
pub const kMusicDeviceStartNoteSelect: i16 = 0x0105;
pub const kMusicDeviceStopNoteSelect: i16 = 0x0106;

// --- Properties (AudioUnitProperties.h) --------------------------------------
pub const kAudioUnitProperty_ClassInfo: u32 = 0;
pub const kAudioUnitProperty_MakeConnection: u32 = 1;
pub const kAudioUnitProperty_SampleRate: u32 = 2;
pub const kAudioUnitProperty_ParameterList: u32 = 3;
pub const kAudioUnitProperty_ParameterInfo: u32 = 4;
pub const kAudioUnitProperty_StreamFormat: u32 = 8;
pub const kAudioUnitProperty_ElementCount: u32 = 11;
pub const kAudioUnitProperty_Latency: u32 = 12;
pub const kAudioUnitProperty_SupportedNumChannels: u32 = 13;
pub const kAudioUnitProperty_MaximumFramesPerSlice: u32 = 14;
pub const kAudioUnitProperty_ParameterValueStrings: u32 = 16;
pub const kAudioUnitProperty_TailTime: u32 = 20;
pub const kAudioUnitProperty_BypassEffect: u32 = 21;
pub const kAudioUnitProperty_LastRenderError: u32 = 22;
pub const kAudioUnitProperty_SetRenderCallback: u32 = 23;
pub const kAudioUnitProperty_FactoryPresets: u32 = 24;
pub const kAudioUnitProperty_HostCallbacks: u32 = 27;
pub const kAudioUnitProperty_CurrentPreset: u32 = 28; // legacy alias of PresentPreset
pub const kAudioUnitProperty_InPlaceProcessing: u32 = 29;
pub const kAudioUnitProperty_ElementName: u32 = 30;
pub const kAudioUnitProperty_CocoaUI: u32 = 31;
pub const kAudioUnitProperty_SupportedChannelLayoutTags: u32 = 32;
pub const kAudioUnitProperty_ParameterStringFromValue: u32 = 33;
pub const kAudioUnitProperty_PresentPreset: u32 = 36;
pub const kAudioUnitProperty_ParameterValueFromString: u32 = 38;
pub const kAudioUnitProperty_OfflineRender: u32 = 37;
pub const kAudioUnitProperty_ShouldAllocateBuffer: u32 = 51;
pub const kMusicDeviceProperty_InstrumentCount: u32 = 1000;

// --- Errors (AUComponent.h) ---------------------------------------------------
pub const kAudioUnitErr_InvalidProperty: OSStatus = -10879;
pub const kAudioUnitErr_InvalidParameter: OSStatus = -10878;
pub const kAudioUnitErr_InvalidElement: OSStatus = -10877;
pub const kAudioUnitErr_FailedInitialization: OSStatus = -10875;
pub const kAudioUnitErr_TooManyFramesToProcess: OSStatus = -10874;
pub const kAudioUnitErr_FormatNotSupported: OSStatus = -10868;
pub const kAudioUnitErr_Uninitialized: OSStatus = -10867;
pub const kAudioUnitErr_InvalidScope: OSStatus = -10866;
pub const kAudioUnitErr_PropertyNotWritable: OSStatus = -10865;
pub const kAudioUnitErr_CannotDoInCurrentContext: OSStatus = -10863;
pub const kAudioUnitErr_InvalidPropertyValue: OSStatus = -10851;
pub const kAudioUnitErr_PropertyNotInUse: OSStatus = -10850;
pub const kAudioUnitErr_Initialized: OSStatus = -10849;
pub const kAudio_ParamError: OSStatus = -50;
pub const noErr: OSStatus = 0;
pub const badComponentSelector: OSStatus = -2003i32;

// --- Parameter info (AudioUnitProperties.h) -----------------------------------
pub const kAudioUnitParameterUnit_Generic: u32 = 0;
pub const kAudioUnitParameterUnit_Indexed: u32 = 1;
pub const kAudioUnitParameterUnit_Percent: u32 = 3;
pub const kAudioUnitParameterUnit_Seconds: u32 = 4;
pub const kAudioUnitParameterUnit_Hertz: u32 = 8;
pub const kAudioUnitParameterUnit_Cents: u32 = 9;
pub const kAudioUnitParameterUnit_Octaves: u32 = 21;
pub const kAudioUnitParameterUnit_CustomUnit: u32 = 26;

pub const kAudioUnitParameterFlag_CFNameRelease: u32 = 1 << 4;
pub const kAudioUnitParameterFlag_ValuesHaveStrings: u32 = 1 << 21;
pub const kAudioUnitParameterFlag_DisplayLogarithmic: u32 = 1 << 22;
pub const kAudioUnitParameterFlag_IsHighResolution: u32 = 1 << 23;
pub const kAudioUnitParameterFlag_HasCFNameString: u32 = 1 << 27;
pub const kAudioUnitParameterFlag_IsReadable: u32 = 1 << 30;
pub const kAudioUnitParameterFlag_IsWritable: u32 = 1 << 31;

pub const kParameterEvent_Immediate: u32 = 1;
pub const kParameterEvent_Ramped: u32 = 2;

// --- Render action flags (AUComponent.h) --------------------------------------
pub const kAudioUnitRenderAction_PreRender: u32 = 1 << 2;
pub const kAudioUnitRenderAction_PostRender: u32 = 1 << 3;
pub const kAudioUnitRenderAction_OutputIsSilence: u32 = 1 << 4;
pub const kAudioUnitRenderAction_PostRenderError: u32 = 1 << 8;

// --- Audio formats (CoreAudioTypes.h) ------------------------------------------
pub const kAudioFormatLinearPCM: u32 = fourcc(b"lpcm");
pub const kAudioFormatFlagIsFloat: u32 = 1 << 0;
pub const kAudioFormatFlagIsPacked: u32 = 1 << 3;
pub const kAudioFormatFlagIsNonInterleaved: u32 = 1 << 5;

// --- Structs -------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioComponentDescription {
    pub componentType: u32,
    pub componentSubType: u32,
    pub componentManufacturer: u32,
    pub componentFlags: u32,
    pub componentFlagsMask: u32,
}

/// Generic function pointer returned by Lookup; the host casts it to the
/// selector's concrete signature before calling.
pub type AudioComponentMethod = Option<unsafe extern "C" fn()>;

#[repr(C)]
pub struct AudioComponentPlugInInterface {
    pub Open: unsafe extern "C" fn(this: *mut c_void, instance: AudioComponentInstance) -> OSStatus,
    pub Close: unsafe extern "C" fn(this: *mut c_void) -> OSStatus,
    pub Lookup: unsafe extern "C" fn(selector: i16) -> AudioComponentMethod,
    pub reserved: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AudioStreamBasicDescription {
    pub mSampleRate: f64,
    pub mFormatID: u32,
    pub mFormatFlags: u32,
    pub mBytesPerPacket: u32,
    pub mFramesPerPacket: u32,
    pub mBytesPerFrame: u32,
    pub mChannelsPerFrame: u32,
    pub mBitsPerChannel: u32,
    pub mReserved: u32,
}

impl AudioStreamBasicDescription {
    /// The unit's native format: 32-bit float, non-interleaved.
    pub fn non_interleaved_f32(sample_rate: f64, channels: u32) -> Self {
        Self {
            mSampleRate: sample_rate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat
                | kAudioFormatFlagIsPacked
                | kAudioFormatFlagIsNonInterleaved,
            mBytesPerPacket: 4,
            mFramesPerPacket: 1,
            mBytesPerFrame: 4,
            mChannelsPerFrame: channels,
            mBitsPerChannel: 32,
            mReserved: 0,
        }
    }
}

#[repr(C)]
pub struct AudioBuffer {
    pub mNumberChannels: u32,
    pub mDataByteSize: u32,
    pub mData: *mut c_void,
}

#[repr(C)]
pub struct AudioBufferList {
    pub mNumberBuffers: u32,
    /// Variable length; index past 0 via `buffers_mut`.
    pub mBuffers: [AudioBuffer; 1],
}

impl AudioBufferList {
    pub unsafe fn buffers_mut(&mut self) -> &mut [AudioBuffer] {
        std::slice::from_raw_parts_mut(self.mBuffers.as_mut_ptr(), self.mNumberBuffers as usize)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SMPTETime {
    pub mSubframes: i16,
    pub mSubframeDivisor: i16,
    pub mCounter: u32,
    pub mType: u32,
    pub mFlags: u32,
    pub mHours: i16,
    pub mMinutes: i16,
    pub mSeconds: i16,
    pub mFrames: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioTimeStamp {
    pub mSampleTime: f64,
    pub mHostTime: u64,
    pub mRateScalar: f64,
    pub mWordClockTime: u64,
    pub mSMPTETime: SMPTETime,
    pub mFlags: u32,
    pub mReserved: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioUnitParameterInfo {
    pub name: [c_char; 52],
    pub unitName: CFStringRef,
    pub clumpID: u32,
    pub cfNameString: CFStringRef,
    pub unit: u32,
    pub minValue: f32,
    pub maxValue: f32,
    pub defaultValue: f32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AUChannelInfo {
    pub inChannels: i16,
    pub outChannels: i16,
}

/// In/out struct for kAudioUnitProperty_ParameterStringFromValue: the host
/// fills inParamID/inValue, the unit fills outString (+1, host releases).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioUnitParameterStringFromValue {
    pub inParamID: AudioUnitParameterID,
    pub inValue: *const AudioUnitParameterValue,
    pub outString: CFStringRef,
}

/// In/out struct for kAudioUnitProperty_ParameterValueFromString.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioUnitParameterValueFromString {
    pub inParamID: AudioUnitParameterID,
    pub inString: CFStringRef,
    pub outValue: AudioUnitParameterValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AUPreset {
    pub presetNumber: i32,
    pub presetName: CFStringRef,
}

/// Answer to kAudioUnitProperty_CocoaUI: which bundle to load and which
/// AUCocoaUIBase class inside it makes the view.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioUnitCocoaViewInfo {
    pub mCocoaAUViewBundleLocation: *const c_void,
    pub mCocoaAUViewClass: [CFStringRef; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioUnitParameterEvent {
    pub scope: AudioUnitScope,
    pub element: AudioUnitElement,
    pub parameter: AudioUnitParameterID,
    pub eventType: u32,
    /// Union of `ramp` (4 words) and `immediate` (2 words); interpreted per
    /// `eventType`.
    pub eventValues: [u32; 4],
}

impl AudioUnitParameterEvent {
    /// Final value the event lands on, whether immediate or ramped.
    pub fn target_value(&self) -> f32 {
        match self.eventType {
            kParameterEvent_Ramped => f32::from_bits(self.eventValues[3]),
            _ => f32::from_bits(self.eventValues[1]),
        }
    }
}

#[repr(C)]
pub struct MusicDeviceNoteParams {
    pub argCount: u32,
    pub mPitch: f32,
    pub mVelocity: f32,
    // NoteParamsControlValue mControls[]; not used
}

pub type AURenderCallback = unsafe extern "C" fn(
    inRefCon: *mut c_void,
    ioActionFlags: *mut AudioUnitRenderActionFlags,
    inTimeStamp: *const AudioTimeStamp,
    inBusNumber: u32,
    inNumberFrames: u32,
    ioData: *mut AudioBufferList,
) -> OSStatus;

pub type AudioUnitPropertyListenerProc = unsafe extern "C" fn(
    inRefCon: *mut c_void,
    inUnit: AudioComponentInstance,
    inID: AudioUnitPropertyID,
    inScope: AudioUnitScope,
    inElement: AudioUnitElement,
);

// --- CoreFoundation ------------------------------------------------------------

pub const kCFStringEncodingUTF8: u32 = 0x0800_0100;
pub const kCFNumberSInt32Type: CFIndex = 3;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFAllocatorDefault: *const c_void;
    pub static kCFTypeDictionaryKeyCallBacks: c_void;
    pub static kCFTypeDictionaryValueCallBacks: c_void;
    pub static kCFTypeArrayCallBacks: c_void;

    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;

    pub fn CFStringGetTypeID() -> CFTypeID;
    pub fn CFStringCreateWithCString(
        alloc: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    pub fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> Boolean;

    pub fn CFDictionaryGetTypeID() -> CFTypeID;
    pub fn CFDictionaryCreateMutable(
        alloc: *const c_void,
        capacity: CFIndex,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFMutableDictionaryRef;
    pub fn CFDictionarySetValue(dict: CFMutableDictionaryRef, key: CFTypeRef, value: CFTypeRef);
    pub fn CFDictionaryGetValue(dict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;

    pub fn CFNumberGetTypeID() -> CFTypeID;
    pub fn CFNumberCreate(
        alloc: *const c_void,
        the_type: CFIndex,
        value_ptr: *const c_void,
    ) -> CFNumberRef;
    pub fn CFNumberGetValue(
        number: CFNumberRef,
        the_type: CFIndex,
        value_ptr: *mut c_void,
    ) -> Boolean;

    pub fn CFDataGetTypeID() -> CFTypeID;
    pub fn CFDataCreate(alloc: *const c_void, bytes: *const u8, length: CFIndex) -> CFDataRef;
    pub fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    pub fn CFDataGetLength(data: CFDataRef) -> CFIndex;

    pub fn CFArrayCreate(
        alloc: *const c_void,
        values: *const CFTypeRef,
        num_values: CFIndex,
        callbacks: *const c_void,
    ) -> CFArrayRef;
}

/// Owned CFString built from a Rust str.
pub fn cfstring(s: &str) -> CFStringRef {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

/// Rust String from a CFString (empty on failure).
pub fn cfstring_to_string(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    let mut buf = [0i8; 512];
    unsafe {
        if CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as CFIndex, kCFStringEncodingUTF8) != 0
        {
            std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
        } else {
            String::new()
        }
    }
}

pub fn cfnumber_i32(v: i32) -> CFNumberRef {
    unsafe {
        CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &v as *const i32 as *const c_void)
    }
}

pub fn cfnumber_to_i32(n: CFNumberRef) -> Option<i32> {
    if n.is_null() {
        return None;
    }
    let mut out: i32 = 0;
    unsafe {
        (CFGetTypeID(n) == CFNumberGetTypeID()
            && CFNumberGetValue(n, kCFNumberSInt32Type, &mut out as *mut i32 as *mut c_void) != 0)
            .then_some(out)
    }
}
