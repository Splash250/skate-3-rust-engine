//! Generation-tagged readiness and support-retirement decisions.
use skate_resources::locations::{EntryIntent, LocationStatus};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub owner: String,
    pub owner_generation: u64,
    pub map_generation: u64,
    pub request: String,
}
#[derive(Clone, Debug)]
pub struct Visit {
    pub ticket: Ticket,
    pub location: String,
    pub floor: String,
    pub interior: String,
    pub return_position: [f32; 3],
    pub return_heading: f32,
}
#[derive(Default)]
pub struct Runtime {
    pub visit: Option<Visit>,
    pub phase: Phase,
    pub error: Option<String>,
    deadline: f64,
    retired: bool,
    ready: bool,
}
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Exterior,
    Preparing,
    AwaitingApproval,
    Interior,
    Returning,
}
impl Runtime {
    pub fn begin(&mut self, visit: Visit, now: f64) -> Result<(), String> {
        if self.phase != Phase::Exterior || !now.is_finite() {
            return Err("location travel already active or invalid clock".into());
        }
        EntryIntent {
            location: visit.location.clone(),
            floor: visit.floor.clone(),
            generation: visit.ticket.owner_generation.to_string(),
            request: visit.ticket.request.clone(),
        }
        .validate()?;
        self.visit = Some(visit);
        self.phase = Phase::Preparing;
        self.deadline = now + 10.;
        self.retired = false;
        self.ready = false;
        self.error = None;
        Ok(())
    }
    pub fn ready(
        &mut self,
        ticket: &Ticket,
        now: f64,
        scene: bool,
        collision: bool,
        support: bool,
        dedicated: bool,
    ) -> bool {
        if self.phase != Phase::Preparing || !self.current(ticket) {
            return false;
        }
        if !now.is_finite() || now >= self.deadline {
            self.fail(
                "Interior preparation timed out after 10 seconds; retry at the entrance".into(),
            );
            return false;
        }
        self.ready = scene && collision && support;
        if self.ready && dedicated {
            self.phase = Phase::AwaitingApproval;
        }
        self.ready
    }
    /// Host calls this after native travel commits, following approval if dedicated.
    pub fn committed(&mut self, ticket: &Ticket) -> bool {
        if !self.ready
            || !self.current(ticket)
            || !matches!(self.phase, Phase::Preparing | Phase::AwaitingApproval)
        {
            return false;
        }
        self.phase = Phase::Interior;
        self.error = None;
        true
    }
    pub fn timeout(&mut self, now: f64) {
        if now < self.deadline {
            return;
        }
        if self.phase == Phase::Preparing && !self.ready {
            self.fail("Interior preparation timed out; retry at the entrance".into());
        } else if self.phase == Phase::AwaitingApproval {
            // A timeout is not an acknowledged cancellation. Keep the visit and
            // its support alive until the host rejects or commits the request.
            self.error = Some("Waiting for the server to resolve interior travel".into());
        }
    }

    pub fn current(&self, ticket: &Ticket) -> bool {
        self.visit.as_ref().is_some_and(|v| &v.ticket == ticket) && !self.retired
    }
    pub fn fail(&mut self, error: String) {
        if matches!(self.phase, Phase::Preparing | Phase::AwaitingApproval) {
            self.phase = Phase::Exterior;
            self.visit = None;
            self.ready = false;
        }
        self.error = Some(error);
    }
    pub fn return_requested(&mut self) -> bool {
        if self.phase == Phase::Interior {
            self.phase = Phase::Returning;
            true
        } else {
            false
        }
    }
    pub fn returned(&mut self, ticket: &Ticket) -> bool {
        if self.phase == Phase::Returning
            && self.visit.as_ref().is_some_and(|v| &v.ticket == ticket)
        {
            self.phase = Phase::Exterior;
            self.visit = None;
            self.ready = false;
            true
        } else {
            false
        }
    }
    /// True only when no occupant still needs the supporting collision.
    pub fn retire(&mut self, owner: &str, generation: u64) -> bool {
        if !self
            .visit
            .as_ref()
            .is_some_and(|v| v.ticket.owner == owner && v.ticket.owner_generation == generation)
        {
            return true;
        }
        self.retired = true;
        match self.phase {
            Phase::Interior | Phase::Returning => {
                self.phase = Phase::Returning;
                false
            }
            _ => {
                self.phase = Phase::Exterior;
                self.visit = None;
                self.ready = false;
                true
            }
        }
    }
    pub fn status(&self) -> LocationStatus {
        match self.phase {
            Phase::Exterior => LocationStatus::Exterior,
            Phase::Preparing => LocationStatus::Preparing,
            Phase::AwaitingApproval => LocationStatus::AwaitingApproval,
            Phase::Interior => LocationStatus::Interior,
            Phase::Returning => LocationStatus::Returning,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn visit() -> Visit {
        Visit {
            ticket: Ticket {
                owner: "engine".into(),
                owner_generation: 1,
                map_generation: 2,
                request: "1".into(),
            },
            location: "entry".into(),
            floor: "one".into(),
            interior: "room".into(),
            return_position: [3., 0., 0.],
            return_heading: 0.,
        }
    }
    #[test]
    fn travel_waits_for_scene_collision_and_support() {
        let mut r = Runtime::default();
        let v = visit();
        r.begin(v.clone(), 0.).unwrap();
        assert!(!r.committed(&v.ticket));
        for ready in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(!r.ready(&v.ticket, 1., ready.0, ready.1, ready.2, false));
        }
        assert!(r.ready(&v.ticket, 1., true, true, true, true));
        assert_eq!(r.phase, Phase::AwaitingApproval);
        assert!(r.committed(&v.ticket));
        assert_eq!(r.phase, Phase::Interior);
    }
    #[test]
    fn delayed_authority_preserves_visit_until_reset_commits() {
        let mut r = Runtime::default();
        let v = visit();
        r.begin(v.clone(), 0.).unwrap();
        assert!(r.ready(&v.ticket, 1., true, true, true, true));
        r.timeout(11.);
        assert!(r.current(&v.ticket));
        assert_eq!(r.phase, Phase::AwaitingApproval);
        assert!(r.committed(&v.ticket));
        assert!(r.return_requested());
        assert!(r.returned(&v.ticket));
    }
    #[test]
    fn ten_second_timeout_keeps_exterior_pose() {
        let mut r = Runtime::default();
        let v = visit();
        r.begin(v.clone(), 5.).unwrap();
        assert!(!r.ready(&v.ticket, 14.999, false, true, true, false));
        r.timeout(15.);
        assert_eq!(r.phase, Phase::Exterior);
        assert!(r.visit.is_none());
    }
    #[test]
    fn retired_generation_discards_load_completion() {
        let mut r = Runtime::default();
        let v = visit();
        r.begin(v.clone(), 0.).unwrap();
        assert!(r.retire("engine", 1));
        assert!(!r.ready(&v.ticket, 1., true, true, true, false));
        assert!(!r.committed(&v.ticket));
    }
    #[test]
    fn occupied_retirement_returns_before_collision_release() {
        let mut r = Runtime::default();
        let v = visit();
        r.begin(v.clone(), 0.).unwrap();
        assert!(r.ready(&v.ticket, 1., true, true, true, false));
        assert!(r.committed(&v.ticket));
        assert!(!r.retire("engine", 1));
        assert_eq!(r.phase, Phase::Returning);
        assert!(r.returned(&v.ticket));
        assert!(r.retire("engine", 1));
    }
}
