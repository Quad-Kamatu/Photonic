//! Design-token parsing — the shared engine behind the MCP `import_design_tokens`
//! tool and the GUI "Import design tokens…" action (issue #207).
//!
//! Extracts named color tokens (name → `#hex`) from CSS custom properties, flat
//! or nested JSON, and Style Dictionary (`{ "value": "#hex" }`) payloads.

/// Parse a design-tokens payload into `(name, hex)` pairs.
///
/// `hint` is `"css"`, `"json"`, or (default) `"auto"` — which picks JSON when the
/// text starts with `{`/`[`, else CSS. Non-color entries are ignored.
pub fn parse_token_colors(text: &str, hint: Option<&str>) -> Vec<(String, String)> {
    let hint = hint.unwrap_or("auto").to_ascii_lowercase();
    let looks_json = {
        let t = text.trim_start();
        t.starts_with('{') || t.starts_with('[')
    };
    let use_json = hint == "json" || (hint == "auto" && looks_json);

    let mut out = Vec::new();
    if use_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            collect_json(&v, String::new(), &mut out);
        }
    } else {
        parse_css(text, &mut out);
    }
    out
}

/// Extract `--name: #hex;` custom properties from CSS text.
fn parse_css(css: &str, out: &mut Vec<(String, String)>) {
    for raw in css.split(';') {
        let mut line = raw.trim();
        // Drop any leading selector/block opener (e.g. ":root {") so its colon
        // isn't mistaken for the property separator.
        if let Some(brace) = line.rfind('{') {
            line = line[brace + 1..].trim();
        }
        let Some(idx) = line.find(':') else { continue };
        let (key, val) = line.split_at(idx);
        let key = key.trim();
        let val = val[1..].trim().trim_end_matches('}').trim();
        if let Some(name) = key.strip_prefix("--") {
            if let Some(hex) = extract_hex(val) {
                out.push((name.to_string(), hex));
            }
        }
    }
}

/// Recursively walk a JSON tokens tree, registering any hex-color string value
/// under a `/`-joined key path. Handles flat maps, `export_design_tokens` output,
/// and Style Dictionary `{ "value": "#hex" }` leaves.
fn collect_json(v: &serde_json::Value, path: String, out: &mut Vec<(String, String)>) {
    match v {
        serde_json::Value::String(s) => {
            if let Some(hex) = extract_hex(s) {
                if !path.is_empty() {
                    out.push((path, hex));
                }
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(s)) = map.get("value") {
                if let Some(hex) = extract_hex(s) {
                    if !path.is_empty() {
                        out.push((path.clone(), hex));
                        return;
                    }
                }
            }
            for (k, child) in map {
                // Skip non-color scaffolding keys from export_design_tokens.
                if matches!(k.as_str(), "font_families" | "font_sizes" | "stroke_widths") {
                    continue;
                }
                let next = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}/{k}")
                };
                collect_json(child, next, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, child) in arr.iter().enumerate() {
                let next = if path.is_empty() {
                    i.to_string()
                } else {
                    format!("{path}/{i}")
                };
                collect_json(child, next, out);
            }
        }
        _ => {}
    }
}

/// Pull a leading `#rgb`/`#rrggbb`/`#rrggbbaa` hex color out of a CSS value.
pub fn extract_hex(val: &str) -> Option<String> {
    let v = val.trim();
    if !v.starts_with('#') {
        return None;
    }
    let hexpart: String = v[1..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if matches!(hexpart.len(), 3 | 4 | 6 | 8) {
        Some(format!("#{hexpart}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_css_custom_properties() {
        let css =
            ":root {\n  --brand-primary: #2f56cf;\n  --brand-secondary: #7c3aed;\n  --pad: 8px;\n}";
        let toks = parse_token_colors(css, None);
        assert_eq!(toks.len(), 2);
        assert!(toks.contains(&("brand-primary".to_string(), "#2f56cf".to_string())));
        assert!(toks.contains(&("brand-secondary".to_string(), "#7c3aed".to_string())));
    }

    #[test]
    fn parses_flat_and_style_dictionary_json() {
        let flat = r##"{ "primary": "#2f56cf", "accent": "#7c3aed", "size": 12 }"##;
        let toks = parse_token_colors(flat, None);
        assert_eq!(toks.len(), 2);

        let sd = r##"{ "color": { "primary": { "value": "#2f56cf" } } }"##;
        let toks = parse_token_colors(sd, None);
        assert_eq!(
            toks,
            vec![("color/primary".to_string(), "#2f56cf".to_string())]
        );
    }

    #[test]
    fn ignores_export_scaffolding_keys() {
        let json = r##"{ "colors": { "color-1": "#ffffff" }, "font_sizes": [12, 16] }"##;
        let toks = parse_token_colors(json, None);
        assert_eq!(
            toks,
            vec![("colors/color-1".to_string(), "#ffffff".to_string())]
        );
    }
}
