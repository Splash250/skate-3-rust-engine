//! Local photographic UI requests. There is deliberately no scripted shutter.
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Mode { enabled: bool },
    Gallery,
    Thumbnail { id: String },
}

impl Operation {
    pub fn validate(&self) -> bool {
        match self {
            Self::Thumbnail { id } => {
                !id.is_empty()
                    && id.len() <= 40
                    && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripts_cannot_request_capture_or_supply_paths() {
        for json in [
            r#"{"op":"shutter"}"#,
            r#"{"op":"mode","enabled":true,"path":"/tmp/a.png"}"#,
        ] {
            assert!(serde_json::from_str::<Operation>(json).is_err());
        }
        assert!(
            !Operation::Thumbnail {
                id: "../../Desktop/private.png".into()
            }
            .validate()
        );
        assert!(Operation::Thumbnail { id: "1-2".into() }.validate());
    }
}
