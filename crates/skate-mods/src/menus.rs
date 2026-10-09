//! Menus extension (`sdk.menus`, capability `retail_menus` = 1): edit the game's
//! retail menu structure (pause menu categories and items, Game Settings rows)
//! and register handlers for any entry. Entries are keyed by retail internal
//! names (`GameSettings`, `SkateFeed`, `SinglePlayer`, `ID_GAMESETTINGS_SFXVOLUME`);
//! new entries a mod adds are named `<mod id>.<name>`. Everything a mod did is
//! removed when it is disabled.
use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES_PER_MOD: usize = 64;

/// Retail ids are upper/mixed case (`ID_GAMESETTINGS_*`, `GameSettings`).
pub fn valid_menu_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 96 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub fn valid_entry_kind(s: &str) -> bool {
    matches!(s, "category" | "item" | "setting")
}

fn valid_label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 96 && !s.chars().any(char::is_control)
}

fn valid_path(s: &str) -> bool {
    !s.is_empty() && s.len() <= 160 && !s.contains("..") && !s.starts_with(['/', '\\']) && !s.contains(':')
}

/// Add-or-override options for a category, item or settings row. Fields left
/// out keep the current value (retail's for retail entries).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryOptions {
    /// Display text (a language string id or plain text).
    #[serde(default)]
    pub label: Option<String>,
    /// A retail icon frame label (`map`, `settings`, `skatefeed`, ...).
    #[serde(default)]
    pub icon: Option<String>,
    /// Or an image inside the mod (drawn once the menus render; wins over `icon`).
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
    /// Item: the tab (category id) it goes into.
    #[serde(default)]
    pub category: Option<String>,
    /// Mode keys (Career, CareerPark, FreePlay, FreePlayPark, FreeSkateRestricted, ...);
    /// empty = every mode.
    #[serde(default, deserialize_with = "crate::lua_list::list")]
    pub modes: Vec<String>,
    /// 0-based position in its tab / screen / tab bar.
    #[serde(default)]
    pub position: Option<usize>,
    /// Setting: `option`, `slider` or `selector`.
    #[serde(default)]
    pub widget: Option<String>,
    /// Setting: the settings screen index it goes on.
    #[serde(default)]
    pub screen: Option<usize>,
}

impl EntryOptions {
    pub fn validate(&self) -> bool {
        self.label.as_deref().is_none_or(valid_label)
            && self.help.as_deref().is_none_or(|h| h.len() <= 512 && !h.chars().any(char::is_control))
            && self.icon.as_deref().is_none_or(valid_menu_id)
            && self.image.as_deref().is_none_or(valid_path)
            && self.category.as_deref().is_none_or(valid_menu_id)
            && self.modes.len() <= 16
            && self.modes.iter().all(|m| valid_menu_id(m))
            && self.position.is_none_or(|p| p < 64)
            && self.widget.as_deref().is_none_or(|w| matches!(w, "option" | "slider" | "selector"))
            && self.screen.is_none_or(|s| s < 64)
    }
}

/// A setting value: boolean, integer or number.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MenuValue {
    Bool(bool),
    Int(i64),
    Float(f64),
}

impl MenuValue {
    pub fn validate(&self) -> bool {
        !matches!(self, Self::Float(v) if !v.is_finite())
    }
}

/// A data rule: `true` / `false`, or a table with one of
/// `fact` + `eq|ne|lt|le|gt|ge`, `modes`, `all`, `any`, `not`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RuleSpec {
    Bool(bool),
    Table(Box<RuleTable>),
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTable {
    #[serde(default)]
    pub fact: Option<String>,
    #[serde(default)]
    pub eq: Option<f64>,
    #[serde(default)]
    pub ne: Option<f64>,
    #[serde(default)]
    pub lt: Option<f64>,
    #[serde(default)]
    pub le: Option<f64>,
    #[serde(default)]
    pub gt: Option<f64>,
    #[serde(default)]
    pub ge: Option<f64>,
    #[serde(default, deserialize_with = "crate::lua_list::opt_list")]
    pub modes: Option<Vec<String>>,
    #[serde(default, deserialize_with = "crate::lua_list::opt_list")]
    pub all: Option<Vec<RuleSpec>>,
    #[serde(default, deserialize_with = "crate::lua_list::opt_list")]
    pub any: Option<Vec<RuleSpec>>,
    #[serde(default)]
    pub not: Option<RuleSpec>,
}

impl RuleSpec {
    pub fn validate(&self) -> bool {
        self.valid_at(0)
    }
    fn valid_at(&self, depth: usize) -> bool {
        let Self::Table(t) = self else { return true };
        if depth > 8 {
            return false;
        }
        let comparisons = [t.eq, t.ne, t.lt, t.le, t.gt, t.ge];
        let compared = comparisons.iter().flatten().count();
        let forms = usize::from(t.fact.is_some()) + usize::from(t.modes.is_some()) + usize::from(t.all.is_some())
            + usize::from(t.any.is_some()) + usize::from(t.not.is_some());
        let list_ok = |l: &Option<Vec<RuleSpec>>| l.as_ref().is_none_or(|l| l.len() <= 16 && l.iter().all(|r| r.valid_at(depth + 1)));
        forms == 1
            && (t.fact.is_some() == (compared == 1))
            && (t.fact.is_some() || compared == 0)
            && t.fact.as_deref().is_none_or(valid_menu_id)
            && comparisons.iter().flatten().all(|v| v.is_finite())
            && t.modes.as_ref().is_none_or(|m| m.len() <= 16 && m.iter().all(|m| valid_menu_id(m)))
            && list_ok(&t.all)
            && list_ok(&t.any)
            && t.not.as_ref().is_none_or(|r| r.valid_at(depth + 1))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandleOptions {
    /// Receive `menu_select` (enables the entry). Default true.
    #[serde(default = "yes")]
    pub select: bool,
    /// Receive `menu_highlight`.
    #[serde(default)]
    pub highlight: bool,
    /// Bind a settings row's value (kept by the game; changes arrive as `menu_value`).
    #[serde(default)]
    pub value: Option<MenuValue>,
    /// Override when the entry is enabled / visible (stacked over retail's rule).
    #[serde(default)]
    pub enabled: Option<RuleSpec>,
    #[serde(default)]
    pub visible: Option<RuleSpec>,
    /// Ask this confirmation first when `when` holds (default always).
    #[serde(default)]
    pub confirm: Option<ConfirmSpec>,
    /// Left / Right rule for a settings row (stacked over retail's; reverted on disable).
    #[serde(default)]
    pub rule: Option<ValueRuleSpec>,
}

/// `{kind='slider', min=0, max=1, step=0.1, snap=0.05, bars=10}` (retail's volume rule),
/// `{kind='toggle', labels={'ID_COMMON_OFF','ID_COMMON_ON'}}`, `{kind='cycle', count=3, labels={...}}`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ValueRuleSpec {
    Slider {
        #[serde(default)]
        min: f64,
        #[serde(default = "one")]
        max: f64,
        step: f64,
        #[serde(default)]
        snap: f64,
        #[serde(default = "ten")]
        bars: f64,
    },
    Toggle {
        #[serde(default = "off_on")]
        labels: [String; 2],
    },
    Cycle {
        count: i64,
        #[serde(default, deserialize_with = "crate::lua_list::list")]
        labels: Vec<String>,
    },
}

fn one() -> f64 {
    1.0
}
fn ten() -> f64 {
    10.0
}
fn off_on() -> [String; 2] {
    ["ID_COMMON_OFF".into(), "ID_COMMON_ON".into()]
}

impl ValueRuleSpec {
    pub fn validate(&self) -> bool {
        match self {
            Self::Slider { min, max, step, snap, bars } => {
                [min, max, step, snap, bars].iter().all(|v| v.is_finite() && v.abs() <= 1e6) && min < max && *step > 0.0 && *bars > 0.0
            }
            Self::Toggle { labels } => labels.iter().all(|l| valid_menu_id(l)),
            Self::Cycle { count, labels } => (1..=64).contains(count) && labels.len() <= 64 && labels.iter().all(|l| valid_menu_id(l)),
        }
    }
}

impl Default for HandleOptions {
    fn default() -> Self {
        Self { select: true, highlight: false, value: None, enabled: None, visible: None, confirm: None, rule: None }
    }
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmSpec {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub yes: Option<String>,
    #[serde(default)]
    pub no: Option<String>,
    #[serde(default)]
    pub when: Option<RuleSpec>,
}

impl HandleOptions {
    pub fn validate(&self) -> bool {
        self.value.as_ref().is_none_or(MenuValue::validate)
            && self.rule.as_ref().is_none_or(ValueRuleSpec::validate)
            && self.enabled.as_ref().is_none_or(RuleSpec::validate)
            && self.visible.as_ref().is_none_or(RuleSpec::validate)
            && self.confirm.as_ref().is_none_or(|c| {
                valid_label(&c.title)
                    && c.description.len() <= 512
                    && [&c.yes, &c.no].iter().all(|l| l.as_deref().is_none_or(valid_label))
                    && c.when.as_ref().is_none_or(RuleSpec::validate)
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn value_rule_specs_parse_and_validate() {
        use super::*;
        let h: HandleOptions = serde_json::from_value(serde_json::json!({"value":0.5,"rule":{"kind":"slider","step":0.25}})).unwrap();
        assert_eq!(h.rule, Some(ValueRuleSpec::Slider { min: 0.0, max: 1.0, step: 0.25, snap: 0.0, bars: 10.0 }));
        assert!(h.validate());
        let bad: HandleOptions = serde_json::from_value(serde_json::json!({"rule":{"kind":"cycle","count":0}})).unwrap();
        assert!(!bad.validate());
        assert!(serde_json::from_value::<HandleOptions>(serde_json::json!({"rule":{"kind":"wheel"}})).is_err());
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn options_and_rules_validate() {
        let o: EntryOptions = serde_json::from_value(json!({"label":"Races","icon":"map","category":"SinglePlayer","modes":["Career"],"position":0})).unwrap();
        assert!(o.validate());
        let o: EntryOptions = serde_json::from_value(json!({"image":"../x.png"})).unwrap();
        assert!(!o.validate());
        assert!(serde_json::from_value::<EntryOptions>(json!({"colour":1})).is_err());
        let r: RuleSpec = serde_json::from_value(json!({"all":[{"fact":"sfx_packs","gt":1}, {"not":{"modes":["CareerPark"]}}, true]})).unwrap();
        assert!(r.validate());
        let bad: RuleSpec = serde_json::from_value(json!({"fact":"x","gt":1,"lt":2})).unwrap();
        assert!(!bad.validate(), "one comparison per fact");
        let bad: RuleSpec = serde_json::from_value(json!({"fact":"x","modes":["Career"],"gt":1})).unwrap();
        assert!(!bad.validate(), "one form per table");
        let h: HandleOptions = serde_json::from_value(json!({"value":0.5,"highlight":true,"confirm":{"title":"Sure?"}})).unwrap();
        assert!(h.validate() && h.select);
        assert_eq!(serde_json::from_value::<MenuValue>(json!(3)).unwrap(), MenuValue::Int(3));
        assert_eq!(serde_json::from_value::<MenuValue>(json!(true)).unwrap(), MenuValue::Bool(true));
        assert!(valid_menu_id("ID_GAMESETTINGS_SFXVOLUME") && valid_menu_id("my-mod.Races") && !valid_menu_id("a b"));
    }
}
