//! Graph scoring publication shared with headless data audits.
use skate_core::animation::output::attributes::AttributeName;

/// Meaningful graph outputs are retained even though a scoring UI is outside
/// this game.82595D08 sets a name and ORs a bit in the graph's score packet.
#[derive(Default)]
pub struct ScorePacket {
    pub handplant: Option<(
        skate_core::animation::output::attributes::AttributeName,
        [f32; 2],
    )>,
    /// ScoringGrabs 82BBEF60: selected authored name and tweak vector.
    pub grab: Option<(
        skate_core::animation::output::attributes::AttributeName,
        [f32; 2],
    )>,
    pub trick_names: Names,
    pub name: Option<u32>,
    pub flags: u32,
}
impl ScorePacket {
    pub fn set(&mut self, value: u32) {
        let bit = match value {
            0 => 31,
            1 => 30,
            2 => 29,
            3 => 28,
            4 => 27,
            5 => 26,
            6 => 23,
            7 => 21,
            8 => 22,
            9 => 20,
            _ => return,
        };
        self.flags |= 1 << bit;
        if value != 2 && value != 3 {
            self.name = Some(value);
        }
    }
}

/// MotionGraph full-object5924 and5948, retained alongside score flags5972.
/// Native24-byte slots contain five encoded words plus initialized padding;
/// host representation preserves the names without copying native padding.
#[derive(Default, Debug)]
pub struct Names {
    pub first: Option<AttributeName>,
    pub second: Option<AttributeName>,
}
