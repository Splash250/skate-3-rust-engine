#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
profile=debug
build_command="./BUILD.sh"
if [ "${SKATE_RELEASE:-}" = 1 ]; then
    profile=release
    build_command="./BUILD.sh --release"
fi
game="$repo/target/$profile/skate3rust"
if [ ! -x "$game" ]; then
    echo "Game executable is missing. Run: $build_command" >&2
    exit 2
fi
# Cargo places the hashed Bevy development dylib and Rust's shared stdlib in
# the profile's deps directory. Match `cargo run`'s loader environment.
rust_lib=$(rustc --print target-libdir)
LD_LIBRARY_PATH="$repo/target/$profile/deps:$rust_lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export LD_LIBRARY_PATH
if [ "${SKATE3_MODS+x}" != x ]; then
    SKATE3_MODS="$repo/mods"
    export SKATE3_MODS
fi
if [ -d "$repo/assets" ]; then
    exec "$game" --assets "$repo/assets" "$@"
fi
exec "$game" "$@"
