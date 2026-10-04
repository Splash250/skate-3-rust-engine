//! Server-owned grind definitions, scoped by instance and resource generation.
use std::collections::BTreeMap;
use skate_net::{dedicated::Server,rails::{Command,Rail,Snapshot},resources::{Kind,Message,Scope}};
#[derive(Default)]
pub struct Rails {
    generations:BTreeMap<String,u64>,
    sets:BTreeMap<(String,u64),Snapshot>,
    revision:u64,
}
impl Rails {
    pub fn instances(&self) -> std::collections::BTreeSet<u64> {
        self.sets.keys().map(|(_, instance)| *instance).collect()
    }
    pub fn sync_resources(&mut self,generations:BTreeMap<String,u64>) {
        self.sets.retain(|(resource,_),_|self.generations.get(resource)==generations.get(resource));
        self.generations=generations;
    }
    pub fn command(&mut self,resource:&str,generation:u64,request:serde_json::Value,server:&mut Server)->Result<(),String> {
        if self.generations.get(resource)!=Some(&generation) || generation==0 {return Err("Native rail owner generation is not active".into());}
        if serde_json::to_vec(&request).map_err(|e|e.to_string())?.len()>65536 {return Err("Native rail command exceeds 64 KiB".into());}
        let command:Command=serde_json::from_value(request).map_err(|e|format!("Invalid native rail command: {e}"))?;
        let instance=match &command {Command::RailUpsert {instance,..}|Command::RailRemove {instance,..}=>*instance};
        let owner=(resource.to_owned(),instance);
        let revision=self.revision.checked_add(1).ok_or("Native rail revisions exhausted")?;
        let mut snapshot=self.sets.get(&owner).cloned().unwrap_or(Snapshot {version:1,revision:revision.to_string(),rails:Vec::new()});
        snapshot.revision=revision.to_string();
        match command {
            Command::RailUpsert {key,points,closed,..}=>{
                let rail=Rail {key,points,closed};if !rail.valid() {return Err("Invalid native rail geometry".into());}
                if let Some(existing)=snapshot.rails.iter_mut().find(|r|r.key==rail.key) {*existing=rail;} else {snapshot.rails.push(rail);}
            }
            Command::RailRemove {key,..}=>{
                let Some(index)=snapshot.rails.iter().position(|r|r.key==key) else {return Err("Unknown owned native rail".into());};
                snapshot.rails.remove(index);
            }
        }
        let others=self.sets.iter().filter(|(key,_)|*key!=&owner);
        let resource_rails=others.clone().filter(|((r,_),_)|r==resource).map(|(_,s)|s.rails.len()).sum::<usize>()+snapshot.rails.len();
        let resource_points=others.clone().filter(|((r,_),_)|r==resource).flat_map(|(_,s)|&s.rails).map(|r|r.points.len()).sum::<usize>()+snapshot.rails.iter().map(|r|r.points.len()).sum::<usize>();
        let total_points=others.flat_map(|(_,s)|&s.rails).map(|r|r.points.len()).sum::<usize>()+snapshot.rails.iter().map(|r|r.points.len()).sum::<usize>();
        if !snapshot.valid() || resource_rails>skate_net::rails::MAX_RAILS || resource_points>skate_net::rails::MAX_POINTS || total_points>skate_net::rails::MAX_TOTAL_POINTS {
            return Err("Native rail owner/global budget exhausted".into());
        }
        server.send_resource(None,Message {scope:Scope::Instance{id:instance},id:1,resource:resource.into(),generation,
            kind:Kind::State,name:skate_net::rails::STATE_KEY.into(),value:if snapshot.rails.is_empty() {serde_json::Value::Null} else {serde_json::to_value(&snapshot).map_err(|e|e.to_string())?}})?;
        if snapshot.rails.is_empty() {self.sets.remove(&owner);} else {self.sets.insert(owner,snapshot);}
        self.revision=revision;Ok(())
    }
    pub fn republish(&self,server:&mut Server)->Result<(),String> {
        for ((resource,instance),snapshot) in &self.sets {
            server.send_resource(None,Message {scope:Scope::Instance{id:*instance},id:1,resource:resource.clone(),
                generation:self.generations[resource],kind:Kind::State,name:skate_net::rails::STATE_KEY.into(),
                value:serde_json::to_value(snapshot).map_err(|e|e.to_string())?})?;
        }
        Ok(())
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn native_rail_crud_enforces_generation_and_retires_restart_ownership() {
        let mut server=Server::new(skate_net::dedicated::Config{session:1,server_id:99,map:1,max_players:16}).unwrap();
        let generations=BTreeMap::from([("park".into(),1)]);
        server.configure_resources("a".repeat(64),31030,generations.clone()).unwrap();
        let mut rails=Rails::default();rails.sync_resources(generations);
        let command=serde_json::json!({"op":"rail_upsert","key":"rail","instance":"7","points":[[0,1,0],[0,1,8]]});
        rails.command("park",1,command.clone(),&mut server).unwrap();
        assert_eq!(rails.sets[&("park".into(),7)].rails.len(),1);
        assert!(rails.command("park",2,command.clone(),&mut server).is_err());
        assert!(rails.command("other",1,command,&mut server).is_err());
        rails.command("park",1,serde_json::json!({"op":"rail_remove","key":"rail","instance":"7"}),&mut server).unwrap();
        assert!(!rails.sets.contains_key(&("park".into(),7)));
        rails.sync_resources(BTreeMap::from([("park".into(),2)]));assert!(rails.sets.is_empty());
    }
}
