//! Bounded local JSX/Tailwind adapter for the CSS vector compiler.
//!
//! React itself is intentionally not executed.  This adapter accepts only a
//! static JSX fragment of intrinsic elements and a small, documented Tailwind
//! utility subset, then delegates all geometry lowering to the core compiler.

use crate::handlers::css_vectors::create_vectors_from_css;
use crate::protocol::{CreateVectorsFromCssArgs, CreateVectorsFromReactArgs, ToolResult};
use crate::server::AppState;

const MAX_JSX_BYTES: usize = 256 * 1024;
const MAX_ELEMENTS: usize = 512;

pub async fn create_vectors_from_react(
    state: &AppState,
    args: CreateVectorsFromReactArgs,
) -> ToolResult {
    let css = match jsx_to_css(&args.jsx) {
        Ok(css) => css,
        Err(errors) => {
            return ToolResult::error("React component conversion rejected")
                .with_data(serde_json::json!({"diagnostics": errors, "contract_version": 1}))
        }
    };
    let result = create_vectors_from_css(
        state,
        CreateVectorsFromCssArgs {
            css,
            selector: Some(".component".into()),
            origin: args.origin,
            viewport: args.viewport,
            layer_id: args.layer_id,
            group_name: args.group_name,
            strict: args.strict,
            dry_run: args.dry_run,
        },
    )
    .await;
    result
}

fn jsx_to_css(jsx: &str) -> Result<String, Vec<serde_json::Value>> {
    if jsx.len() > MAX_JSX_BYTES {
        return Err(vec![diag("JSX_LIMIT", "JSX input exceeds 256 KiB", "")]);
    }
    // Dynamic expressions are intentionally rejected, rather than evaluated
    // or silently omitted.  That also prohibits imports, hooks, and arbitrary
    // component execution at this boundary.
    if jsx.contains('{') || jsx.contains('}') || jsx.contains("import ") || jsx.contains("export ")
    {
        return Err(vec![diag(
            "JSX_DYNAMIC",
            "only a static JSX fragment is supported",
            "",
        )]);
    }
    let bytes = jsx.as_bytes();
    let mut i = 0;
    // Source tag name and generated selector are both retained: closing tags
    // validate against the former while children extend the latter.
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut rules = Vec::new();
    let mut elements = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let next = jsx[i..]
                .find('<')
                .map(|offset| i + offset)
                .unwrap_or(bytes.len());
            if !jsx[i..next].trim().is_empty() {
                return Err(vec![diag(
                    "JSX_TEXT_UNSUPPORTED",
                    "text children are not yet vectorizable; use editable text nodes separately",
                    jsx[i..next].trim(),
                )]);
            }
            i = next;
            continue;
        }
        let Some(end_rel) = jsx[i..].find('>') else {
            return Err(vec![diag("JSX_MALFORMED", "unterminated JSX tag", "")]);
        };
        let end = i + end_rel;
        let token = jsx[i + 1..end].trim();
        i = end + 1;
        if token.starts_with('!') {
            continue;
        }
        if let Some(close) = token.strip_prefix('/') {
            let close = close.trim();
            match stack.pop() {
                Some((tag, _)) if tag == close => continue,
                _ => {
                    return Err(vec![diag(
                        "JSX_MALFORMED",
                        "closing tag does not match an open tag",
                        close,
                    )])
                }
            }
        }
        let self_closing = token.ends_with('/');
        let token = token.trim_end_matches('/').trim();
        let tag_end = token.find(char::is_whitespace).unwrap_or(token.len());
        let tag = &token[..tag_end];
        if !matches!(
            tag,
            "div"
                | "section"
                | "main"
                | "article"
                | "header"
                | "footer"
                | "button"
                | "span"
                | "p"
                | "h1"
                | "h2"
                | "h3"
        ) {
            return Err(vec![diag(
                "JSX_UNSUPPORTED_ELEMENT",
                "only intrinsic layout elements are supported",
                tag,
            )]);
        }
        elements += 1;
        if elements > MAX_ELEMENTS {
            return Err(vec![diag(
                "JSX_LIMIT",
                "component contains more than 512 elements",
                tag,
            )]);
        }
        if stack.len() >= 32 {
            return Err(vec![diag(
                "JSX_LIMIT",
                "component nesting exceeds 32 levels",
                tag,
            )]);
        }
        let index = elements;
        let selector = if let Some((_, parent)) = stack.last() {
            format!("{parent} > .node-{index}")
        } else {
            if index != 1 {
                return Err(vec![diag(
                    "JSX_MULTIPLE_ROOTS",
                    "JSX fragment must have exactly one root element",
                    tag,
                )]);
            }
            ".component".into()
        };
        let classes = attr(token, "className")
            .or_else(|| attr(token, "class"))
            .unwrap_or_default();
        let declarations = tailwind(&classes, index == 1)?;
        rules.push(format!("{selector} {{ {declarations} }}"));
        if !self_closing {
            stack.push((tag.to_string(), selector));
        }
    }
    if !stack.is_empty() {
        return Err(vec![diag("JSX_MALFORMED", "unclosed JSX tag", "")]);
    }
    if rules.is_empty() {
        return Err(vec![diag("JSX_EMPTY", "JSX contains no elements", "")]);
    }
    Ok(rules.join("\n"))
}

fn attr(token: &str, name: &str) -> Option<String> {
    let start = token.find(name)? + name.len();
    let value = token[start..].trim_start().strip_prefix('=')?.trim_start();
    let quote = value.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    value[1..]
        .find(quote)
        .map(|end| value[1..1 + end].to_string())
}

fn tailwind(classes: &str, root: bool) -> Result<String, Vec<serde_json::Value>> {
    let mut out = if root {
        vec!["width:100%".into(), "height:100%".into()]
    } else {
        vec!["width:100%".into(), "height:40px".into()]
    };
    for class in classes.split_whitespace() {
        let mapped = match class {
            "w-full" => Some("width:100%".into()),
            "h-full" => Some("height:100%".into()),
            "rounded" => Some("border-radius:4px".into()),
            "rounded-md" => Some("border-radius:6px".into()),
            "rounded-lg" => Some("border-radius:8px".into()),
            "rounded-xl" => Some("border-radius:12px".into()),
            "rounded-full" => Some("border-radius:9999px".into()),
            "border" => Some("border:1px solid #000000".into()),
            "border-2" => Some("border:2px solid #000000".into()),
            "opacity-50" => Some("opacity:0.5".into()),
            "opacity-75" => Some("opacity:0.75".into()),
            "opacity-100" => Some("opacity:1".into()),
            "bg-white" => Some("background:#ffffff".into()),
            "bg-black" => Some("background:#000000".into()),
            "bg-slate-900" => Some("background:#0f172a".into()),
            "bg-slate-100" => Some("background:#f1f5f9".into()),
            "bg-blue-500" => Some("background:#3b82f6".into()),
            "bg-indigo-600" => Some("background:#4f46e5".into()),
            "bg-emerald-500" => Some("background:#10b981".into()),
            "bg-red-500" => Some("background:#ef4444".into()),
            _ => arbitrary_size(class),
        };
        match mapped {
            Some(value) => out.push(value),
            None => {
                return Err(vec![diag(
                    "TAILWIND_UNSUPPORTED",
                    "utility is outside the bounded Tailwind subset",
                    class,
                )])
            }
        }
    }
    Ok(out.join(";"))
}

fn arbitrary_size(class: &str) -> Option<String> {
    let (property, value) = class
        .strip_prefix("w-[")
        .map(|v| ("width", v))
        .or_else(|| class.strip_prefix("h-[").map(|v| ("height", v)))?;
    let value = value.strip_suffix(']')?;
    if value.ends_with("px")
        && value[..value.len() - 2]
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1_000_000.0)
            .is_some()
    {
        Some(format!("{property}:{value}"))
    } else {
        None
    }
}

fn diag(code: &str, message: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"severity":"error", "code":code, "message":message, "value":value})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_tailwind_component_becomes_css_tree() {
        let css = jsx_to_css(r#"<div className="w-[320px] h-[160px] bg-slate-900 rounded-xl"><button className="w-[120px] h-[40px] bg-blue-500 rounded" /></div>"#).unwrap();
        assert!(css.contains(".component"));
        assert!(css.contains(".component > .node-2"));
        assert!(css.contains("background:#3b82f6"));
    }
    #[test]
    fn dynamic_jsx_is_rejected_not_ignored() {
        assert!(jsx_to_css("<div>{label}</div>").is_err());
    }
    #[test]
    fn text_is_rejected_not_silently_dropped() {
        assert!(jsx_to_css("<div>Save</div>").is_err());
    }
}
