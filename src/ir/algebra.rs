//! Algebraic-structure detection from Alloy facts.
//!
//! Scans the lowered IR for a binary operation on a sig plus the *proven* laws
//! (associativity / identity / inverse) that turn it into a Semigroup / Monoid /
//! Group. Because Alloy has machine-checked these facts, an emitted konpu
//! annotation asserts a *verified* structure — strictly stronger than konpu's
//! shape-only `--infer`. False positives are worse than misses, so we only emit
//! when the law shapes match exactly and the identity/inverse elements exist.

use super::nodes::OxidtrIR;
use crate::parser::ast::{CompareOp, Expr, LogicOp, QuantKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgebraKind {
    Semigroup,
    Monoid,
    Group,
}

impl AlgebraKind {
    /// konpu directive head (`monoid`, `group`, ...).
    pub fn head(self) -> &'static str {
        match self {
            AlgebraKind::Semigroup => "semigroup",
            AlgebraKind::Monoid => "monoid",
            AlgebraKind::Group => "group",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgebraFact {
    pub sig: String,
    pub kind: AlgebraKind,
    pub op: String,
    pub identity: Option<String>,
    pub inverse: Option<String>,
}

/// Detect algebraic structures proven by the model's facts + assertions.
/// At most one structure per sig (the strongest op family found first).
pub fn detect(ir: &OxidtrIR) -> Vec<AlgebraFact> {
    let laws: Vec<&Expr> = ir
        .constraints
        .iter()
        .map(|c| &c.expr)
        .chain(ir.properties.iter().map(|p| &p.expr))
        .collect();

    let mut out: Vec<AlgebraFact> = Vec::new();
    for (op, sig) in binary_ops(ir) {
        if out.iter().any(|f| f.sig == sig) {
            continue; // one structure per sig
        }
        if !laws.iter().any(|l| matches_assoc(l, &op, &sig)) {
            continue;
        }
        let identity = laws
            .iter()
            .find_map(|l| matches_identity(l, &op, &sig))
            .filter(|e| element_exists(ir, e, &sig));
        let inverse = identity.as_ref().and_then(|e| {
            laws.iter().find_map(|l| matches_inverse(l, &op, &sig, e))
        });
        let kind = match (&identity, &inverse) {
            (Some(_), Some(_)) => AlgebraKind::Group,
            (Some(_), None) => AlgebraKind::Monoid,
            (None, _) => AlgebraKind::Semigroup,
        };
        out.push(AlgebraFact { sig, kind, op, identity, inverse });
    }
    out
}

/// Binary operations: `fun op[a, b: S]: S` (free) or `fun (r: S) op[b: S]: S`
/// (receiver form). Returns (op_name, sig).
fn binary_ops(ir: &OxidtrIR) -> Vec<(String, String)> {
    let mut ops = Vec::new();
    for o in &ir.operations {
        let ret = match &o.return_type {
            Some(r) => &r.type_name,
            None => continue,
        };
        // free form: two params of type S, return S
        if o.receiver_sig.is_none()
            && o.params.len() == 2
            && o.params[0].type_name == *ret
            && o.params[1].type_name == *ret
        {
            ops.push((o.name.clone(), ret.clone()));
            continue;
        }
        // receiver form: receiver S, one param S, return S
        if o.receiver_sig.as_deref() == Some(ret.as_str())
            && o.params.len() == 1
            && o.params[0].type_name == *ret
        {
            ops.push((o.name.clone(), ret.clone()));
        }
    }
    ops
}

/// Normalize a binary-op application to (op_name, left, right), covering both
/// free form `op[x, y]` and receiver form `x.op[y]`.
fn as_binop(e: &Expr) -> Option<(&str, &Expr, &Expr)> {
    match e {
        Expr::FunApp { name, receiver: None, args } if args.len() == 2 => {
            Some((name.as_str(), &args[0], &args[1]))
        }
        Expr::FunApp { name, receiver: Some(r), args } if args.len() == 1 => {
            Some((name.as_str(), r.as_ref(), &args[0]))
        }
        _ => None,
    }
}

fn is_var(e: &Expr, name: &str) -> bool {
    matches!(e, Expr::VarRef(n) if n == name)
}

/// Flatten the vars of a single-domain `all` quantifier over `sig`; returns the
/// var names in order and the body. None if the domain isn't `sig`.
fn all_over<'a>(law: &'a Expr, sig: &str) -> Option<(Vec<&'a str>, &'a Expr)> {
    if let Expr::Quantifier { kind: QuantKind::All, bindings, body } = law {
        let mut vars = Vec::new();
        for b in bindings {
            if !is_var(&b.domain, sig) {
                return None;
            }
            vars.extend(b.vars.iter().map(String::as_str));
        }
        return Some((vars, body));
    }
    None
}

/// Split a conjunction into its (possibly nested) conjuncts; a non-`And`
/// expression is a single conjunct.
fn conjuncts(e: &Expr) -> Vec<&Expr> {
    match e {
        Expr::BinaryLogic { op: LogicOp::And, left, right } => {
            let mut v = conjuncts(left);
            v.extend(conjuncts(right));
            v
        }
        _ => vec![e],
    }
}

/// `all a,b,c: S | op[op[a,b],c] = op[a, op[b,c]]` (either side may be the
/// left-nested one).
fn matches_assoc(law: &Expr, op: &str, sig: &str) -> bool {
    let (vars, body) = match all_over(law, sig) {
        Some(v) => v,
        None => return false,
    };
    if vars.len() < 3 {
        return false;
    }
    let (a, b, c) = (vars[0], vars[1], vars[2]);
    if let Expr::Comparison { op: CompareOp::Eq, left, right } = body {
        (assoc_left(left, op, a, b, c) && assoc_right(right, op, a, b, c))
            || (assoc_left(right, op, a, b, c) && assoc_right(left, op, a, b, c))
    } else {
        false
    }
}

/// `op[op[a,b], c]`
fn assoc_left(e: &Expr, op: &str, a: &str, b: &str, c: &str) -> bool {
    match as_binop(e) {
        Some((name, inner, z)) if name == op && is_var(z, c) => {
            matches!(as_binop(inner), Some((n2, x, y)) if n2 == op && is_var(x, a) && is_var(y, b))
        }
        _ => false,
    }
}

/// `op[a, op[b,c]]`
fn assoc_right(e: &Expr, op: &str, a: &str, b: &str, c: &str) -> bool {
    match as_binop(e) {
        Some((name, x, inner)) if name == op && is_var(x, a) => {
            matches!(as_binop(inner), Some((n2, y, z)) if n2 == op && is_var(y, b) && is_var(z, c))
        }
        _ => false,
    }
}

/// `all a: S | op[a, e] = a` (and/or `op[e, a] = a`); returns the identity
/// element name `e`.
fn matches_identity(law: &Expr, op: &str, sig: &str) -> Option<String> {
    let (vars, body) = all_over(law, sig)?;
    let a = *vars.first()?;
    for conj in conjuncts(body) {
        if let Expr::Comparison { op: CompareOp::Eq, left, right } = conj {
            if !is_var(right, a) {
                continue;
            }
            if let Some((name, x, y)) = as_binop(left) {
                if name != op {
                    continue;
                }
                // one operand is the bound var `a`, the other is the identity element
                if is_var(x, a) {
                    if let Expr::VarRef(e) = y {
                        return Some(e.clone());
                    }
                } else if is_var(y, a) {
                    if let Expr::VarRef(e) = x {
                        return Some(e.clone());
                    }
                }
            }
        }
    }
    None
}

/// `all a: S | op[a, inv[a]] = e`; returns the inverse operation name `inv`.
fn matches_inverse(law: &Expr, op: &str, sig: &str, identity: &str) -> Option<String> {
    let (vars, body) = all_over(law, sig)?;
    let a = *vars.first()?;
    for conj in conjuncts(body) {
        if let Expr::Comparison { op: CompareOp::Eq, left, right } = conj {
            if !is_var(right, identity) {
                continue;
            }
            let (name, x, y) = as_binop(left)?;
            if name != op {
                continue;
            }
            // one operand is `a`, the other is a unary application `inv[a]` / `a.inv`
            let other = if is_var(x, a) {
                y
            } else if is_var(y, a) {
                x
            } else {
                continue;
            };
            if let Some(inv) = unary_of(other, a) {
                return Some(inv.to_string());
            }
        }
    }
    None
}

/// `inv[a]` (free) or `a.inv` (receiver); returns the function name.
fn unary_of<'a>(e: &'a Expr, a: &str) -> Option<&'a str> {
    match e {
        Expr::FunApp { name, receiver: None, args } if args.len() == 1 && is_var(&args[0], a) => {
            Some(name.as_str())
        }
        Expr::FunApp { name, receiver: Some(r), args } if args.is_empty() && is_var(r, a) => {
            Some(name.as_str())
        }
        _ => None,
    }
}

/// The identity element is real: a `one sig` of the type, or a nullary fun
/// returning it. Guards against matching a stray quantifier variable as identity.
fn element_exists(ir: &OxidtrIR, name: &str, sig: &str) -> bool {
    use crate::parser::ast::SigMultiplicity;
    let is_one_sig = ir.structures.iter().any(|s| {
        s.name == name
            && s.sig_multiplicity == SigMultiplicity::One
            && (s.name == sig || s.parent.as_deref() == Some(sig))
    });
    let is_const_fun = ir.operations.iter().any(|o| {
        o.name == name
            && o.params.is_empty()
            && o.return_type.as_ref().is_some_and(|r| r.type_name == sig)
    });
    is_one_sig || is_const_fun
}
