// The AU's custom view: kAudioUnitProperty_CocoaUI hands Logic a factory
// class name; Logic instantiates it and calls uiViewForAudioUnit:withSize:,
// and we answer with an NSView hosting the shared egui editor
// (src/editor.rs) via baseview. The two Objective-C classes involved are
// registered with the runtime on first use — no .xib, no Objective-C
// source, no extra frameworks.
//
// Class names carry the crate version so two loaded Patina versions can
// never collide in the ObjC runtime's flat class namespace.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_void, CString};
use std::mem::transmute;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};

use baseview::{Size, WindowHandle, WindowOpenOptions, WindowScalePolicy};
use egui_baseview::{EguiWindow, GraphicsConfig};
use raw_window_handle::{
    AppKitWindowHandle, HasRawWindowHandle, RawWindowHandle,
};

use super::ffi::{cfstring, CFStringRef, OSStatus};
use super::AuUnit;
use crate::editor::{EditorState, ParamHost, EDITOR_HEIGHT, EDITOR_WIDTH};

/// Private property the view uses to find its AuUnit in-process. Apple
/// reserves IDs below 64000 for the system; this lives in the third-party
/// range and is invisible to auval's conformance checks.
pub const PROP_PATINA_UNIT: u32 = 64001;

// --- Objective-C runtime ----------------------------------------------------

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;

#[repr(C)]
struct ObjcSuper {
    receiver: Id,
    super_class: Class,
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn objc_getProtocol(name: *const c_char) -> *mut c_void;
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, sel: Sel, imp: *const c_void, types: *const c_char) -> u8;
    fn class_addIvar(
        cls: Class,
        name: *const c_char,
        size: usize,
        alignment: u8,
        types: *const c_char,
    ) -> u8;
    fn class_addProtocol(cls: Class, protocol: *mut c_void) -> u8;
    fn object_getClass(obj: Id) -> Class;
    fn object_getInstanceVariable(obj: Id, name: *const c_char, out: *mut *mut c_void) -> Id;
    fn object_setInstanceVariable(obj: Id, name: *const c_char, value: *mut c_void) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_msgSendSuper();
}

fn sel(name: &str) -> Sel {
    let c = CString::new(name).unwrap();
    unsafe { sel_registerName(c.as_ptr()) }
}

fn cls(name: &str) -> Class {
    let c = CString::new(name).unwrap();
    unsafe { objc_getClass(c.as_ptr()) }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

// --- AudioToolbox: event-listener notifications ------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioUnitParameterRef {
    mAudioUnit: *mut c_void,
    mParameterID: u32,
    mScope: u32,
    mElement: u32,
}

/// kAudioUnitEvent_ParameterValueChange = 0, Begin = 1, End = 2.
#[repr(C)]
struct AudioUnitEvent {
    mEventType: u32,
    mArgument: AudioUnitParameterRef,
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioUnitGetProperty(
        unit: *mut c_void,
        prop: u32,
        scope: u32,
        elem: u32,
        out_data: *mut c_void,
        io_size: *mut u32,
    ) -> OSStatus;
    fn AUEventListenerNotify(
        listener: *mut c_void,
        object: *mut c_void,
        event: *const AudioUnitEvent,
    ) -> OSStatus;
}

// --- Bundle location (for AudioUnitCocoaViewInfo) -----------------------------

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

extern "C" {
    fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
    fn CFURLCreateWithFileSystemPath(
        alloc: *const c_void,
        path: CFStringRef,
        style: isize, // kCFURLPOSIXPathStyle = 0
        is_directory: u8,
    ) -> *const c_void;
}

/// Path of the bundle containing this code:
/// .../Patina.component/Contents/MacOS/Patina -> .../Patina.component
fn bundle_path() -> Option<String> {
    unsafe {
        let mut info: DlInfo = std::mem::zeroed();
        if dladdr(bundle_path as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let dylib = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
        let p = std::path::Path::new(&dylib);
        Some(p.parent()?.parent()?.parent()?.to_string_lossy().into_owned())
    }
}

// --- The editor's view of the AU ----------------------------------------------

/// ParamHost over the AU's own parameter atomics. The view always lives in
/// the same process as the unit (Logic loads the component bundle either
/// in-process or inside the same AUHostingService that renders it), so a
/// plain pointer is sound for the view's lifetime: hosts tear the view
/// down before disposing the unit.
struct AuParamHost {
    unit: *const AuUnit,
    au: *mut c_void,
}

unsafe impl Send for AuParamHost {}
unsafe impl Sync for AuParamHost {}

impl AuParamHost {
    fn notify(&self, event_type: u32, index: usize) {
        let event = AudioUnitEvent {
            mEventType: event_type,
            mArgument: AudioUnitParameterRef {
                mAudioUnit: self.au,
                mParameterID: index as u32,
                mScope: 0, // global
                mElement: 0,
            },
        };
        unsafe { AUEventListenerNotify(null_mut(), null_mut(), &event) };
    }
}

impl ParamHost for AuParamHost {
    fn get(&self, index: usize) -> f32 {
        unsafe { (*self.unit).value(index) }
    }

    fn set(&self, index: usize, value: f32) {
        unsafe { (*self.unit).set_value(index, value) };
        self.notify(0, index); // kAudioUnitEvent_ParameterValueChange
    }

    fn begin_gesture(&self, index: usize) {
        self.notify(1, index); // kAudioUnitEvent_BeginParameterChangeGesture
    }

    fn end_gesture(&self, index: usize) {
        self.notify(2, index); // kAudioUnitEvent_EndParameterChangeGesture
    }
}

// --- The two runtime-registered classes ---------------------------------------

struct ViewGlue {
    window: WindowHandle,
}

const GLUE_IVAR: &str = "patinaGlue";

static REGISTER: Once = Once::new();
static FACTORY_CLASS: AtomicUsize = AtomicUsize::new(0);
static CONTAINER_CLASS: AtomicUsize = AtomicUsize::new(0);

fn versioned(name: &str) -> String {
    format!("{}_{}", name, env!("CARGO_PKG_VERSION").replace('.', "_"))
}

pub fn factory_class_name() -> String {
    versioned("PatinaAUViewFactory")
}

/// Register both classes with the ObjC runtime (idempotent).
pub fn register_classes() {
    REGISTER.call_once(|| unsafe {
        // Container: NSView subclass that closes its baseview child on the
        // way out.
        let container_name = CString::new(versioned("PatinaAUContainerView")).unwrap();
        let container = objc_allocateClassPair(cls("NSView"), container_name.as_ptr(), 0);
        if container.is_null() {
            return;
        }
        let ivar = CString::new(GLUE_IVAR).unwrap();
        class_addIvar(
            container,
            ivar.as_ptr(),
            size_of::<*mut c_void>(),
            align_of::<*mut c_void>().trailing_zeros() as u8,
            CString::new("^v").unwrap().as_ptr(),
        );
        class_addMethod(
            container,
            sel("dealloc"),
            container_dealloc as *const c_void,
            CString::new("v@:").unwrap().as_ptr(),
        );
        objc_registerClassPair(container);
        CONTAINER_CLASS.store(container as usize, Ordering::Release);

        // Factory: the AUCocoaUIBase implementor named in CocoaUI info.
        let factory_name = CString::new(factory_class_name()).unwrap();
        let factory = objc_allocateClassPair(cls("NSObject"), factory_name.as_ptr(), 0);
        if factory.is_null() {
            return;
        }
        class_addMethod(
            object_getClass(factory), // class method -> metaclass
            sel("interfaceVersion"),
            interface_version as *const c_void,
            CString::new("I@:").unwrap().as_ptr(),
        );
        class_addMethod(
            factory,
            sel("uiViewForAudioUnit:withSize:"),
            ui_view_for_audio_unit as *const c_void,
            CString::new("@@:^v{CGSize=dd}").unwrap().as_ptr(),
        );
        let proto = objc_getProtocol(CString::new("AUCocoaUIBase").unwrap().as_ptr());
        if !proto.is_null() {
            class_addProtocol(factory, proto);
        }
        objc_registerClassPair(factory);
        FACTORY_CLASS.store(factory as usize, Ordering::Release);
    });
}

unsafe extern "C" fn interface_version(_this: Id, _cmd: Sel) -> u32 {
    0
}

unsafe extern "C" fn container_dealloc(this: Id, _cmd: Sel) {
    let ivar = CString::new(GLUE_IVAR).unwrap();
    let mut glue_ptr: *mut c_void = null_mut();
    object_getInstanceVariable(this, ivar.as_ptr(), &mut glue_ptr);
    if !glue_ptr.is_null() {
        object_setInstanceVariable(this, ivar.as_ptr(), null_mut());
        let mut glue = Box::from_raw(glue_ptr as *mut ViewGlue);
        glue.window.close();
    }
    let sup = ObjcSuper {
        receiver: this,
        super_class: cls("NSView"),
    };
    let send: unsafe extern "C" fn(*const ObjcSuper, Sel) =
        transmute(objc_msgSendSuper as *const c_void);
    send(&sup, sel("dealloc"));
}

/// Wrapper handing baseview the container NSView as parent.
struct ParentView(*mut c_void);

unsafe impl HasRawWindowHandle for ParentView {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let mut handle = AppKitWindowHandle::empty();
        handle.ns_view = self.0;
        RawWindowHandle::AppKit(handle)
    }
}

unsafe extern "C" fn ui_view_for_audio_unit(
    _this: Id,
    _cmd: Sel,
    audio_unit: *mut c_void,
    _size: CGSize,
) -> Id {
    // Find our AuUnit through the private property. The [pointer, pid]
    // pair only dereferences when the unit lives in THIS process — if a
    // host ever creates the view in a different process than the unit,
    // returning nil here makes it fall back to its generic view instead
    // of us touching a foreign address.
    let mut handshake: [u64; 2] = [0, 0];
    let mut io_size = size_of::<[u64; 2]>() as u32;
    let status = AudioUnitGetProperty(
        audio_unit,
        PROP_PATINA_UNIT,
        0, // global scope
        0,
        handshake.as_mut_ptr() as *mut c_void,
        &mut io_size,
    );
    let unit_addr = handshake[0] as usize;
    if status != 0 || unit_addr == 0 || handshake[1] != std::process::id() as u64 {
        return null_mut();
    }

    let container_cls = CONTAINER_CLASS.load(Ordering::Acquire) as Class;
    if container_cls.is_null() {
        return null_mut();
    }
    let send_id: unsafe extern "C" fn(Id, Sel) -> Id = transmute(objc_msgSend as *const c_void);
    let send_rect: unsafe extern "C" fn(Id, Sel, CGRect) -> Id =
        transmute(objc_msgSend as *const c_void);
    let alloc = send_id(container_cls, sel("alloc"));
    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: EDITOR_WIDTH as f64, height: EDITOR_HEIGHT as f64 },
    };
    let container = send_rect(alloc, sel("initWithFrame:"), frame);
    if container.is_null() {
        return null_mut();
    }

    let host = Arc::new(AuParamHost { unit: unit_addr as *const AuUnit, au: audio_unit });
    let state = EditorState::new(host);
    let window = EguiWindow::open_parented(
        &ParentView(container),
        WindowOpenOptions {
            title: "Patina".to_string(),
            size: Size::new(EDITOR_WIDTH as f64, EDITOR_HEIGHT as f64),
            scale: WindowScalePolicy::SystemScaleFactor,
            gl_config: Some(Default::default()),
        },
        GraphicsConfig::default(),
        state,
        |_ctx: &egui::Context, _queue: &mut egui_baseview::Queue, _state: &mut EditorState| {},
        |ctx: &egui::Context, _queue: &mut egui_baseview::Queue, state: &mut EditorState| {
            state.update(ctx);
        },
    );

    let glue = Box::new(ViewGlue { window });
    let ivar = CString::new(GLUE_IVAR).unwrap();
    object_setInstanceVariable(container, ivar.as_ptr(), Box::into_raw(glue) as *mut c_void);

    // Returned +1 per AUCocoaUIBase convention; the host releases it.
    container
}

/// Build the (bundle URL, view class CFString) pair for
/// kAudioUnitProperty_CocoaUI. Returns null URL if the bundle path can't
/// be resolved (the host then falls back to its generic view).
pub fn cocoa_view_info() -> Option<(*const c_void, CFStringRef)> {
    register_classes();
    if FACTORY_CLASS.load(Ordering::Acquire) == 0 {
        return None;
    }
    let path = bundle_path()?;
    unsafe {
        let cf_path = cfstring(&path);
        let url = CFURLCreateWithFileSystemPath(null(), cf_path, 0, 1);
        super::ffi::CFRelease(cf_path);
        if url.is_null() {
            return None;
        }
        Some((url, cfstring(&factory_class_name())))
    }
}
