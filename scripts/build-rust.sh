#!/bin/sh
# Build the Rust core for Android and generate Kotlin bindings. Gradle runs this before every
# build (preBuild), including from Android Studio, whose PATH often lacks cargo, so the tools are
# looked up in their usual install locations too.
# Needs: cargo, the aarch64-linux-android and x86_64-linux-android Rust targets, cargo-ndk, an NDK.
set -e
cd "$(dirname "$0")/.."

for d in "$HOME/.cargo/bin" "$HOME/.local/bin" $(ls -d "$HOME"/.rustup/toolchains/stable-*/bin 2>/dev/null); do
    [ -d "$d" ] && PATH="$PATH:$d"
done
export PATH
command -v cargo >/dev/null || { echo "build-rust.sh: cargo not found" >&2; exit 1; }
command -v cargo-ndk >/dev/null || { echo "build-rust.sh: cargo-ndk not found (cargo install cargo-ndk)" >&2; exit 1; }

# NDK: ANDROID_NDK_HOME, else the newest NDK in the SDK (ANDROID_HOME or local.properties).
if [ -z "$ANDROID_NDK_HOME" ]; then
    SDK=${ANDROID_HOME:-$(sed -n 's/^sdk\.dir=//p' android/local.properties 2>/dev/null)}
    SDK=${SDK:-$HOME/Android/Sdk}
    ANDROID_NDK_HOME=$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1)
    export ANDROID_NDK_HOME
fi
[ -d "$ANDROID_NDK_HOME" ] || { echo "build-rust.sh: no NDK found (set ANDROID_NDK_HOME)" >&2; exit 1; }

# App version for the User-Agent (core/src/config.rs).
LG_VERSION=${LG_VERSION:-$(cat version.txt)}
export LG_VERSION

PROFILE=${RUST_PROFILE:-release}
JNI=android/app/src/main/jniLibs
cargo ndk -t arm64-v8a -t x86_64 -P 26 -o "$JNI" build --profile "$PROFILE" -p lootlens-ffi
# Bindings come from the host build of the same crate (metadata only).
cargo build -q -p lootlens-ffi
cargo run -q -p lootlens-ffi --bin uniffi-bindgen -- generate \
    --library target/debug/liblootlens_ffi.so --language kotlin \
    --out-dir android/app/src/main/java --no-format
