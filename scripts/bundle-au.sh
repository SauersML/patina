#!/usr/bin/env bash
# Build the native AUv2 Audio Unit, wrap it in Patina.component, install it
# where Logic Pro / GarageBand scan, and (with --validate) run auval against
# it. Wired into install-plugins.sh so the AU always matches the code.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "Audio Units are macOS-only; skipping."
    exit 0
fi

# Component identity — must match src/au/mod.rs (AU_TYPE/SUBTYPE/MANUFACTURER)
AU_TYPE="aumu"
AU_SUBTYPE="Ptna"
AU_MANU="Saur"

# 0.1.0 -> 0x000100 -> 256, the integer AudioComponents version format
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
IFS=. read -r vmaj vmin vpat <<<"$version"
version_int=$(( (vmaj << 16) | (vmin << 8) | vpat ))

echo "[$(date '+%Y-%m-%d %H:%M:%S')] building Audio Unit v$version ($version_int)"
CARGO_INCREMENTAL=0 cargo build --release --no-default-features --features au

bundle="target/bundled/Patina.component"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
cp target/release/libpatina.dylib "$bundle/Contents/MacOS/Patina"
printf 'BNDL????' > "$bundle/Contents/PkgInfo"

cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>English</string>
	<key>CFBundleDisplayName</key>
	<string>Patina</string>
	<key>CFBundleExecutable</key>
	<string>Patina</string>
	<key>CFBundleIdentifier</key>
	<string>com.sauers.patina.component</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Patina</string>
	<key>CFBundlePackageType</key>
	<string>BNDL</string>
	<!-- The AU view bridge resolves our Cocoa view factory through the
	     bundle; naming it here is how working AUv2 plugins expose it. -->
	<key>NSPrincipalClass</key>
	<string>PatinaAUViewFactory</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>AudioComponents</key>
	<array>
		<dict>
			<key>description</key>
			<string>Circuit-modeled polyphonic synthesizer with tape, spring, and germanium fuzz</string>
			<key>factoryFunction</key>
			<string>PatinaAUFactory</string>
			<key>manufacturer</key>
			<string>$AU_MANU</string>
			<key>name</key>
			<string>Sauers: Patina</string>
			<key>sandboxSafe</key>
			<true/>
			<key>subtype</key>
			<string>$AU_SUBTYPE</string>
			<key>tags</key>
			<array>
				<string>Synthesizer</string>
			</array>
			<key>type</key>
			<string>$AU_TYPE</string>
			<key>version</key>
			<integer>$version_int</integer>
		</dict>
	</array>
</dict>
</plist>
PLIST

codesign --force --sign - "$bundle"

components_dir="$HOME/Library/Audio/Plug-Ins/Components"
mkdir -p "$components_dir"
rm -rf "$components_dir/Patina.component"
cp -R "$bundle" "$components_dir/"
echo "[$(date '+%Y-%m-%d %H:%M:%S')] installed Patina.component -> $components_dir"

# Nudge the component registrar so hosts see the new build immediately
killall -9 AudioComponentRegistrar 2>/dev/null || true

if [[ "${1:-}" == "--validate" ]]; then
    auval -v "$AU_TYPE" "$AU_SUBTYPE" "$AU_MANU"
fi
