//! Transport-independent resource-owned native grind rail contracts.
use serde::{Deserialize, Serialize};

pub const STATE_KEY: &str = "native_rails";
pub const MAX_RAILS: usize = 64;
pub const MAX_POINTS: usize = 4096;
pub const MAX_TOTAL_POINTS: usize = 16384;
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rail {
    pub key:String,
    pub points:Vec<[f32;3]>,
    #[serde(default)] pub closed:bool,
}
impl Rail {
    pub fn valid(&self)->bool {
        crate::entities::label(&self.key) && (2..=1024).contains(&self.points.len())
            && self.points.iter().all(|p|crate::entities::vector(*p,99_999.))
            && self.points.windows(2).all(|p|p[0].iter().zip(p[1]).map(|(a,b)|(a-b).powi(2)).sum::<f32>()>0.000001)
    }
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(tag="op",rename_all="snake_case",deny_unknown_fields)]
pub enum Command {
    RailUpsert {key:String, #[serde(default,deserialize_with="crate::entities::input_id")] instance:u64,
        points:Vec<[f32;3]>,#[serde(default)] closed:bool},
    RailRemove {key:String,#[serde(default,deserialize_with="crate::entities::input_id")] instance:u64},
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version:u32,
    /// Decimal to preserve exact revisions through script observations.
    pub revision:String,
    pub rails:Vec<Rail>,
}
impl Snapshot {
    pub fn valid(&self)->bool {
        self.version==1 && self.revision.parse::<u64>().is_ok_and(|n|n>0 && n.to_string()==self.revision)
            && self.rails.len()<=MAX_RAILS && self.rails.iter().all(Rail::valid)
            && self.rails.iter().map(|r|r.points.len()).sum::<usize>()<=MAX_POINTS
            && self.rails.iter().map(|r|&r.key).collect::<std::collections::BTreeSet<_>>().len()==self.rails.len()
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn rails_are_bounded_and_script_ids_are_exact() {
        let command:Command=serde_json::from_value(serde_json::json!({"op":"rail_upsert","key":"practice","instance":"18446744073709551615","points":[[0,1,0],[0,1,10]]})).unwrap();
        assert!(matches!(command,Command::RailUpsert {instance:u64::MAX,..}));
        let mut snapshot=Snapshot {version:1,revision:"1".into(),rails:vec![Rail {key:"practice".into(),points:vec![[0.,1.,0.],[0.,1.,10.]],closed:false}]};
        assert!(snapshot.valid());
        snapshot.rails.push(snapshot.rails[0].clone());assert!(!snapshot.valid());
        snapshot.rails.pop();snapshot.rails[0].points[1]=snapshot.rails[0].points[0];assert!(!snapshot.valid());
    }
}
