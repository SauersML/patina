// Compiles the Objective-C AU view factory into the library.
//
// It cannot be a runtime-registered class: the AU view bridge looks the
// factory up by name through the plugin bundle, and a class created with
// objc_allocateClassPair isn't attributed to the bundle's image, so that
// lookup fails and the custom panel never gets built. See src/au/factory.m.

fn main() {
    println!("cargo:rerun-if-changed=src/au/factory.m");
    println!("cargo:rerun-if-changed=build.rs");

    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    let editor = std::env::var("CARGO_FEATURE_EDITOR").is_ok();
    if !(macos && editor) {
        return;
    }

    cc::Build::new()
        .file("src/au/factory.m")
        .flag("-fno-objc-arc")
        .compile("patina_au_factory");

    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Foundation");
}
