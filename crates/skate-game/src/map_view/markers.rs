use super::view::ViewFrame;
use bevy::prelude::*;
pub(super) struct PlayerMarker {
    pub id: Option<u64>,
    pub position: Vec3,
    pub label: String,
}
pub(super) fn project_markers(
    frame: &ViewFrame,
    players: &[PlayerMarker],
) -> Vec<(Option<u64>, Vec2, String)> {
    players
        .iter()
        .filter_map(|p| {
            super::view::project(frame, p.position).map(|uv| (p.id, uv, p.label.clone()))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::super::{geometry::OverviewBounds, view::frame};
    use super::*;
    #[test]
    fn snapshots_follow_join_move_rename_leave_and_clip() {
        let f = frame(
            OverviewBounds {
                min: Vec3::splat(-50.),
                max: Vec3::splat(50.),
            },
            Vec3::ZERO,
            false,
            4. / 3.,
            1.,
        );
        let mut players = vec![PlayerMarker {
            id: None,
            position: Vec3::ZERO,
            label: "YOU".into(),
        }];
        assert_eq!(project_markers(&f, &players)[0].2, "YOU");
        players.push(PlayerMarker {
            id: Some(7),
            position: Vec3::ZERO,
            label: "Alex".into(),
        });
        assert_eq!(project_markers(&f, &players).len(), 2);
        players[1].position = Vec3::Y * 8.;
        players[1].label = "Sam".into();
        let result = project_markers(&f, &players);
        assert_eq!(result[1].2, "Sam");
        assert!(result[1].1.y < result[0].1.y);
        players[1].position = Vec3::X * 1000.;
        assert_eq!(project_markers(&f, &players).len(), 1);
        players.pop();
        assert_eq!(project_markers(&f, &players).len(), 1);
        assert!(project_markers(&f, &[]).is_empty());
    }
}
