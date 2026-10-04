# Skate Multiplayer Platform

This context defines the shared language for the community-hosted multiplayer and mod platform built around the Skate 3 Rust engine.

## Language

**Dedicated Server**:
A headless, community-hosted process that owns a multiplayer server's canonical shared gameplay state and resource set.
_Avoid_: Relay, lobby host, peer host

**Hybrid Authority**:
The dedicated server owns canonical shared state and validates player actions, while an owning client predicts its detailed skating simulation for responsive control.
_Avoid_: Peer authority, fully client-authoritative simulation, lockstep physics
