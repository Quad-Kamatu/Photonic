//! Native, deliberately bounded CSS-to-vector planning.
//!
//! This module does not depend on MCP.  It turns the portable part of the
//! CSS contract into a deterministic tree of editable box/ellipse paths; the
//! transport layer is responsible only for assigning ids and inserting it.

use crate::{Color, Fill, PathData, Stroke};
use serde::Serialize;
use std::collections::BTreeMap;

pub const CONTRACT_VERSION: u32 = 1;
pub const MAX_CSS_BYTES: usize = 256 * 1024;
pub const MAX_ELEMENTS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct CssViewport {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct CssOrigin {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CssDiagnostic {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub selector: String,
    pub property: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CssVectorPath {
    pub name: String,
    pub path: PathData,
    pub fill: Fill,
    pub stroke: Stroke,
    pub opacity: f32,
}
#[derive(Debug, Clone)]
pub struct CssVectorGroup {
    pub name: String,
    pub children: Vec<CssVectorNode>,
    pub opacity: f32,
    pub provenance: String,
}
#[derive(Debug, Clone)]
pub enum CssVectorNode {
    Group(CssVectorGroup),
    Path(CssVectorPath),
}
#[derive(Debug, Clone)]
pub struct CssVectorPlan {
    pub roots: Vec<CssVectorNode>,
    pub diagnostics: Vec<CssDiagnostic>,
    pub bounds: (f64, f64, f64, f64),
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
struct Rule {
    selector: String,
    declarations: Vec<(String, String)>,
}

/// Compile supported CSS without a browser, DOM, SVG, or external resources.
/// Unsupported declarations are errors in strict mode and recorded omissions in
/// permissive mode.  The parser intentionally rejects constructs it cannot
/// lower faithfully instead of accepting and silently dropping them.
pub fn compile_css_vectors(
    css: &str,
    selector: Option<&str>,
    origin: CssOrigin,
    viewport: CssViewport,
    strict: bool,
) -> Result<CssVectorPlan, Vec<CssDiagnostic>> {
    let mut diagnostics = Vec::new();
    if css.len() > MAX_CSS_BYTES {
        return Err(vec![diag(
            "CSS_LIMIT",
            "CSS input exceeds 256 KiB",
            "",
            None,
            None,
        )]);
    }
    if !viewport.width.is_finite()
        || !viewport.height.is_finite()
        || viewport.width <= 0.0
        || viewport.height <= 0.0
    {
        return Err(vec![diag(
            "CSS_INVALID_VIEWPORT",
            "viewport dimensions must be finite and positive",
            "",
            None,
            None,
        )]);
    }
    let css = strip_comments(css);
    let rules = parse_rules(&css, &mut diagnostics);
    let rules = if rules.is_empty() && !css.trim().is_empty() {
        vec![Rule {
            selector: "CSS Object".into(),
            declarations: parse_declarations(&css, "CSS Object", &mut diagnostics),
        }]
    } else {
        rules
    };
    if rules.is_empty() {
        return Err(vec![diag(
            "CSS_EMPTY",
            "CSS contains no declarations",
            "",
            None,
            None,
        )]);
    }
    let mut elements: BTreeMap<String, Vec<&Rule>> = BTreeMap::new();
    for rule in &rules {
        for sel in rule
            .selector
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !supported_selector(sel) {
                diagnostics.push(diag(
                    "CSS_UNSUPPORTED_SELECTOR",
                    "selector is not supported by the CSS-only object model",
                    sel,
                    None,
                    None,
                ));
                continue;
            }
            elements.entry(sel.to_string()).or_default().push(rule);
        }
    }
    if elements.len() > MAX_ELEMENTS {
        diagnostics.push(diag(
            "CSS_LIMIT",
            "CSS creates more than 512 virtual elements",
            "",
            None,
            None,
        ));
    }
    let selected: Vec<String> = match selector {
        Some(s) if elements.contains_key(s) => vec![s.to_string()],
        Some(s) => {
            diagnostics.push(diag(
                "CSS_UNKNOWN_SELECTOR",
                "selector does not resolve to a virtual element",
                s,
                None,
                None,
            ));
            vec![]
        }
        None => elements
            .keys()
            .filter(|s| !s.contains('>') && !s.contains("::"))
            .cloned()
            .collect(),
    };
    if selected.is_empty() {
        diagnostics.push(diag(
            "CSS_NO_ROOT",
            "no virtual root could be selected",
            "",
            None,
            None,
        ));
    }
    let mut roots = Vec::new();
    for (root_ix, root) in selected.iter().enumerate() {
        let node = build_element(
            root,
            &elements,
            origin,
            viewport,
            &mut diagnostics,
            root_ix == 0,
        )?;
        roots.push(CssVectorNode::Group(node));
    }
    if strict && diagnostics.iter().any(|d| d.severity == "error") {
        return Err(diagnostics);
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    visit_paths(&roots, &mut |p| {
        if let Some(b) = p.path.bounding_box() {
            min_x = min_x.min(b.x0);
            min_y = min_y.min(b.y0);
            max_x = max_x.max(b.x1);
            max_y = max_y.max(b.y1);
        }
    });
    let bounds = if min_x.is_finite() {
        (min_x, min_y, max_x - min_x, max_y - min_y)
    } else {
        (origin.x, origin.y, 0.0, 0.0)
    };
    let fingerprint = stable_fingerprint(&css, selector, origin, viewport);
    Ok(CssVectorPlan {
        roots,
        diagnostics,
        bounds,
        fingerprint,
    })
}

fn build_element(
    selector: &str,
    elements: &BTreeMap<String, Vec<&Rule>>,
    origin: CssOrigin,
    viewport: CssViewport,
    diagnostics: &mut Vec<CssDiagnostic>,
    is_root: bool,
) -> Result<CssVectorGroup, Vec<CssDiagnostic>> {
    let mut props = BTreeMap::new();
    if let Some(rules) = elements.get(selector) {
        for rule in rules {
            for (k, v) in &rule.declarations {
                props.insert(k.clone(), v.clone());
            }
        }
    }
    for (key, value) in &props {
        if !supported_property(key) {
            diagnostics.push(diag(
                "CSS_UNSUPPORTED_PROPERTY",
                &format!("{key} cannot be represented as editable paths"),
                selector,
                Some(key),
                Some(value),
            ));
        }
    }
    let width =
        length(props.get("width").map(String::as_str), viewport.width, 16.0).or_else(|| {
            length(props.get("right").map(String::as_str), viewport.width, 16.0).map(|r| {
                viewport.width
                    - r
                    - length(props.get("left").map(String::as_str), viewport.width, 16.0)
                        .unwrap_or(0.0)
            })
        });
    let height = length(
        props.get("height").map(String::as_str),
        viewport.height,
        16.0,
    )
    .or_else(|| {
        length(
            props.get("bottom").map(String::as_str),
            viewport.height,
            16.0,
        )
        .map(|b| {
            viewport.height
                - b
                - length(props.get("top").map(String::as_str), viewport.height, 16.0).unwrap_or(0.0)
        })
    });
    let (Some(width), Some(height)) = (width, height) else {
        diagnostics.push(diag(
            "CSS_UNRESOLVED_GEOMETRY",
            "width and height are required for a vectorizable box",
            selector,
            None,
            None,
        ));
        return Ok(CssVectorGroup {
            name: selector_name(selector),
            children: vec![],
            opacity: 1.0,
            provenance: selector.into(),
        });
    };
    if !(width.is_finite()
        && height.is_finite()
        && width >= 0.0
        && height >= 0.0
        && width <= 1_000_000.0
        && height <= 1_000_000.0)
    {
        diagnostics.push(diag(
            "CSS_INVALID_GEOMETRY",
            "resolved dimensions are outside supported bounds",
            selector,
            None,
            None,
        ));
    }
    let x = origin.x
        + length(
            props
                .get("left")
                .or_else(|| props.get("inset"))
                .map(String::as_str),
            viewport.width,
            16.0,
        )
        .unwrap_or(0.0);
    let y = origin.y
        + length(
            props
                .get("top")
                .or_else(|| props.get("inset"))
                .map(String::as_str),
            viewport.height,
            16.0,
        )
        .unwrap_or(0.0);
    let opacity = props
        .get("opacity")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let radius = length(
        props.get("border-radius").map(String::as_str),
        width.min(height),
        16.0,
    )
    .unwrap_or(0.0)
    .min(width.min(height) / 2.0);
    let mut children = Vec::new();
    if let Some(fill) = background(&props, selector, diagnostics) {
        let path = if radius >= width.min(height) / 2.0 && (width - height).abs() < 1e-6 {
            PathData::ellipse(x + width / 2.0, y + height / 2.0, width / 2.0, height / 2.0)
        } else {
            PathData::rounded_rect(x, y, width, height, radius)
        };
        children.push(CssVectorNode::Path(CssVectorPath {
            name: format!("{}/background", selector_name(selector)),
            path,
            fill,
            stroke: Stroke::none(),
            opacity,
        }));
    }
    if let Some((color, border_width)) = border(&props, selector, diagnostics) {
        let path = PathData::rounded_rect(x, y, width, height, radius);
        children.push(CssVectorNode::Path(CssVectorPath {
            name: format!("{}/border", selector_name(selector)),
            path,
            fill: Fill::none(),
            stroke: Stroke::solid(color, border_width),
            opacity,
        }));
    }
    // Direct children and pseudo elements are ordered per the contract.
    for suffix in ["::before", ""] {
        for child in elements.keys().filter(|k| {
            k.starts_with(&(selector.to_string() + " > "))
                && if suffix.is_empty() {
                    !k.contains("::")
                } else {
                    k.ends_with(suffix)
                }
        }) {
            children.push(CssVectorNode::Group(build_element(
                child,
                elements,
                origin,
                viewport,
                diagnostics,
                false,
            )?));
        }
    }
    for child in elements
        .keys()
        .filter(|k| k.starts_with(&(selector.to_string() + "::after")))
    {
        children.push(CssVectorNode::Group(build_element(
            child,
            elements,
            origin,
            viewport,
            diagnostics,
            false,
        )?));
    }
    let _ = is_root;
    Ok(CssVectorGroup {
        name: selector_name(selector),
        children,
        opacity,
        provenance: selector.into(),
    })
}

fn supported_selector(s: &str) -> bool {
    !s.is_empty() && !s.contains(['+', '~', '[', ']', '*']) && !s.contains(':')
        || s.ends_with("::before")
        || s.ends_with("::after")
}
fn supported_property(k: &str) -> bool {
    matches!(
        k,
        "width"
            | "height"
            | "left"
            | "top"
            | "right"
            | "bottom"
            | "inset"
            | "background"
            | "background-color"
            | "border"
            | "border-color"
            | "border-width"
            | "border-radius"
            | "opacity"
            | "position"
            | "display"
            | "visibility"
            | "box-sizing"
    )
}
fn selector_name(s: &str) -> String {
    s.trim_matches('.')
        .trim_matches('#')
        .replace("::", "/")
        .replace(" > ", "/")
}
fn background(
    props: &BTreeMap<String, String>,
    sel: &str,
    ds: &mut Vec<CssDiagnostic>,
) -> Option<Fill> {
    let value = props
        .get("background")
        .or_else(|| props.get("background-color"))?;
    match color(value) {
        Some(c) => Some(Fill::solid(c)),
        None => {
            ds.push(diag(
                "CSS_UNSUPPORTED_PAINT",
                "only solid background colors are currently vectorizable",
                sel,
                Some("background"),
                Some(value),
            ));
            None
        }
    }
}
fn border(
    props: &BTreeMap<String, String>,
    sel: &str,
    ds: &mut Vec<CssDiagnostic>,
) -> Option<(Color, f64)> {
    let v = props.get("border")?;
    let mut it = v.split_whitespace();
    let width = length(it.next(), 1.0, 16.0)?;
    let _style = it.next();
    let c = it.next().and_then(color);
    if c.is_none() {
        ds.push(diag(
            "CSS_UNSUPPORTED_BORDER",
            "border requires a supported solid color",
            sel,
            Some("border"),
            Some(v),
        ));
    }
    c.map(|c| (c, width))
}
fn color(v: &str) -> Option<Color> {
    let v = v.trim();
    let expanded = if v.len() == 4 && v.starts_with('#') {
        format!(
            "#{}{}{}{}{}{}",
            &v[1..2],
            &v[1..2],
            &v[2..3],
            &v[2..3],
            &v[3..4],
            &v[3..4]
        )
    } else {
        v.to_string()
    };
    Color::from_hex(&expanded).or_else(|| match v.to_ascii_lowercase().as_str() {
        "white" => Some(Color::WHITE),
        "black" => Some(Color::BLACK),
        "transparent" => Some(Color::new(0.0, 0.0, 0.0, 0.0)),
        "red" => Color::from_hex("#ff0000"),
        "blue" => Color::from_hex("#0000ff"),
        "green" => Color::from_hex("#008000"),
        _ => None,
    })
}
fn length(v: Option<&str>, percent_base: f64, _em: f64) -> Option<f64> {
    let v = v?.trim();
    if v == "0" {
        return Some(0.0);
    }
    if let Some(p) = v.strip_suffix('%') {
        return p
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| n * percent_base / 100.0);
    }
    v.strip_suffix("px").unwrap_or(v).trim().parse::<f64>().ok()
}
fn parse_rules(css: &str, ds: &mut Vec<CssDiagnostic>) -> Vec<Rule> {
    let mut out = Vec::new();
    for part in css.split('}') {
        let Some((sel, body)) = part.split_once('{') else {
            continue;
        };
        if sel.trim().is_empty() {
            continue;
        };
        out.push(Rule {
            selector: sel.trim().into(),
            declarations: parse_declarations(body, sel.trim(), ds),
        });
    }
    out
}
fn parse_declarations(body: &str, sel: &str, ds: &mut Vec<CssDiagnostic>) -> Vec<(String, String)> {
    body.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().to_string();
            if k.is_empty() || v.is_empty() {
                ds.push(diag(
                    "CSS_MALFORMED_DECLARATION",
                    "declaration requires property and value",
                    sel,
                    Some(&k),
                    Some(&v),
                ));
                None
            } else {
                Some((k, v))
            }
        })
        .collect()
}
fn strip_comments(s: &str) -> String {
    let mut o = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        o.push_str(&rest[..i]);
        if let Some(j) = rest[i + 2..].find("*/") {
            rest = &rest[i + 2 + j + 2..]
        } else {
            return o;
        }
    }
    o.push_str(rest);
    o
}
fn diag(
    code: &str,
    message: &str,
    selector: &str,
    property: Option<&str>,
    value: Option<&str>,
) -> CssDiagnostic {
    CssDiagnostic {
        severity: "error".into(),
        code: code.into(),
        message: message.into(),
        selector: selector.into(),
        property: property.map(str::to_string),
        value: value.map(str::to_string),
    }
}
fn visit_paths(nodes: &[CssVectorNode], f: &mut impl FnMut(&CssVectorPath)) {
    for n in nodes {
        match n {
            CssVectorNode::Path(p) => f(p),
            CssVectorNode::Group(g) => visit_paths(&g.children, f),
        }
    }
}
fn stable_fingerprint(
    css: &str,
    selector: Option<&str>,
    origin: CssOrigin,
    viewport: CssViewport,
) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!(
        "{css}\0{:?}\0{:.9},{:.9},{:.9},{:.9}\0{}",
        selector, origin.x, origin.y, viewport.width, viewport.height, CONTRACT_VERSION
    )
    .bytes()
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3)
    }
    format!("cssv1-{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowers_box_and_border_deterministically() {
        let a=compile_css_vectors(".badge { width: 240px; height:96px; background:#6941c6; border:3px solid #fff; border-radius:24px; }",Some(".badge"),CssOrigin{x:100.,y:100.},CssViewport{width:512.,height:512.},true).unwrap();
        assert!((a.bounds.0 - 100.).abs() < 1e-9);
        assert!((a.bounds.1 - 100.).abs() < 1e-9);
        assert!((a.bounds.2 - 240.).abs() < 1e-9);
        assert!((a.bounds.3 - 96.).abs() < 1e-9);
        assert_eq!(a.fingerprint,compile_css_vectors(".badge { width: 240px; height:96px; background:#6941c6; border:3px solid #fff; border-radius:24px; }",Some(".badge"),CssOrigin{x:100.,y:100.},CssViewport{width:512.,height:512.},true).unwrap().fingerprint);
    }
}
