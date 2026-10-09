//! Physical edge ownership persists through focus loss and device replacement.
#[derive(Default)]
pub struct LocationInput {
    previous: bool,
    active: bool,
    device: Option<u64>,
    initialized: bool,
    release: bool,
    consumed: bool,
}
impl LocationInput {
    pub fn advance(&mut self, down: bool, active: bool, device: Option<u64>) -> bool {
        if !self.initialized || self.active != active || self.device != device {
            self.release = down;
        }
        self.initialized = true;
        self.active = active;
        self.device = device;
        let edge = active && down && !self.previous && !self.release;
        if !down {
            self.release = false;
            self.consumed = false;
        }
        self.previous = down;
        edge
    }
    pub fn consume(&mut self) {
        self.consumed = true;
    }
    pub fn blocked(&self) -> bool {
        self.consumed || self.release
    }
}
#[derive(Default)]
pub struct FloorMenu {
    count: usize,
    pub selected: usize,
}
impl FloorMenu {
    pub fn open(&mut self, count: usize) -> Option<usize> {
        self.cancel();
        if count == 1 {
            Some(0)
        } else {
            self.count = count.min(8);
            None
        }
    }
    pub fn active(&self) -> bool {
        self.count > 1
    }
    pub fn navigate(&mut self, delta: isize) {
        if self.active() {
            self.selected =
                (self.selected as isize + delta).rem_euclid(self.count as isize) as usize;
        }
    }
    pub fn choose(&mut self) -> Option<usize> {
        let out = self.active().then_some(self.selected);
        self.cancel();
        out
    }
    pub fn cancel(&mut self) {
        self.count = 0;
        self.selected = 0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_a_focus_and_reconnect_require_new_edge() {
        let mut input = LocationInput::default();
        assert!(!input.advance(true, true, Some(1)));
        assert!(!input.advance(false, true, Some(1)));
        assert!(input.advance(true, true, Some(1)));
        assert!(!input.advance(true, false, Some(1)));
        assert!(!input.advance(true, true, Some(1)));
        assert!(!input.advance(true, true, Some(2)));
        assert!(!input.advance(false, true, Some(2)));
        assert!(input.advance(true, true, Some(2)));
    }
    #[test]
    fn interaction_consumes_skating_action_until_release() {
        let mut input = LocationInput::default();
        input.advance(false, true, None);
        assert!(input.advance(true, true, None));
        input.consume();
        assert!(input.blocked());
        input.advance(true, true, None);
        assert!(input.blocked());
        input.advance(false, true, None);
        assert!(!input.blocked());
    }
    #[test]
    fn focus_and_device_release_guard_blocks_skating() {
        let mut input = LocationInput::default();
        input.advance(false, true, Some(1));
        input.advance(true, false, Some(1));
        assert!(!input.advance(true, true, Some(1)));
        assert!(input.blocked());
        input.advance(false, true, Some(1));
        assert!(!input.blocked());
        assert!(!input.advance(true, true, Some(2)));
        assert!(input.blocked());
        input.advance(false, true, Some(2));
        assert!(!input.blocked());
    }
    #[test]
    fn single_floor_direct_multi_floor_select_cancel() {
        let mut menu = FloorMenu::default();
        assert_eq!(menu.open(1), Some(0));
        assert!(!menu.active());
        assert_eq!(menu.open(2), None);
        assert!(menu.active());
        menu.navigate(1);
        assert_eq!(menu.choose(), Some(1));
        assert!(!menu.active());
        menu.open(2);
        menu.cancel();
        assert_eq!(menu.choose(), None);
    }
}
