//! Pure input policy for the built-in dedicated-server shove action.
#[derive(Default)]
pub(crate) struct ShoveInput {
    held: bool,
    next: u64,
}
impl ShoveInput {
    pub fn sample(
        &mut self,
        held: bool,
        origin: [f32; 3],
        forward: [f32; 3],
        targets: impl IntoIterator<Item = (u64, [f32; 3])>,
    ) -> Option<(u64, u64)> {
        let pressed = held && !self.held;
        self.held = held;
        if !pressed {
            return None;
        }
        let forward_len = (forward[0] * forward[0] + forward[2] * forward[2]).sqrt();
        if !forward_len.is_finite() || forward_len <= f32::EPSILON {
            return None;
        }
        let target = targets
            .into_iter()
            .filter_map(|(id, position)| {
                let delta: [f32; 3] = std::array::from_fn(|i| position[i] - origin[i]);
                let distance = delta.iter().map(|v| v * v).sum::<f32>();
                let horizontal = (delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
                let facing =
                    (delta[0] * forward[0] + delta[2] * forward[2]) / (horizontal * forward_len);
                (distance.is_finite()
                    && distance <= 2.5 * 2.5
                    && delta[1].abs() <= 1.5
                    && facing >= 0.25)
                    .then_some((id, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)))?;
        self.next = self.next.checked_add(1)?;
        Some((self.next, target.0))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn press_selects_nearest_visible_target_and_never_repeats_while_held() {
        let mut input = ShoveInput::default();
        let targets = [(7, [0., 0., 1.8]), (8, [0., 0., 1.]), (9, [0., 0., -0.5])];
        assert_eq!(
            input.sample(true, [0.; 3], [0., 0., 1.], targets),
            Some((1, 8))
        );
        assert_eq!(input.sample(true, [0.; 3], [0., 0., 1.], targets), None);
        assert_eq!(input.sample(false, [0.; 3], [0., 0., 1.], targets), None);
        assert_eq!(
            input.sample(true, [0.; 3], [0., 0., 1.], targets),
            Some((2, 8))
        );
    }
    #[test]
    fn empty_press_does_not_hit_someone_who_enters_range_while_held() {
        let mut input = ShoveInput::default();
        assert_eq!(input.sample(true, [0.; 3], [0., 0., 1.], []), None);
        assert_eq!(
            input.sample(true, [0.; 3], [0., 0., 1.], [(7, [0., 0., 1.])]),
            None
        );
    }
    #[test]
    fn rejects_far_vertical_rear_and_nonfinite_targets() {
        let mut input = ShoveInput::default();
        assert_eq!(
            input.sample(
                true,
                [0.; 3],
                [0., 0., 1.],
                [
                    (1, [0., 0., 4.]),
                    (2, [0., 5., 1.]),
                    (3, [0., 0., -1.]),
                    (4, [f32::NAN, 0., 1.])
                ]
            ),
            None
        );
    }
}

pub(crate) fn trick_label(value: &str) -> String {
    let mut label = String::new();
    for ch in value.chars().filter(|ch| !ch.is_control()) {
        if label.len() + ch.len_utf8() > 128 {
            break;
        }
        label.push(ch);
    }
    label
}
#[cfg(test)]
mod label_tests {
    #[test]
    fn unicode_tricks_fit_protocol_byte_limit_and_strip_controls() {
        let label = super::trick_label(&"跳".repeat(96));
        assert!(label.len() <= 128);
        assert_eq!(super::trick_label("Kick\nflip\t"), "Kickflip");
    }
}
