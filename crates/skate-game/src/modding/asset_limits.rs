//! Local import policy: remote manifests cannot raise client allocation limits.
use serde::{Deserialize, Serialize};
use std::path::Path;

const MIB:u64=1024*1024;
#[derive(Clone,Copy,Debug,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub(crate) struct Limits {
    pub content:skate_resources::Limits,
    pub model_file_bytes:u64,
    pub model_json_bytes:u64,
    pub geometry_bytes:u64,
    pub texture_bytes:u64,
    pub image_dimension:u32,
    pub set_decoded_bytes:u64,
}
impl Default for Limits {
    fn default()->Self {Self {
        content:Default::default(),model_file_bytes:16*MIB,model_json_bytes:256*1024,
        geometry_bytes:16*MIB,texture_bytes:16*MIB,image_dimension:2048,set_decoded_bytes:256*MIB,
    }}
}
impl Limits {
    pub fn validate(&self)->Result<(),String> {
        self.content.validate().map_err(|e|e.to_string())?;
        if self.model_file_bytes==0 || self.model_file_bytes>128*MIB || self.model_file_bytes>self.content.max_file_bytes
            || self.model_json_bytes==0 || self.model_json_bytes>2*MIB
            || self.geometry_bytes==0 || self.geometry_bytes>256*MIB
            || self.texture_bytes==0 || self.texture_bytes>256*MIB
            || self.image_dimension==0 || self.image_dimension>8192
            || self.set_decoded_bytes==0 || self.set_decoded_bytes>2*1024*MIB {
            return Err("asset-limits.json exceeds hard import bounds or model_file_bytes exceeds content.max_file_bytes".into());
        }
        Ok(())
    }
    pub fn read(root:&Path)->Result<Self,String> {
        use std::io::Read;
        let path=root.join("asset-limits.json");
        let metadata=match std::fs::symlink_metadata(&path) {
            Ok(value)=>value,Err(error) if error.kind()==std::io::ErrorKind::NotFound=>return Ok(Self::default()),
            Err(error)=>return Err(format!("Local asset limits: {error}")),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len()>16384 {
            return Err("asset-limits.json must be a regular local file of at most 16 KiB".into());
        }
        let mut bytes=Vec::new();std::fs::File::open(path).map_err(|e|e.to_string())?.take(16385)
            .read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>16384 {return Err("asset-limits.json exceeds 16 KiB".into());}
        let value:serde_json::Value=serde_json::from_slice(&bytes).map_err(|e|format!("Local asset limits: {e}"))?;
        if !value.is_object() {return Err("asset-limits.json must contain an object".into());}
        let limits:Self=serde_json::from_value(value).map_err(|e|format!("Local asset limits: {e}"))?;
        limits.validate()?;Ok(limits)
    }
    pub fn charge(&self,used:&mut u64,decoded:u64)->Result<(),String> {
        let total=used.checked_add(decoded).filter(|&n|n<=self.set_decoded_bytes)
            .ok_or("Resource set exceeds cumulative decoded model/texture budget")?;
        *used=total;Ok(())
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn local_asset_limits_bound_expansion_and_combined_imports() {
        let defaults=Limits::default();defaults.validate().unwrap();
        let mut limits=defaults;limits.model_file_bytes=128*MIB;
        assert!(limits.validate().is_err(),"content quota must agree with larger model files");
        limits.content.max_file_bytes=128*MIB;limits.geometry_bytes=128*MIB;limits.texture_bytes=128*MIB;limits.image_dimension=8192;
        limits.validate().unwrap();
        limits.image_dimension=8193;assert!(limits.validate().is_err());
        limits=defaults;limits.model_json_bytes=2*MIB+1;assert!(limits.validate().is_err());
        limits=defaults;limits.set_decoded_bytes=2*1024*MIB+1;assert!(limits.validate().is_err());
        let mut used=defaults.set_decoded_bytes-1;
        assert!(defaults.charge(&mut used,2).is_err());
        assert_eq!(used,defaults.set_decoded_bytes-1,"failed allocation must leave budget unchanged");
    }
    #[test] fn local_asset_policy_is_explicit_bounded_and_not_a_download() {
        let root=std::env::temp_dir().join(format!("skate-asset-limits-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("asset-limits.json"),br#"{"image_dimension":4096,"texture_bytes":67108864}"#).unwrap();
        assert_eq!(Limits::read(&root).unwrap().image_dimension,4096);
        std::fs::write(root.join("asset-limits.json"),b"[]").unwrap();
        assert!(Limits::read(&root).is_err());
        std::fs::write(root.join("asset-limits.json"),b"{\"unknown\":1}").unwrap();
        assert!(Limits::read(&root).is_err());
        std::fs::write(root.join("asset-limits.json"),vec![b' ';16385]).unwrap();
        assert!(Limits::read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
