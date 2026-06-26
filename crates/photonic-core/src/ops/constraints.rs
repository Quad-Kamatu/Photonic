//! Live property-constraint evaluation.
//!
//! A [`PropertyConstraint`](crate::document::PropertyConstraint) binds a node
//! property to an arithmetic expression over other nodes' properties, e.g.
//! `nodes['logo'].x + 20`. [`evaluate_constraints`] resolves all constraints in
//! dependency order (forward evaluation, not a solver), detecting cycles, and
//! writes each result back onto its target node.
//!
//! Grammar (minimal, no scripting):
//! - numbers (`12`, `3.5`, `-2`), `+ - * /`, parentheses;
//! - property references `nodes['<id-or-name>'].<prop>` where `<prop>` is one of
//!   `x`, `y`, `width`, `height`, `opacity`, `font_size`.
//!
//! Settable target properties are `x`, `y`, `opacity`, `font_size`. `width` and
//! `height` may be *referenced* but not used as a constraint target (setting
//! them would require non-uniform geometry scaling — tracked as follow-up).

use crate::document::Document;
use crate::node::{NodeId, SceneNodeKind};
use std::collections::{HashMap, HashSet};

/// Errors raised while evaluating the constraint set.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintError {
    /// A dependency cycle among the listed target node ids.
    Cycle(Vec<NodeId>),
    /// An expression failed to parse.
    Parse { constraint: NodeId, message: String },
    /// A target property is not settable (e.g. `width`/`height`).
    UnsupportedTarget { constraint: NodeId, property: String },
}

impl std::fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintError::Cycle(ids) => write!(f, "constraint cycle among {ids:?}"),
            ConstraintError::Parse { constraint, message } => {
                write!(f, "constraint on {constraint}: parse error: {message}")
            }
            ConstraintError::UnsupportedTarget { constraint, property } => write!(
                f,
                "constraint on {constraint}: property '{property}' is not a settable target"
            ),
        }
    }
}

impl std::error::Error for ConstraintError {}

/// Properties that may be used as a constraint *target* (written to).
const SETTABLE: [&str; 4] = ["x", "y", "opacity", "font_size"];

/// Read a node property's current numeric value.
pub fn get_property(doc: &Document, node_id: NodeId, prop: &str) -> Option<f64> {
    let node = doc.nodes.get(&node_id)?;
    match prop {
        "opacity" => Some(node.opacity as f64),
        "font_size" => match &node.kind {
            SceneNodeKind::Text(t) => Some(t.font_size),
            _ => None,
        },
        "x" | "y" | "width" | "height" => {
            let local = node.local_bounds()?;
            // World-space AABB: transform the four corners.
            let pts = [
                (local.x0, local.y0),
                (local.x1, local.y0),
                (local.x1, local.y1),
                (local.x0, local.y1),
            ];
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for (lx, ly) in pts {
                let (wx, wy) = node.transform.apply(lx, ly);
                min_x = min_x.min(wx);
                min_y = min_y.min(wy);
                max_x = max_x.max(wx);
                max_y = max_y.max(wy);
            }
            Some(match prop {
                "x" => min_x,
                "y" => min_y,
                "width" => max_x - min_x,
                _ => max_y - min_y, // height
            })
        }
        _ => None,
    }
}

/// Write a settable node property to `value`.
fn set_property(doc: &mut Document, node_id: NodeId, prop: &str, value: f64) {
    // For x/y we need the current value before mutating.
    let current = get_property(doc, node_id, prop);
    let Some(node) = doc.nodes.get_mut(&node_id) else {
        return;
    };
    match prop {
        "opacity" => node.opacity = (value as f32).clamp(0.0, 1.0),
        "font_size" => {
            if let SceneNodeKind::Text(t) = &mut node.kind {
                t.font_size = value.max(0.0);
            }
        }
        "x" => {
            if let Some(cur) = current {
                node.transform.matrix[4] += value - cur;
            }
        }
        "y" => {
            if let Some(cur) = current {
                node.transform.matrix[5] += value - cur;
            }
        }
        _ => {}
    }
}

/// A parsed property reference `nodes['key'].prop`.
struct Ref {
    /// Byte range of the whole `nodes['..'].prop` match in the source string.
    span: (usize, usize),
    key: String,
    prop: String,
}

/// Scan an expression for `nodes['key'].prop` references.
fn scan_refs(expr: &str) -> Vec<Ref> {
    let bytes = expr.as_bytes();
    let mut refs = Vec::new();
    let mut i = 0;
    while let Some(rel) = expr[i..].find("nodes") {
        let start = i + rel;
        let mut j = start + "nodes".len();
        let eat_ws = |j: &mut usize| {
            while *j < bytes.len() && bytes[*j].is_ascii_whitespace() {
                *j += 1;
            }
        };
        eat_ws(&mut j);
        if j >= bytes.len() || bytes[j] != b'[' {
            i = start + "nodes".len();
            continue;
        }
        j += 1;
        eat_ws(&mut j);
        if j >= bytes.len() || (bytes[j] != b'\'' && bytes[j] != b'"') {
            i = start + "nodes".len();
            continue;
        }
        let quote = bytes[j];
        j += 1;
        let key_start = j;
        while j < bytes.len() && bytes[j] != quote {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let key = expr[key_start..j].to_string();
        j += 1; // closing quote
        eat_ws(&mut j);
        if j >= bytes.len() || bytes[j] != b']' {
            i = start + "nodes".len();
            continue;
        }
        j += 1;
        eat_ws(&mut j);
        if j >= bytes.len() || bytes[j] != b'.' {
            i = start + "nodes".len();
            continue;
        }
        j += 1;
        let prop_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        let prop = expr[prop_start..j].to_string();
        refs.push(Ref {
            span: (start, j),
            key,
            prop,
        });
        i = j;
    }
    refs
}

/// Resolve a node key (UUID string or node name) to a NodeId.
fn resolve_key(doc: &Document, key: &str) -> Option<NodeId> {
    if let Ok(id) = uuid::Uuid::parse_str(key) {
        if doc.nodes.contains_key(&id) {
            return Some(id);
        }
    }
    doc.find_node_by_name(key).map(|n| n.id)
}

/// Substitute property references with their current numeric values, then
/// evaluate the resulting arithmetic expression.
fn eval_expression(doc: &Document, expr: &str) -> Result<f64, String> {
    let refs = scan_refs(expr);
    // Rebuild the string with references replaced by literal values.
    let mut out = String::new();
    let mut last = 0;
    for r in &refs {
        out.push_str(&expr[last..r.span.0]);
        let node_id = resolve_key(doc, &r.key).ok_or_else(|| format!("unknown node '{}'", r.key))?;
        let val = get_property(doc, node_id, &r.prop)
            .ok_or_else(|| format!("unknown property '{}'", r.prop))?;
        out.push_str(&format!("({val})"));
        last = r.span.1;
    }
    out.push_str(&expr[last..]);
    eval_arithmetic(&out)
}

// ── Minimal arithmetic evaluator (recursive descent) ──────────────────────────

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'+') => {
                    self.i += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.i += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'*') => {
                    self.i += 1;
                    v *= self.factor()?;
                }
                Some(b'/') => {
                    self.i += 1;
                    let d = self.factor()?;
                    if d == 0.0 {
                        return Err("division by zero".into());
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn factor(&mut self) -> Result<f64, String> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                self.ws();
                if self.s.get(self.i) != Some(&b')') {
                    return Err("expected ')'".into());
                }
                self.i += 1;
                Ok(v)
            }
            Some(b'-') => {
                self.i += 1;
                Ok(-self.factor()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.factor()
            }
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.')
        {
            self.i += 1;
        }
        if self.i == start {
            return Err(format!("unexpected token at byte {}", self.i));
        }
        std::str::from_utf8(&self.s[start..self.i])
            .unwrap()
            .parse::<f64>()
            .map_err(|e| e.to_string())
    }
}

fn eval_arithmetic(expr: &str) -> Result<f64, String> {
    let mut p = Parser {
        s: expr.as_bytes(),
        i: 0,
    };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing input at byte {}", p.i));
    }
    Ok(v)
}

/// Evaluate every constraint in dependency order and apply the results.
///
/// Returns `Err` on a dependency cycle, an unsettable target, or a parse error;
/// in those cases already-applied results are left in place and the document
/// remains usable.
pub fn evaluate_constraints(doc: &mut Document) -> Result<(), ConstraintError> {
    if doc.constraints.is_empty() {
        return Ok(());
    }

    // Snapshot constraint identity → (target node, property, refs).
    struct C {
        idx: usize,
        target: NodeId,
        prop: String,
        deps: HashSet<(NodeId, String)>,
    }
    let mut cs: Vec<C> = Vec::with_capacity(doc.constraints.len());
    for (idx, con) in doc.constraints.iter().enumerate() {
        if !SETTABLE.contains(&con.target_property.as_str()) {
            return Err(ConstraintError::UnsupportedTarget {
                constraint: con.target_node_id,
                property: con.target_property.clone(),
            });
        }
        let mut deps = HashSet::new();
        for r in scan_refs(&con.expression) {
            if let Some(id) = resolve_key(doc, &r.key) {
                deps.insert((id, r.prop));
            }
        }
        cs.push(C {
            idx,
            target: con.target_node_id,
            prop: con.target_property.clone(),
            deps,
        });
    }

    // Topological order over constraints: an edge B→A means A depends on B's
    // output (A references B's target property), so B must evaluate first.
    let targets: HashMap<(NodeId, String), usize> = cs
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.target, c.prop.clone()), i))
        .collect();

    let n = cs.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for (a, c) in cs.iter().enumerate() {
        for dep in &c.deps {
            if let Some(&b) = targets.get(dep) {
                if b != a {
                    adj[b].push(a);
                    indeg[a] += 1;
                }
            }
        }
    }

    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    let mut head = 0;
    while head < queue.len() {
        let u = queue[head];
        head += 1;
        order.push(u);
        for &v in &adj[u] {
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push(v);
            }
        }
    }

    if order.len() != n {
        // Cycle: the nodes still with in-degree > 0 participate.
        let cyclic: Vec<NodeId> = (0..n)
            .filter(|&i| indeg[i] > 0)
            .map(|i| cs[i].target)
            .collect();
        return Err(ConstraintError::Cycle(cyclic));
    }

    for &ci in &order {
        let c = &cs[ci];
        let expr = doc.constraints[c.idx].expression.clone();
        match eval_expression(doc, &expr) {
            Ok(value) => set_property(doc, c.target, &c.prop, value),
            Err(message) => {
                return Err(ConstraintError::Parse {
                    constraint: c.target,
                    message,
                })
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::PropertyConstraint;
    use crate::node::{PathNode, SceneNode};
    use crate::path::PathData;

    fn rect_node(doc: &Document, name: &str, x: f64) -> SceneNode {
        let mut n = SceneNode::new(
            name,
            doc.active_layer_id.unwrap(),
            SceneNodeKind::Path(PathNode::new(PathData::rect(0.0, 0.0, 10.0, 10.0))),
        );
        n.transform = crate::transform::Transform::translate(x, 0.0);
        n
    }

    #[test]
    fn arithmetic_evaluator() {
        assert_eq!(eval_arithmetic("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(eval_arithmetic("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(eval_arithmetic("-4 / 2").unwrap(), -2.0);
        assert!(eval_arithmetic("1 / 0").is_err());
        assert!(eval_arithmetic("1 +").is_err());
    }

    #[test]
    fn scans_property_references() {
        let refs = scan_refs("nodes['a'].x * 2 + nodes[\"b\"].width");
        assert_eq!(refs.len(), 2);
        assert_eq!((refs[0].key.as_str(), refs[0].prop.as_str()), ("a", "x"));
        assert_eq!((refs[1].key.as_str(), refs[1].prop.as_str()), ("b", "width"));
    }

    #[test]
    fn constraint_propagates_position() {
        let mut doc = Document::new("t", 100.0, 100.0);
        let a = doc.add_node(rect_node(&doc, "a", 10.0), None);
        let b = doc.add_node(rect_node(&doc, "b", 0.0), None);
        doc.constraints
            .push(PropertyConstraint::new(b, "x", "nodes['a'].x + 20"));

        evaluate_constraints(&mut doc).unwrap();
        assert!((get_property(&doc, b, "x").unwrap() - 30.0).abs() < 1e-6);

        // Move a; re-evaluate; b follows.
        doc.nodes.get_mut(&a).unwrap().transform.matrix[4] = 50.0;
        evaluate_constraints(&mut doc).unwrap();
        assert!((get_property(&doc, b, "x").unwrap() - 70.0).abs() < 1e-6);
    }

    #[test]
    fn chained_constraints_evaluate_in_order() {
        let mut doc = Document::new("t", 100.0, 100.0);
        let a = doc.add_node(rect_node(&doc, "a", 5.0), None);
        let b = doc.add_node(rect_node(&doc, "b", 0.0), None);
        let c = doc.add_node(rect_node(&doc, "c", 0.0), None);
        // c depends on b depends on a — defined out of order to exercise topo sort.
        doc.constraints
            .push(PropertyConstraint::new(c, "x", "nodes['b'].x + 1"));
        doc.constraints
            .push(PropertyConstraint::new(b, "x", "nodes['a'].x + 1"));
        let _ = a;
        evaluate_constraints(&mut doc).unwrap();
        assert!((get_property(&doc, b, "x").unwrap() - 6.0).abs() < 1e-6);
        assert!((get_property(&doc, c, "x").unwrap() - 7.0).abs() < 1e-6);
    }

    #[test]
    fn cycle_is_detected() {
        let mut doc = Document::new("t", 100.0, 100.0);
        let a = doc.add_node(rect_node(&doc, "a", 0.0), None);
        let b = doc.add_node(rect_node(&doc, "b", 0.0), None);
        doc.constraints
            .push(PropertyConstraint::new(a, "x", "nodes['b'].x + 1"));
        doc.constraints
            .push(PropertyConstraint::new(b, "x", "nodes['a'].x + 1"));
        assert!(matches!(
            evaluate_constraints(&mut doc),
            Err(ConstraintError::Cycle(_))
        ));
    }

    #[test]
    fn unsupported_target_rejected() {
        let mut doc = Document::new("t", 100.0, 100.0);
        let a = doc.add_node(rect_node(&doc, "a", 0.0), None);
        doc.constraints
            .push(PropertyConstraint::new(a, "width", "100"));
        assert!(matches!(
            evaluate_constraints(&mut doc),
            Err(ConstraintError::UnsupportedTarget { .. })
        ));
    }
}
