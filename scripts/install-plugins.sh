#!/usr/bin/env bash
# Rebuild the CLAP/VST3/AU bundles and install them where DAWs scan, so the
# plugins always match the code. Wired to the post-commit/post-merge hooks
# (see .githooks/) — normally you never run this by hand.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "[$(date '+%Y-%m-%d %H:%M:%S')] bundling plugins at $(git rev-parse --short HEAD 2>/dev/null || echo '?')"

cargo xtask bundle patina --release --no-default-features --features plugin

case "$(uname -s)" in
    Darwin)
        clap_dir="$HOME/Library/Audio/Plug-Ins/CLAP"
        vst3_dir="$HOME/Library/Audio/Plug-Ins/VST3"
        ;;
    Linux)
        clap_dir="$HOME/.clap"
        vst3_dir="$HOME/.vst3"
        ;;
    *)
        echo "unsupported OS for auto-install; bundles are in target/bundled/"
        exit 0
        ;;
esac

mkdir -p "$clap_dir" "$vst3_dir"
rm -rf "$clap_dir/Patina.clap" "$vst3_dir/Patina.vst3"
cp -R target/bundled/Patina.clap "$clap_dir/"
cp -R target/bundled/Patina.vst3 "$vst3_dir/"

echo "[$(date '+%Y-%m-%d %H:%M:%S')] installed Patina.clap -> $clap_dir, Patina.vst3 -> $vst3_dir"

# The native Audio Unit (Logic Pro / GarageBand); macOS only, no-op elsewhere
scripts/bundle-au.sh
