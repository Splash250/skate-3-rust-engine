//! Native-input-v1 records controller input only. Physical SDK mutations must
//! not enter that prediction stream; visual layers and cleanup remain usable.
use super::{Command, Mods};
use bevy::prelude::*;

pub(crate) fn ready(world: &World) -> Result<(), String> {
    let occupied = world.get_resource::<Mods>().is_some_and(|mods| {
        mods.attach.is_some()
            || mods.detach_pending.is_some()
            || !mods.suspended_by.is_empty()
            || !mods.bodies.is_empty()
            || !mods.graph_gates.is_empty()
    }) || world
        .get_resource::<crate::physics::SkaterRuntime>()
        .is_some_and(|skater| {
            !skater.mod_joint_overrides.is_empty() || !skater.mod_part_overrides.is_empty()
        });
    if occupied {
        return Err("Clear local physical bodies, player suspension, attachments and native overrides before a native attempt.".into());
    }
    Ok(())
}

pub(super) fn check(world: &World, command: &Command) -> Result<(), String> {
    if !crate::multiplayer::native_authority::active(world) {
        return Ok(());
    }
    let physical = matches!(
        command,
        Command::NativeImpulse { .. }
            | Command::PlayerJoint { .. }
            | Command::PlayerTeleport { .. }
            | Command::PlayerAttach { .. }
            | Command::SessionTeleport { .. }
            | Command::PlayerSuspend { suspended: true }
            | Command::RigPart {
                options: Some(_),
                ..
            }
            | Command::GraphGate {
                enabled: Some(_),
                ..
            }
            | Command::PhysicsSpawn { .. }
            | Command::PhysicsAddCollider { .. }
            | Command::PhysicsForce { .. }
            | Command::PhysicsImpulse { .. }
            | Command::PhysicsTorque { .. }
            | Command::PhysicsTorqueImpulse { .. }
            | Command::PhysicsSetLinvel { .. }
            | Command::PhysicsSetAngvel { .. }
            | Command::PhysicsSetPose { .. }
            | Command::PhysicsRevolute { .. }
            | Command::PhysicsPrismatic { .. }
            | Command::PhysicsJointMotor { .. }
            | Command::PhysicsJointSpring { .. }
    );
    if physical {
        return Err("Cancel or finish the native attempt before changing local physics or native graphs; native-input-v1 accepts controller input only.".into());
    }
    // Request wrappers reach this guard again through apply_one, allowing the
    // existing command-result mechanism to report a denied leaf to its owner.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_prediction_rejects_unlogged_changes_but_retains_inputs_and_cleanup() {
        let mutations = [
            json!({"kind":"native_impulse","body":{"kind":"board","index":0},"impulse":[1,0,0],"point":null,"angular":false}),
            json!({"kind":"player_teleport","options":{"position":[0,1,0]}}),
            json!({"kind":"player_suspend","suspended":true}),
            json!({"kind":"player_joint","joint":0,"options":{"free_swing":true}}),
            json!({"kind":"rig_part","index":0,"options":{"collision":false}}),
            json!({"kind":"graph_gate","graph":"air","target":"state","index":0,"enabled":false}),
            json!({"kind":"physics_force","key":"body","force":[1,0,0]}),
        ];
        let mut world = World::new();
        let commands: Vec<Command> = mutations
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect();
        for command in &commands {
            assert!(check(&world, command).is_ok());
        }
        world.insert_resource(crate::multiplayer::native_authority::active_prediction());
        for command in &commands {
            assert!(
                check(&world, command)
                    .unwrap_err()
                    .contains("native-input-v1")
            );
        }
        for value in [
            json!({"kind":"input_override","action":3,"value":1}),
            json!({"kind":"player_suspend","suspended":false}),
            json!({"kind":"player_reset_joints"}),
            json!({"kind":"physics_remove","key":"body"}),
            json!({"kind":"graph_gate","graph":"air","target":"state","index":0,"enabled":null}),
            json!({"kind":"overlay","key":"score","text":"Ready"}),
            json!({"kind":"engine_inspect","system":"physics"}),
        ] {
            let command: Command = serde_json::from_value(value).unwrap();
            assert!(check(&world, &command).is_ok(), "{command:?}");
        }
        world.remove_resource::<crate::multiplayer::native_authority::Prediction>();
        for command in &commands {
            assert!(check(&world, command).is_ok());
        }
    }
}
