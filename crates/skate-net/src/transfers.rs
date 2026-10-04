//! Bounded host-side request keys and deadlines for the large-message lane.
//! No script identity or completion state is accepted from a received payload.
use crate::{bulk::Progress,resources::LargeTicket};
use serde_json::{Value,json};
use std::{collections::BTreeMap,time::{Duration,Instant}};
struct Pending {ticket:LargeTicket,deadline:Instant,last:Progress}
#[derive(Default)]
pub struct Transfers {pending:BTreeMap<(String,u64,String),Pending>}
pub struct Event {pub resource:String,pub generation:u64,pub value:Value}
fn event(owner:&(String,u64,String),state:&str,progress:Progress,error:Option<String>)->Event {
    let (acknowledged,total)=match progress {
        Progress::Pending{acknowledged,total}=>(acknowledged,total),
        Progress::Delivered{total}=>(total,total),Progress::Cancelled=>(0,0),
    };
    Event{resource:owner.0.clone(),generation:owner.1,value:json!({"key":owner.2,"state":state,
        "acknowledged_bytes":acknowledged,"total_bytes":total,"error":error})}
}
impl Transfers {
    pub fn available(&self,resource:&str,generation:u64,key:&str,timeout_ms:u64)->Result<(),String> {
        if generation==0 || resource.is_empty() || resource.len()>64 || key.is_empty() || key.len()>64
            || !key.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-.:".contains(&b))
            || !(1..=120000).contains(&timeout_ms) {return Err("Invalid transfer key or timeout (1..120000 ms)".into());}
        if self.pending.contains_key(&(resource.into(),generation,key.into())) {return Err("Transfer key already pending".into());}
        if self.pending.len()>=128 || self.pending.keys().filter(|(r,_,_)|r==resource).count()>=16 {
            return Err("Transfer request budget exhausted (16 per resource,128 per host)".into());
        }
        Ok(())
    }
    pub fn insert(&mut self,resource:&str,generation:u64,key:&str,timeout_ms:u64,ticket:LargeTicket,progress:Progress,now:Instant)->Result<Event,String> {
        self.available(resource,generation,key,timeout_ms)?;
        let owner=(resource.into(),generation,key.into());
        let result=event(&owner,"queued",progress,None);
        self.pending.insert(owner,Pending{ticket,deadline:now+Duration::from_millis(timeout_ms),last:progress});
        Ok(result)
    }
    pub fn failed(resource:&str,generation:u64,key:&str,error:String)->Event {
        event(&(resource.into(),generation,key.into()),"failed",Progress::Cancelled,Some(error))
    }
    /// The operation callback queries transport progress or requests cancellation.
    /// A timeout retires the request but the bounded wire cancellation tombstone
    /// remains until acknowledged, preventing transfer sequence gaps.
    pub fn poll(&mut self,now:Instant,mut live:impl FnMut(&str,u64)->bool,
        mut operation:impl FnMut(LargeTicket,bool)->Result<Progress,String>)->Vec<Event> {
        let mut events=Vec::new();
        self.pending.retain(|owner,p| {
            if !live(&owner.0,owner.1) {let _=operation(p.ticket,true);return false;}
            match operation(p.ticket,false) {
                Err(error)=>{events.push(event(owner,"failed",p.last,Some(error)));false}
                Ok(Progress::Delivered{total})=>{events.push(event(owner,"delivered",Progress::Delivered{total},None));false}
                Ok(Progress::Cancelled)=>{events.push(event(owner,"cancelled",p.last,None));false}
                Ok(progress)=>{
                    if now>=p.deadline {
                        let _=operation(p.ticket,true);
                        events.push(event(owner,"timed_out",progress,Some("Transfer deadline expired".into())));false
                    } else {
                        if progress!=p.last {events.push(event(owner,"sending",progress,None));p.last=progress;}
                        true
                    }
                }
            }
        });
        events
    }
    pub fn cancel(&mut self,resource:&str,generation:u64,key:&str,
        mut operation:impl FnMut(LargeTicket,bool)->Result<Progress,String>)->Event {
        let owner=(resource.into(),generation,key.into());
        let Some(p)=self.pending.remove(&owner) else {return Self::failed(resource,generation,key,"Unknown transfer key".into());};
        match operation(p.ticket,false) {
            Ok(progress@Progress::Delivered{..})=>event(&owner,"delivered",progress,None),
            _=>match operation(p.ticket,true) {
                Ok(_)=>event(&owner,"cancelled",p.last,None),
                Err(error)=>event(&owner,"failed",p.last,Some(error)),
            }
        }
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn deadlines_ownership_and_request_keys_are_bounded() {
        let now=Instant::now();let ticket=LargeTicket{recipient:2,epoch:3,id:1};
        let progress=Progress::Pending{acknowledged:192,total:4000};let mut book=Transfers::default();
        book.insert("app",1,"large",100,ticket,progress,now).unwrap();
        assert!(book.available("app",1,"large",100).is_err());
        assert!(book.poll(now,|_,_|true,|_,_|Ok(progress)).is_empty());
        let mut cancelled=false;
        let events=book.poll(now+Duration::from_millis(100),|_,_|true,|handle,cancel|{assert_eq!(handle,ticket);cancelled|=cancel;Ok(progress)});
        assert!(cancelled);assert_eq!(events.len(),1);assert_eq!(events[0].value["state"],"timed_out");
        assert_eq!(events[0].value["acknowledged_bytes"],192);assert!(book.available("app",1,"large",100).is_ok());
        book.insert("app",1,"large",100,ticket,progress,now).unwrap();
        let mut cancelled=false;assert!(book.poll(now,|_,_|false,|_,cancel|{cancelled|=cancel;Ok(progress)}).is_empty());assert!(cancelled);
        for i in 0..16 {book.insert("app",2,&i.to_string(),100,ticket,progress,now).unwrap();}
        assert!(book.available("app",2,"overflow",100).is_err());
    }
}
