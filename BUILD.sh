#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
release=
steam=
browser=
usage() { echo "Usage: $0 [--release] [--steam] [--browser]" >&2; exit 2; }
for argument in "$@"; do
    case "$argument" in
        --release) [ -z "$release" ] || usage; release=1 ;;
        --steam) [ -z "$steam" ] || usage; steam=1 ;;
        --browser) [ -z "$browser" ] || usage; browser=1 ;;
        *) usage ;;
    esac
done

cd "$repo"
if [ -n "$steam" ]; then
    if [ "${CARGO_BUILD_TARGET+x}" = x ]; then
        echo "Unset CARGO_BUILD_TARGET for --steam; the game launcher requires Cargo's default native output layout." >&2
        exit 2
    fi
    steam_target=$(python3 scripts/stage-steam-relay.py --check-platform)
fi

if ldd --version 2>&1 | grep -qi musl; then
    RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C target-feature=-crt-static"
    export RUSTFLAGS
fi

if [ -n "$release" ]; then
    cargo build --locked -p skate-game --bin skate3rust --release
    cargo build --locked -p skate-xiso --release
else
    cargo build --locked -p skate-game --bin skate3rust
    cargo build --locked -p skate-xiso
fi

if [ -n "$browser" ]; then
    if [ -n "$release" ]; then
        cargo build --locked -p skate-browser --features host --release
    else
        cargo build --locked -p skate-browser --features host
    fi
fi

if [ -n "$steam" ]; then
    # Explicitly select the supported native SDK target. The staged copy stays
    # beside the game executable in Cargo's default native output layout.
    if [ -n "$release" ]; then
        cargo build --locked -p skate-steam-relay --target "$steam_target" --release
        python3 scripts/stage-steam-relay.py --profile release
    else
        cargo build --locked -p skate-steam-relay --target "$steam_target"
        python3 scripts/stage-steam-relay.py
    fi
fi
