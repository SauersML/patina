// The AUCocoaUIBase factory, as a REAL compiled Objective-C class.
//
// It has to be compiled rather than registered at runtime with
// objc_allocateClassPair: the host resolves this class by name *through the
// bundle* (Info.plist NSPrincipalClass names it, and the CocoaUI property
// hands back this class name plus the bundle path). A class conjured at
// runtime is not attributed to the bundle's image, so bundle-scoped lookup
// (-[NSBundle classNamed:]/-principalClass) cannot find it and the view is
// never created — which is exactly how the panel came up blank out of
// process. Every AUv2 that renders a custom view out of process ships a
// compiled class here.
//
// The class is a thin shim: all the real work (building the NSView that
// software-renders the egui panel) stays in Rust, in src/au/cocoa.rs.

#import <AppKit/AppKit.h>
#import <AudioUnit/AudioUnit.h>
#import <Foundation/Foundation.h>

// Implemented in Rust (src/au/cocoa.rs).
extern void *patina_au_create_view(void *audioUnit);

// Nothing in Rust names PatinaAUViewFactory directly — the host looks it up
// by string — so without a referenced symbol the linker drops this whole
// object file and the class silently vanishes from the binary. Rust calls
// this to anchor it.
void patina_au_factory_anchor(void) {}

@interface PatinaAUViewFactory : NSObject
- (unsigned)interfaceVersion;
- (NSView *)uiViewForAudioUnit:(AudioUnit)inAudioUnit withSize:(NSSize)inPreferredSize;
@end

@implementation PatinaAUViewFactory

- (unsigned)interfaceVersion {
    return 0;
}

- (NSString *)description {
    return @"Patina";
}

- (NSView *)uiViewForAudioUnit:(AudioUnit)inAudioUnit withSize:(NSSize)inPreferredSize {
    (void)inPreferredSize;
    // Rust returns a +1 NSView; AUCocoaUIBase's contract has the host
    // release it, so hand it straight back.
    return (NSView *)patina_au_create_view((void *)inAudioUnit);
}

@end
