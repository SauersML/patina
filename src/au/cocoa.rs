// The AU's custom view. kAudioUnitProperty_CocoaUI hands the host a factory
// class name; the host instantiates it and calls uiViewForAudioUnit:, and we
// answer with an NSView that software-renders the shared egui editor
// (src/editor.rs, src/au/raster.rs).
//
// Getting this to work out of process (how Logic always hosts AUs) took two
// things that are easy to get wrong:
//   1. AUCocoaUIBase declares interfaceVersion AND uiViewForAudioUnit: as
//      INSTANCE methods. Register interfaceVersion on the metaclass and the
//      bridge's conformance check fails, so it never calls the factory.
//   2. Rendering must ride AppKit's normal display cycle — drawRect:, marked
//      dirty via setNeedsDisplay: — NOT a private NSTimer pushing
//      layer.contents. The out-of-process view service pumps the former and
//      not the latter, so the timer/layer path draws blank there. A GPU
//      context is worse still: it can't composite across the ViewBridge at
//      all. So we draw a CPU bitmap into the view's CGContext each pass and
//      drive parameters through the AudioUnit proxy API (which bridges
//      across the process boundary), never touching the engine directly.
//
// The factory itself is a compiled ObjC class (src/au/factory.m) so the
// host can resolve it by name through the bundle; the NSView subclass is
// built at runtime, lazily, the first time a view is actually requested.
// Nothing here may run from a library initializer: this code has to load
// AppKit, and dlopen under the dyld lock deadlocks the hosting process.

#![allow(non_snake_case)]

use std::ffi::{c_char, c_void, CString};
use std::mem::transmute;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use egui::{pos2, vec2, Event, Modifiers, PointerButton, Pos2, Rect};

use super::ffi::{cfstring, CFStringRef, OSStatus};
use super::raster::Raster;
use crate::editor::{EditorState, ParamHost, EDITOR_HEIGHT, EDITOR_WIDTH};

/// Private property carrying the [pointer, pid] handshake to the in-process
/// view. Apple reserves IDs below 64000; this sits in the third-party range.
pub const PROP_PATINA_UNIT: u32 = 64001;


/// Editor refresh rate. 30 Hz keeps arcs and hover smooth without burning
/// CPU on a mostly static panel.
const FRAME_INTERVAL: f64 = 1.0 / 30.0;

// ---------------------------------------------------------------------------
// Objective-C + Core Graphics runtime
// ---------------------------------------------------------------------------

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

type CGColorSpaceRef = *mut c_void;
type CGContextRef = *mut c_void;
type CGImageRef = *mut c_void;

const kCGImageAlphaPremultipliedLast: u32 = 1;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> CGColorSpaceRef;
    fn CGColorSpaceRelease(cs: CGColorSpaceRef);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
    fn CGBitmapContextCreateImage(ctx: CGContextRef) -> CGImageRef;
    fn CGContextRelease(ctx: CGContextRef);
    fn CGImageRelease(img: CGImageRef);
    fn CGContextDrawImage(ctx: CGContextRef, rect: CGRect, image: CGImageRef);
    fn CGContextSaveGState(ctx: CGContextRef);
    fn CGContextRestoreGState(ctx: CGContextRef);
    fn CGContextTranslateCTM(ctx: CGContextRef, tx: f64, ty: f64);
    fn CGContextScaleCTM(ctx: CGContextRef, sx: f64, sy: f64);
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioUnitGetParameter(
        unit: *mut c_void,
        param: u32,
        scope: u32,
        elem: u32,
        out_value: *mut f32,
    ) -> OSStatus;
    fn AudioUnitSetParameter(
        unit: *mut c_void,
        param: u32,
        scope: u32,
        elem: u32,
        value: f32,
        buffer_offset: u32,
    ) -> OSStatus;
    fn AUEventListenerNotify(
        listener: *mut c_void,
        object: *mut c_void,
        event: *const AudioUnitEvent,
    ) -> OSStatus;
}


// msgSend, cast per call site to the concrete signature.
unsafe fn send0(obj: Id, s: Sel) -> Id {
    let f: unsafe extern "C" fn(Id, Sel) -> Id = transmute(objc_msgSend as *const c_void);
    f(obj, s)
}
unsafe fn send0_f64(obj: Id, s: Sel) -> f64 {
    let f: unsafe extern "C" fn(Id, Sel) -> f64 = transmute(objc_msgSend as *const c_void);
    f(obj, s)
}
unsafe fn send0_point(obj: Id, s: Sel) -> CGPoint {
    let f: unsafe extern "C" fn(Id, Sel) -> CGPoint = transmute(objc_msgSend as *const c_void);
    f(obj, s)
}
unsafe fn send_void_id(obj: Id, s: Sel, a: Id) {
    let f: unsafe extern "C" fn(Id, Sel, Id) = transmute(objc_msgSend as *const c_void);
    f(obj, s, a)
}
unsafe fn send_void_bool(obj: Id, s: Sel, a: u8) {
    let f: unsafe extern "C" fn(Id, Sel, u8) = transmute(objc_msgSend as *const c_void);
    f(obj, s, a)
}

// --- AU event notifications (parameter changes + gestures) -------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioUnitParameterRef {
    mAudioUnit: *mut c_void,
    mParameterID: u32,
    mScope: u32,
    mElement: u32,
}

#[repr(C)]
struct AudioUnitEvent {
    mEventType: u32,
    mArgument: AudioUnitParameterRef,
}

// --- Bundle location for AudioUnitCocoaViewInfo ------------------------------

#[repr(C)]
struct DlInfo {
    dli_fname: *const c_char,
    dli_fbase: *mut c_void,
    dli_sname: *const c_char,
    dli_saddr: *mut c_void,
}

const RTLD_LAZY: i32 = 1;

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
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

// ---------------------------------------------------------------------------
// ParamHost over the AU's parameter atomics
// ---------------------------------------------------------------------------

/// Parameters are driven through the standard AudioUnit C API on the
/// `audio_unit` the host handed the view. That handle is a real, possibly
/// cross-process proxy: out of process, the view lives in the host's UI
/// process while the engine runs in AUHostingService, and Get/SetParameter
/// bridge between them. So the view never touches the engine directly.
struct AuParamHost {
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
                mScope: 0,
                mElement: 0,
            },
        };
        unsafe { AUEventListenerNotify(null_mut(), self.au, &event) };
    }
}

impl ParamHost for AuParamHost {
    fn get(&self, index: usize) -> f32 {
        let mut value = 0.0f32;
        unsafe { AudioUnitGetParameter(self.au, index as u32, 0, 0, &mut value) };
        value
    }
    fn set(&self, index: usize, value: f32) {
        unsafe { AudioUnitSetParameter(self.au, index as u32, 0, 0, value, 0) };
        // Let the host observe the change for automation recording.
        self.notify(0, index); // kAudioUnitEvent_ParameterValueChange
    }
    fn begin_gesture(&self, index: usize) {
        self.notify(1, index); // Begin
    }
    fn end_gesture(&self, index: usize) {
        self.notify(2, index); // End
    }
}

// ---------------------------------------------------------------------------
// Per-view state (owned through the NSView's ivar)
// ---------------------------------------------------------------------------

struct ViewState {
    view: Id, // unretained; the ivar owner
    ctx: egui::Context,
    editor: EditorState,
    raster: Raster,
    colorspace: CGColorSpaceRef,
    start: Instant,
    mouse: Pos2,
    pending: Vec<Event>,
    timer: Id,
}

const STATE_IVAR: &str = "patinaState";

impl ViewState {
    unsafe fn from_view<'a>(this: Id) -> Option<&'a mut ViewState> {
        let ivar = CString::new(STATE_IVAR).unwrap();
        let mut ptr: *mut c_void = null_mut();
        object_getInstanceVariable(this, ivar.as_ptr(), &mut ptr);
        (ptr as *mut ViewState).as_mut()
    }

    /// Physical pixels-per-point from the hosting window (Retina = 2).
    unsafe fn ppp(&self) -> f32 {
        let window = send0(self.view, sel("window"));
        if window.is_null() {
            2.0
        } else {
            let scale = send0_f64(window, sel("backingScaleFactor"));
            if scale > 0.0 {
                scale as f32
            } else {
                2.0
            }
        }
    }

    /// Run egui once and draw the result into the view's current AppKit
    /// graphics context. Called from `drawRect:` so it rides the standard
    /// display cycle — which the out-of-process AU view service pumps, unlike
    /// a private NSTimer or manual layer.contents push.
    unsafe fn draw_current(&mut self) {
        let ppp = self.ppp();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(EDITOR_WIDTH as f32, EDITOR_HEIGHT as f32),
            )),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.pending),
            focused: true,
            ..Default::default()
        };
        self.ctx.set_pixels_per_point(ppp);

        // The editor closure needs &mut self.editor; split the borrow.
        let editor = &mut self.editor;
        let output = self.ctx.run(raw, |ctx| editor.update(ctx));
        self.raster.update_textures(&output.textures_delta);
        let prims = self.ctx.tessellate(output.shapes, ppp);

        let pw = (EDITOR_WIDTH as f32 * ppp).round() as usize;
        let ph = (EDITOR_HEIGHT as f32 * ppp).round() as usize;
        self.raster.resize(pw, ph);
        // Dark base in case any pixel escapes the full-screen backdrop.
        self.raster.clear([0x0a, 0x11, 0x14]);
        self.raster.paint(&prims, ppp);

        let bitmap = CGBitmapContextCreate(
            self.raster.fb.as_mut_ptr() as *mut c_void,
            pw,
            ph,
            8,
            pw * 4,
            self.colorspace,
            kCGImageAlphaPremultipliedLast,
        );
        if bitmap.is_null() {
            return;
        }
        let image = CGBitmapContextCreateImage(bitmap);
        CGContextRelease(bitmap);
        if image.is_null() {
            return;
        }

        // Draw into the view's current CGContext at its bounds (points); the
        // context already carries the backing-scale transform, so the
        // physical-pixel image maps 1:1.
        let nsgc = send0(cls("NSGraphicsContext"), sel("currentContext"));
        if !nsgc.is_null() {
            let cg = send0(nsgc, sel("CGContext"));
            if !cg.is_null() {
                let h = EDITOR_HEIGHT as f64;
                let bounds = CGRect {
                    origin: CGPoint { x: 0.0, y: 0.0 },
                    size: CGSize { width: EDITOR_WIDTH as f64, height: h },
                };
                // Our framebuffer is top-to-bottom; the view is flipped, so
                // flip the y-axis to cancel CGContextDrawImage's bottom-up
                // default and land the image right-side up.
                CGContextSaveGState(cg);
                CGContextTranslateCTM(cg, 0.0, h);
                CGContextScaleCTM(cg, 1.0, -1.0);
                CGContextDrawImage(cg, bounds, image);
                CGContextRestoreGState(cg);
            }
        }
        CGImageRelease(image);
    }

    /// Ask AppKit to redraw the view on the next display pass.
    unsafe fn set_needs_display(&self) {
        send_void_bool(self.view, sel("setNeedsDisplay:"), 1);
    }

    /// Convert an NSEvent's window-space location into egui points.
    unsafe fn event_pos(&self, event: Id) -> Pos2 {
        let win_pt = send0_point(event, sel("locationInWindow"));
        // convertPoint:fromView:nil -> our (flipped) view coordinates.
        let f: unsafe extern "C" fn(Id, Sel, CGPoint, Id) -> CGPoint =
            transmute(objc_msgSend as *const c_void);
        let local = f(self.view, sel("convertPoint:fromView:"), win_pt, null_mut());
        pos2(local.x as f32, local.y as f32)
    }
}

// ---------------------------------------------------------------------------
// The two runtime classes: the software-rendered view and the CocoaUI factory
// ---------------------------------------------------------------------------

static VIEW_CLASS: AtomicUsize = AtomicUsize::new(0);
static FACTORY_CLASS: AtomicUsize = AtomicUsize::new(0);

// Plain, stable class names. The host resolves the factory by name through
// the bundle (Info.plist NSPrincipalClass names this same string), so it has
// to match exactly and must not move with the crate version.
fn versioned(name: &str) -> String {
    name.to_string()
}

pub fn factory_class_name() -> String {
    "PatinaAUViewFactory".to_string()
}

extern "C" {
    /// Defined in src/au/factory.m purely so the linker keeps that object
    /// file — and with it the PatinaAUViewFactory class the host resolves
    /// by name.
    fn patina_au_factory_anchor();
}

/// Register both Objective-C classes. Safe — and necessary — to call
/// repeatedly, and is only ever called lazily (never from a library
/// initializer). `NSView` doesn't resolve until AppKit is mapped, so a
/// one-shot registration that ran too early would leave VIEW_CLASS null
/// forever and `uiViewForAudioUnit:` could only hand the host nil. Each
/// call retries whatever is still missing.
pub fn register_classes() {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if VIEW_CLASS.load(Ordering::Acquire) != 0 && FACTORY_CLASS.load(Ordering::Acquire) != 0 {
        return;
    }
    let _guard = LOCK.lock();
    unsafe {
        patina_au_factory_anchor();
        if VIEW_CLASS.load(Ordering::Acquire) == 0 {
            register_view_class();
        }
        if FACTORY_CLASS.load(Ordering::Acquire) == 0 {
            register_factory_class();
        }
    }
}

/// Look up a class we may have registered on an earlier attempt (or that a
/// previous load of this dylib left in the runtime's flat namespace).
unsafe fn existing_class(name: &CString) -> Class {
    objc_getClass(name.as_ptr())
}

unsafe fn register_view_class() {
    let name = CString::new(versioned("PatinaAUView")).unwrap();
    let already = existing_class(&name);
    if !already.is_null() {
        VIEW_CLASS.store(already as usize, Ordering::Release);
        return;
    }
    // NSView only resolves once AppKit is mapped, and the AU hosting
    // service does not link it, so we pull AppKit in ourselves. This runs
    // ONLY lazily, when the host asks us to build a view: dlopen from a
    // library initializer re-enters the dyld lock and deadlocks the whole
    // hosting process (which reads to the user as the audio engine dying),
    // so nothing here may run at load time.
    let mut superclass = cls("NSView");
    if superclass.is_null() {
        let appkit = CString::new("/System/Library/Frameworks/AppKit.framework/AppKit").unwrap();
        dlopen(appkit.as_ptr(), RTLD_LAZY);
        superclass = cls("NSView");
    }
    if superclass.is_null() {
        return;
    }
    let class = objc_allocateClassPair(superclass, name.as_ptr(), 0);
    if class.is_null() {
        return;
    }
    let ivar = CString::new(STATE_IVAR).unwrap();
    class_addIvar(
        class,
        ivar.as_ptr(),
        size_of::<*mut c_void>(),
        align_of::<*mut c_void>().trailing_zeros() as u8,
        CString::new("^v").unwrap().as_ptr(),
    );

    let add = |sel_name: &str, imp: *const c_void, types: &str| {
        class_addMethod(class, sel(sel_name), imp, CString::new(types).unwrap().as_ptr());
    };
    add("isFlipped", view_is_flipped as *const c_void, "B@:");
    // The out-of-process view bridge sizes the remote container from Auto
    // Layout, which reads this; without it the container collapses to 1×1.
    add("intrinsicContentSize", intrinsic_content_size as *const c_void, "{CGSize=dd}@:");
    add("acceptsFirstMouse:", accepts_first_mouse as *const c_void, "B@:@");
    add("drawRect:", draw_rect as *const c_void, "v@:{CGRect={CGPoint=dd}{CGSize=dd}}");
    add("drawTick:", draw_tick as *const c_void, "v@:@");
    add("mouseDown:", mouse_down as *const c_void, "v@:@");
    add("mouseDragged:", mouse_dragged as *const c_void, "v@:@");
    add("mouseUp:", mouse_up as *const c_void, "v@:@");
    add("rightMouseDown:", right_mouse_down as *const c_void, "v@:@");
    add("rightMouseUp:", right_mouse_up as *const c_void, "v@:@");
    add("mouseMoved:", mouse_moved as *const c_void, "v@:@");
    add("mouseDragged:", mouse_dragged as *const c_void, "v@:@");
    add("mouseExited:", mouse_exited as *const c_void, "v@:@");
    add("scrollWheel:", scroll_wheel as *const c_void, "v@:@");
    add("viewDidMoveToWindow", view_did_move_to_window as *const c_void, "v@:");
    add("dealloc", view_dealloc as *const c_void, "v@:");
    objc_registerClassPair(class);
    VIEW_CLASS.store(class as usize, Ordering::Release);
}

unsafe fn register_factory_class() {
    let name = CString::new(factory_class_name()).unwrap();
    let already = existing_class(&name);
    if !already.is_null() {
        FACTORY_CLASS.store(already as usize, Ordering::Release);
        return;
    }
    let superclass = cls("NSObject");
    if superclass.is_null() {
        return;
    }
    let factory = objc_allocateClassPair(superclass, name.as_ptr(), 0);
    if factory.is_null() {
        return;
    }
    // AUCocoaUIBase declares BOTH interfaceVersion and uiViewForAudioUnit:
    // as INSTANCE methods. Registering interfaceVersion on the metaclass
    // (a class method) made the bridge's `[factory interfaceVersion]` fail
    // conformance, so it never called our factory.
    class_addMethod(
        factory,
        sel("interfaceVersion"),
        interface_version as *const c_void,
        CString::new("I@:").unwrap().as_ptr(),
    );
    class_addMethod(
        factory,
        sel("description"),
        factory_description as *const c_void,
        CString::new("@@:").unwrap().as_ptr(),
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
}

unsafe extern "C" fn factory_description(_this: Id, _cmd: Sel) -> Id {
    // A CFString is toll-free bridged to NSString; the host treats it as
    // autoreleased, which matches -description's contract.
    cfstring("Patina") as Id
}

/// Start the ~30 Hz redraw timer. NSTimer RETAINS its target, so the view
/// cannot deallocate while this is live — `viewDidMoveToWindow` invalidates
/// it when the host pulls the view out of its window, otherwise the timer
/// outlives the plugin and keeps polling an AudioUnit the host has already
/// disposed (a use-after-free that reads as "the AU made things unstable").
unsafe fn schedule_redraw_timer(view: Id) -> Id {
    let sched: unsafe extern "C" fn(Id, Sel, f64, Id, Sel, Id, u8) -> Id =
        transmute(objc_msgSend as *const c_void);
    sched(
        cls("NSTimer"),
        sel("scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:"),
        FRAME_INTERVAL,
        view,
        sel("drawTick:"),
        null_mut(),
        1,
    )
}

/// The host adds our view to a window to show the panel and removes it when
/// the panel closes. Track that to keep the redraw timer alive exactly as
/// long as the view is actually on screen.
unsafe extern "C" fn view_did_move_to_window(this: Id, _cmd: Sel) {
    let on_screen = !send0(this, sel("window")).is_null();
    if let Some(state) = ViewState::from_view(this) {
        if on_screen {
            if state.timer.is_null() {
                state.timer = schedule_redraw_timer(this);
            }
        } else if !state.timer.is_null() {
            let _: Id = send0(state.timer, sel("invalidate"));
            state.timer = null_mut();
        }
    }
}

// --- View method implementations --------------------------------------------

unsafe extern "C" fn view_is_flipped(_this: Id, _cmd: Sel) -> u8 {
    1 // top-left origin, matching egui
}

unsafe extern "C" fn intrinsic_content_size(_this: Id, _cmd: Sel) -> CGSize {
    CGSize { width: EDITOR_WIDTH as f64, height: EDITOR_HEIGHT as f64 }
}

unsafe extern "C" fn accepts_first_mouse(_this: Id, _cmd: Sel, _event: Id) -> u8 {
    1
}

unsafe extern "C" fn draw_rect(this: Id, _cmd: Sel, _dirty: CGRect) {
    if let Some(state) = ViewState::from_view(this) {
        state.draw_current();
    }
}

/// Timer only marks the view dirty; the actual paint happens in drawRect:
/// on AppKit's display pass, so it works even where our timer wouldn't.
unsafe extern "C" fn draw_tick(this: Id, _cmd: Sel, _timer: Id) {
    if let Some(state) = ViewState::from_view(this) {
        state.set_needs_display();
    }
}

unsafe fn push_button(this: Id, event: Id, button: PointerButton, pressed: bool) {
    if let Some(state) = ViewState::from_view(this) {
        let pos = state.event_pos(event);
        state.mouse = pos;
        state.pending.push(Event::PointerMoved(pos));
        state.pending.push(Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::default(),
        });
        state.set_needs_display();
    }
}

unsafe extern "C" fn mouse_down(this: Id, _cmd: Sel, event: Id) {
    push_button(this, event, PointerButton::Primary, true);
}
unsafe extern "C" fn mouse_up(this: Id, _cmd: Sel, event: Id) {
    push_button(this, event, PointerButton::Primary, false);
}
unsafe extern "C" fn right_mouse_down(this: Id, _cmd: Sel, event: Id) {
    push_button(this, event, PointerButton::Secondary, true);
}
unsafe extern "C" fn right_mouse_up(this: Id, _cmd: Sel, event: Id) {
    push_button(this, event, PointerButton::Secondary, false);
}

unsafe extern "C" fn mouse_moved(this: Id, _cmd: Sel, event: Id) {
    if let Some(state) = ViewState::from_view(this) {
        let pos = state.event_pos(event);
        state.mouse = pos;
        state.pending.push(Event::PointerMoved(pos));
        state.set_needs_display();
    }
}

unsafe extern "C" fn mouse_dragged(this: Id, _cmd: Sel, event: Id) {
    if let Some(state) = ViewState::from_view(this) {
        let pos = state.event_pos(event);
        state.mouse = pos;
        state.pending.push(Event::PointerMoved(pos));
        state.set_needs_display();
    }
}

unsafe extern "C" fn mouse_exited(this: Id, _cmd: Sel, _event: Id) {
    if let Some(state) = ViewState::from_view(this) {
        state.pending.push(Event::PointerGone);
    }
}

unsafe extern "C" fn scroll_wheel(this: Id, _cmd: Sel, event: Id) {
    if let Some(state) = ViewState::from_view(this) {
        let dx = send0_f64(event, sel("scrollingDeltaX")) as f32;
        let dy = send0_f64(event, sel("scrollingDeltaY")) as f32;
        state.pending.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(dx, dy),
            modifiers: Modifiers::default(),
        });
        state.set_needs_display();
    }
}

unsafe extern "C" fn view_dealloc(this: Id, _cmd: Sel) {
    let ivar = CString::new(STATE_IVAR).unwrap();
    let mut ptr: *mut c_void = null_mut();
    object_getInstanceVariable(this, ivar.as_ptr(), &mut ptr);
    if !ptr.is_null() {
        object_setInstanceVariable(this, ivar.as_ptr(), null_mut());
        let state = Box::from_raw(ptr as *mut ViewState);
        // Stop the timer if it is still live (it retains the view, so it is
        // normally already invalidated by viewDidMoveToWindow).
        if !state.timer.is_null() {
            let _: Id = send0(state.timer, sel("invalidate"));
        }
        CGColorSpaceRelease(state.colorspace);
    }
    let sup = ObjcSuper { receiver: this, super_class: cls("NSView") };
    let send: unsafe extern "C" fn(*const ObjcSuper, Sel) =
        transmute(objc_msgSendSuper as *const c_void);
    send(&sup, sel("dealloc"));
}

// --- Factory method implementations ------------------------------------------

unsafe extern "C" fn interface_version(_this: Id, _cmd: Sel) -> u32 {
    0
}

/// Entry point for the compiled Objective-C factory (src/au/factory.m).
/// Returns a +1 NSView; the host releases it, per AUCocoaUIBase.
#[no_mangle]
pub unsafe extern "C" fn patina_au_create_view(audio_unit: *mut c_void) -> *mut c_void {
    ui_view_for_audio_unit(
        null_mut(),
        null_mut(),
        audio_unit,
        CGSize { width: EDITOR_WIDTH as f64, height: EDITOR_HEIGHT as f64 },
    )
}

unsafe extern "C" fn ui_view_for_audio_unit(
    _this: Id,
    _cmd: Sel,
    audio_unit: *mut c_void,
    _size: CGSize,
) -> Id {
    // AppKit is certainly up by now, so this picks up the view class even if
    // the load-time attempt ran too early to build it.
    register_classes();
    let view_cls = VIEW_CLASS.load(Ordering::Acquire) as Class;
    if audio_unit.is_null() || view_cls.is_null() {
        return null_mut();
    }

    let frame = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize { width: EDITOR_WIDTH as f64, height: EDITOR_HEIGHT as f64 },
    };
    let view = send0(view_cls, sel("alloc"));
    let init: unsafe extern "C" fn(Id, Sel, CGRect) -> Id =
        transmute(objc_msgSend as *const c_void);
    let view = init(view, sel("initWithFrame:"), frame);
    if view.is_null() {
        return null_mut();
    }

    // Layer-backed so the bitmap remotes through the ViewBridge.
    send_void_bool(view, sel("setWantsLayer:"), 1);

    // Track mouse-move/exit over the whole (auto-resizing) visible rect so
    // hover feedback works. Options: MouseEnteredAndExited(1) | MouseMoved(2)
    // | ActiveAlways(0x80) | InVisibleRect(0x200).
    let tracking_cls = cls("NSTrackingArea");
    let ta_alloc = send0(tracking_cls, sel("alloc"));
    let ta_init: unsafe extern "C" fn(Id, Sel, CGRect, u64, Id, Id) -> Id =
        transmute(objc_msgSend as *const c_void);
    let zero = CGRect { origin: CGPoint { x: 0.0, y: 0.0 }, size: CGSize { width: 0.0, height: 0.0 } };
    let tracking = ta_init(
        ta_alloc,
        sel("initWithRect:options:owner:userInfo:"),
        zero,
        0x1 | 0x2 | 0x80 | 0x200,
        view,
        null_mut(),
    );
    send_void_id(view, sel("addTrackingArea:"), tracking);
    let _: Id = send0(tracking, sel("release"));

    // Build the egui side.
    let host = Arc::new(AuParamHost { au: audio_unit });
    let ctx = egui::Context::default();
    let mut state = Box::new(ViewState {
        view,
        ctx,
        editor: EditorState::new(host),
        raster: Raster::new(),
        colorspace: CGColorSpaceCreateDeviceRGB(),
        start: Instant::now(),
        mouse: pos2(-1.0, -1.0),
        pending: Vec::new(),
        timer: null_mut(),
    });

    // Repeating timer drives redraws. scheduledTimer... returns an
    // autoreleased, run-loop-retained timer; it retains `view` as its
    // target until invalidated in dealloc.
    state.timer = schedule_redraw_timer(view);

    let ivar = CString::new(STATE_IVAR).unwrap();
    object_setInstanceVariable(view, ivar.as_ptr(), Box::into_raw(state) as *mut c_void);

    // Ask for the first paint on the next display pass.
    send_void_bool(view, sel("setNeedsDisplay:"), 1);
    // Returned +1 per AUCocoaUIBase convention; the host releases it.
    view
}

/// Build the (bundle URL, view class name) pair for
/// kAudioUnitProperty_CocoaUI. Returns None if the bundle path can't be
/// resolved (the host then uses its generic view).
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
