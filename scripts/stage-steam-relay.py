#!/usr/bin/env python3
"""Stage the optional native Linux relay with its locked Steam SDK library."""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


TARGET = "x86_64-unknown-linux-gnu"
SDK_VERSION = "0.13.0"


def check_platform(repo):
    result = subprocess.run(["rustc", "-vV"], cwd=repo, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"Could not determine the Rust host (rustc exited {result.returncode}).")
    host = next((line.removeprefix("host: ") for line in result.stdout.splitlines()
                 if line.startswith("host: ")), "unknown")
    if host != TARGET:
        raise RuntimeError(f"Steam relay staging requires native {TARGET}; Rust host is {host}.")
    if "STEAM_SDK_LOCATION" in os.environ:
        raise RuntimeError("Unset STEAM_SDK_LOCATION to build and stage the locked Steam SDK.")


def stage(repo, profile):
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET],
        cwd=repo, capture_output=True, text=True,
    )
    if result.returncode:
        raise RuntimeError(f"Could not locate the locked Steam SDK: cargo metadata exited {result.returncode}.")
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("Cargo metadata did not return valid JSON.") from error
    if not isinstance(metadata, dict) or not isinstance(metadata.get("packages"), list):
        raise RuntimeError("Cargo metadata is missing its package list.")
    packages = [package for package in metadata["packages"]
                if isinstance(package, dict) and package.get("name") == "steamworks-sys"]
    if len(packages) != 1 or packages[0].get("version") != SDK_VERSION:
        raise RuntimeError(f"Expected exactly one steamworks-sys {SDK_VERSION} in Cargo metadata.")
    manifest_path = packages[0].get("manifest_path")
    target_directory = metadata.get("target_directory")
    if not isinstance(manifest_path, str) or not manifest_path or not Path(manifest_path).is_absolute():
        raise RuntimeError("Cargo metadata is missing the Steam SDK manifest path.")
    if not isinstance(target_directory, str) or not target_directory or not Path(target_directory).is_absolute():
        raise RuntimeError("Cargo metadata is missing its target directory.")
    sdk = Path(manifest_path).parent
    target = Path(target_directory)
    relay = target / TARGET / profile / "skate-steam-relay"
    library = sdk / "lib/steam/redistributable_bin/linux64/libsteam_api.so"
    if not relay.is_file() or not os.access(relay, os.X_OK):
        raise RuntimeError(f"Build the native skate-steam-relay executable before staging: {relay}")
    if not library.is_file():
        raise RuntimeError(f"Missing locked Steam SDK library: {library}")
    destination = target / profile / "steam-relay"
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(relay, destination / relay.name)
    shutil.copy2(library, destination / library.name)
    print(f"Steam relay ready: {destination}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("debug", "release"), default="debug")
    parser.add_argument("--check-platform", action="store_true",
                        help="validate the native Steam target and print its triple without staging")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    try:
        check_platform(repo)
        if args.check_platform:
            print(TARGET)
        else:
            stage(repo, args.profile)
    except (OSError, RuntimeError) as error:
        print(f"Steam relay: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
