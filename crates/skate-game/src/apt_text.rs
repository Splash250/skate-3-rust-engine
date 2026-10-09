//! SkateAptString82CA14D0 and cFont828076D0/82808708 layout.
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
pub struct Glyph {
    pub glyph_index: usize,
    pub width: f32,
    pub height: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub x_advance: f32,
    pub atlas_bounds: [f32; 4],
}
#[derive(Clone)]
pub struct Font {
    /// Secondary native font drawn over this one (825D6B68 attaches it, 82CA1FD8 draws both).
    pub foreground: Option<Box<Font>>,
    pub texture: String,
    pub size: [u32; 2],
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub ascent: f32,
    pub glyphs: BTreeMap<u32, Glyph>,
}
#[derive(Clone, Default)]
pub struct TextAssets {
    pub fonts: BTreeMap<i32, Font>,
    pub language: BTreeMap<String, String>,
}
/// Retail's one secondary-font rule, 825D6B68: an APT font named "Futura Shadow" (8220BD44, stricmp
/// 82AE89B0) gets the native font "futuraheavy" (8220BD54) from the global font table (82809208), whether
/// or not the movie itself uses that font. Data can replace it (`font_pairs`: APT name -> native file name).
pub const RETAIL_FONT_PAIRS: &[(&str, &str)] = &[("Futura Shadow", "futuraheavy")];

/// The native file a font named `apt_name` pairs with (`font_pairs` in the data, else retail's table).
pub fn paired_file(json: &serde_json::Value, apt_name: &str) -> Option<String> {
    match json.get("font_pairs").and_then(|p| p.as_object()) {
        Some(pairs) => pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(apt_name))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_owned),
        None => RETAIL_FONT_PAIRS
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(apt_name))
            .map(|(_, v)| (*v).to_owned()),
    }
}

/// One font from the movie's font data, by its APT name (FontManager 82808AE8 row + bitmap bank).
fn native_font(json: &serde_json::Value, name: &str) -> Result<Font, String> {
    let asset = &json["fonts"][name];
    let d = &asset["definition"];
    let glyphs: Vec<Glyph> =
        serde_json::from_value(d["glyphs"].clone()).map_err(|e| e.to_string())?;
    let layout = &json["font_mappings"][name]["native_layout"];
    let f = |key: &str| {
        layout[key]
            .as_f64()
            .map(|v| v as f32)
            .ok_or_else(|| format!("Missing native font {name}.{key}"))
    };
    let mut font = Font {
        foreground: None,
        texture: asset["texture"]
            .as_str()
            .ok_or("Font texture missing")?
            .into(),
        size: [
            d["textures"][0]["width"]
                .as_u64()
                .ok_or("Font width missing")? as u32,
            d["textures"][0]["height"]
                .as_u64()
                .ok_or("Font height missing")? as u32,
        ],
        scale: [f("ScaleX")?, f("ScaleY")?],
        offset: [f("OffsetX")?, f("OffsetY")?],
        ascent: d["metrics"]["Ascent"]
            .as_f64()
            .ok_or("Font ascent missing")? as f32,
        glyphs: BTreeMap::new(),
    };
    for mapping in d["characters"]
        .as_array()
        .ok_or("Font character map missing")?
    {
        let index = mapping["glyph_index"]
            .as_u64()
            .ok_or("Invalid font glyph index")? as usize;
        let glyph = glyphs
            .iter()
            .find(|g| g.glyph_index == index)
            .ok_or("Absent mapped glyph")?;
        font.glyphs.insert(
            mapping["codepoint"]
                .as_u64()
                .ok_or("Invalid font codepoint")? as u32,
            glyph.clone(),
        );
    }
    Ok(font)
}

/// The secondary font for `apt_name`, looked up in the whole native font table by file name.
/// NOT RETAIL: when the paired font is absent (a mod's data, an old export) retail's lookup falls back to
/// the "debug" row (82809208 loop 2); we have no debug bank, so the text draws its primary pass only.
fn foreground(json: &serde_json::Value, apt_name: &str) -> Result<Option<Box<Font>>, String> {
    let Some(file) = paired_file(json, apt_name) else {
        return Ok(None);
    };
    let Some(name) = json["font_mappings"].as_object().and_then(|m| {
        m.iter()
            .find(|(_, row)| {
                row["file_name"]
                    .as_str()
                    .is_some_and(|f| f.eq_ignore_ascii_case(&file))
            })
            .map(|(name, _)| name.clone())
    }) else {
        return Ok(None);
    };
    if json["fonts"][&name].is_null() {
        return Ok(None);
    }
    Ok(Some(Box::new(native_font(json, &name)?)))
}

impl TextAssets {
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        let mut out = Self::default();
        out.language =
            serde_json::from_value(json["language"].clone()).map_err(|e| e.to_string())?;
        for c in json["characters"]
            .as_array()
            .ok_or("HUD characters missing")?
        {
            if c["type_name"] != "font" {
                continue;
            }
            let name = c["font"]["name"].as_str().ok_or("HUD font name missing")?;
            let mut font = native_font(json, name)?;
            font.foreground = foreground(json, name)?;
            out.fonts.insert(
                c["id"].as_i64().ok_or("Invalid font character id")? as i32,
                font,
            );
        }
        Ok(out)
    }
    pub fn localize(&self, text: &str) -> String {
        if let Some(literal) = text.strip_prefix('#') {
            return literal.into();
        }
        self.language
            .get(text)
            .cloned()
            .unwrap_or_else(|| text.into())
    }
}
impl Font {
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs
            .get(&(c as u32))
            .or_else(|| self.glyphs.get(&65535))
    }
    pub fn width(&self, text: &str, height: f32) -> f32 {
        text.chars()
            .filter_map(|c| self.glyph(c))
            .fold(0.0, |x, g| g.x_advance.mul_add(self.scale[0] * height, x))
    }
}

#[cfg(test)]
mod font_pair_tests {
    use super::*;
    use serde_json::json;

    fn asset(texture: &str) -> serde_json::Value {
        json!({"texture": texture, "definition": {
            "glyphs": [{"glyph_index": 0, "width": 1.0, "height": 1.0, "x_offset": 0.0, "y_offset": 0.0,
                        "x_advance": 1.0, "atlas_bounds": [0.0, 0.0, 1.0, 1.0]}],
            "characters": [{"glyph_index": 0, "codepoint": 65}],
            "textures": [{"width": 8, "height": 8}], "metrics": {"Ascent": 1.0}}})
    }
    fn row(file: &str) -> serde_json::Value {
        json!({"file_name": file, "native_layout": {"ScaleX": 1.0, "ScaleY": 1.0, "OffsetX": 0.0, "OffsetY": 0.0}})
    }
    /// A movie that places only "Futura Shadow" (small_popup's case), with the native table holding both.
    fn movie() -> serde_json::Value {
        json!({"language": {}, "characters": [{"id": 3, "type_name": "font", "font": {"name": "Futura Shadow"}}],
               "fonts": {"Futura Shadow": asset("shadow.rgba"), "Futura Std Medium": asset("heavy.rgba")},
               "font_mappings": {"Futura Shadow": row("futurashadow"), "Futura Std Medium": row("futuraheavy")}})
    }

    #[test]
    fn shadow_gets_futuraheavy_from_the_native_table_without_a_character() {
        let t = TextAssets::load(&movie()).unwrap();
        assert_eq!(t.fonts.len(), 1);
        assert_eq!(t.fonts[&3].foreground.as_ref().unwrap().texture, "heavy.rgba");
    }

    #[test]
    fn missing_partner_draws_primary_only() {
        let mut m = movie();
        m["fonts"].as_object_mut().unwrap().remove("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
        m["font_mappings"].as_object_mut().unwrap().remove("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }

    #[test]
    fn data_pairs_replace_the_retail_table_case_insensitively() {
        let mut m = movie();
        m["font_pairs"] = json!({"futura shadow": "FUTURAHEAVY"});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_some());
        m["font_pairs"] = json!({});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
        m["font_pairs"] = json!({"Futura Shadow": "no_such_font"});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }

    #[test]
    fn other_fonts_get_no_partner() {
        let mut m = movie();
        m["characters"][0]["font"]["name"] = json!("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }
}

/// 825E51A0 localizes each authored component before composing a literal.
pub(crate) fn localize_trick(label: &str, assets: Option<&crate::apt_text::TextAssets>) -> String {
    if let Some(literal) = label.strip_prefix('#') {
        return literal.to_owned();
    }
    label
        .split_whitespace()
        .map(|part| {
            let text = assets
                .map(|a| a.localize(part))
                .unwrap_or_else(|| part.to_owned());
            if text.starts_with("ID_") {
                humanize_trick_id(&text)
            } else {
                text
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize_trick_id(id: &str) -> String {
    let rest = id
        .strip_prefix("ID_TRICK_")
        .or_else(|| id.strip_prefix("ID_"))
        .unwrap_or(id);
    rest.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
