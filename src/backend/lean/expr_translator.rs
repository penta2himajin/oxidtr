use crate::parser::ast::*;
use crate::ir::nodes::OxidtrIR;
use std::collections::HashSet;

pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

pub fn translate_with_ir(expr: &Expr, ir: &OxidtrIR) -> String {
    let sig_names = collect_sig_names(ir);
    translate_inner(expr, false, &sig_names, ir)
}

/// Whether a sig name is used where a *value* belongs.
///
/// In Alloy a sig name in an expression is the set of its atoms — `#P`,
/// `x in Person`. This encoding gives a sig a Lean type and nothing else:
/// there is no term for its extent, so `P.length` names a type where a value
/// belongs and does not elaborate. A quantifier domain is the one position
/// where the type *is* what is meant (`∀ p : P`), and a variant is a
/// constructor of its parent rather than an extent (#105).
///
/// The caller emits `sorry` rather than a body that cannot type-check.
pub fn mentions_whole_sig_as_value(expr: &Expr, ir: &OxidtrIR) -> bool {
    let sig_names = collect_sig_names(ir);
    fn walk(e: &Expr, sigs: &HashSet<String>, ir: &OxidtrIR) -> bool {
        match e {
            Expr::VarRef(name) => {
                sigs.contains(name)
                    && crate::backend::variant_parent(ir, name).is_none()
                    && !crate::backend::is_native_type_alias(name)
            }
            Expr::Quantifier { bindings, body, .. } => {
                bindings.iter().any(|b| match &b.domain {
                    // `∀ p : P` — the type is what is meant here.
                    Expr::VarRef(n) if sigs.contains(n) => false,
                    d => walk(d, sigs, ir),
                }) || walk(body, sigs, ir)
            }
            Expr::Comparison { left, right, .. } | Expr::BinaryLogic { left, right, .. }
            | Expr::SetOp { left, right, .. } | Expr::Product { left, right }
            | Expr::TemporalBinary { left, right, .. } => {
                walk(left, sigs, ir) || walk(right, sigs, ir)
            }
            Expr::Not(i) | Expr::Cardinality(i) | Expr::TransitiveClosure(i)
            | Expr::ReflexiveClosure(i) | Expr::MultFormula { expr: i, .. }
            | Expr::FieldAccess { base: i, .. } | Expr::TemporalUnary { expr: i, .. }
            | Expr::Prime(i) => walk(i, sigs, ir),
            Expr::FunApp { receiver, args, .. } => {
                receiver.as_deref().is_some_and(|r| walk(r, sigs, ir))
                    || args.iter().any(|a| walk(a, sigs, ir))
            }
            Expr::IntLiteral(_) => false,
        }
    }
    walk(expr, &sig_names, ir)
}

/// `x = Variant` / `x != Variant`, as equality with the constructor.
///
/// A `one sig` extending an abstract sig is one atom, so equality with it asks
/// which case the other side is. The variant is a *constructor* of the parent
/// `inductive`, not a type, so the bare name does not elaborate where a value
/// belongs (#105).
fn variant_case_test<F>(
    left: &Expr, right: &Expr, ir: &OxidtrIR, negated: bool, ti: &F,
) -> Option<String>
where F: Fn(&Expr) -> String {
    let variant_of = |e: &Expr| match e {
        Expr::VarRef(name) => crate::backend::variant_parent(ir, name).map(|_| name.clone()),
        _ => None,
    };
    let (variant, subject) = match (variant_of(left), variant_of(right)) {
        (Some(_), Some(_)) | (None, None) => return None,
        (Some(v), None) => (v, right),
        (None, Some(v)) => (v, left),
    };
    // `matches` is the same spelling the exhaustive-categories path already
    // emits, and `..` covers a constructor's payload without naming it.
    let test = format!("{} matches .{} ..", ti(subject), lean_field(&variant));
    Some(if negated { format!("¬({test})") } else { test })
}

/// Lean tokens that cannot appear as a bare identifier. Swept empirically
/// against Lean 4.31 — each one breaks the parse (or, for `Type`/`Prop`/`Sort`,
/// elaborates to the wrong thing) in declaration, field, binder or projection
/// position. Native type aliases are absent by construction, so wrapping is
/// never applied to `Int`/`String`/`Bool` (the C# `@long` trap from #102).
const LEAN_KEYWORDS: &[&str] = &[
    "Prop", "Sort", "Type", "abbrev", "at", "attribute", "axiom", "break", "by",
    "calc", "catch", "class", "continue", "declare_syntax_cat", "def", "deriving",
    "do", "elab", "else", "end", "example", "export", "extends", "finally", "for",
    "from", "fun", "have", "if", "import", "in", "inductive", "infix", "infixl",
    "infixr", "initialize", "instance", "let", "local", "macro", "macro_rules",
    "match", "matches", "mut", "mutual", "namespace", "nofun", "nomatch",
    "noncomputable", "notation", "opaque", "open", "partial", "postfix", "prefix",
    "private", "protected", "return", "scoped", "section", "set_option", "show",
    "sorry", "structure", "suffices", "then", "theorem", "try", "universe",
    "unless", "unsafe", "variable", "where", "while", "with",
];

/// Undoes [`lean_ident`]. Only a wrapper around a token from the list above is
/// unwrapped, so a user identifier quoted for reasons of its own (`«foo bar»`)
/// survives intact.
///
/// ponytail: still context-blind. A hand-written string literal or comment that
/// contains exactly `«in»` is rewritten to `in`, because this is a plain
/// substring pass rather than a lexer. Narrowing it from "strip every
/// guillemet" to "unwrap known keywords" shrinks the blast radius to that case;
/// closing it properly means unescaping per-identifier inside the parsers below.
pub fn unescape_lean_ident(text: &str) -> String {
    let mut out = text.to_string();
    for kw in LEAN_KEYWORDS {
        out = out.replace(&format!("«{kw}»"), kw);
    }
    out
}

/// Wraps a reserved token in guillemets so it can be used as an identifier.
pub fn lean_ident(name: &str) -> String {
    if LEAN_KEYWORDS.contains(&name) { format!("«{name}»") } else { name.to_string() }
}

/// A field, binder or parameter name: lower-camelled, then escaped. Lowering
/// the first character can *manufacture* a keyword (`End` → `end`), so the two
/// steps have to happen in this order.
pub fn lean_field(name: &str) -> String {
    lean_ident(&to_lower_camel(name))
}

pub fn to_lower_camel(name: &str) -> String {
    if name.is_empty() { return name.to_string(); }
    let mut chars = name.chars();
    let first = chars.next().unwrap().to_lowercase().to_string();
    format!("{first}{}", chars.collect::<String>())
}

/// The binary relation behind Alloy's `^f`. `b ∈ a.f` is well-typed for both
/// encodings a field can take — `Option T` for `lone` and `List T` for `set` —
/// so this needs no multiplicity lookup (which Lean's translator cannot do
/// soundly anyway; see issue #95).
fn closure_relation(field: &str) -> String {
    format!("(fun a b => b ∈ a.{})", lean_field(field))
}

fn translate_inner(
    expr: &Expr,
    parens_if_complex: bool,
    sig_names: &HashSet<String>,
    ir: &OxidtrIR,
) -> String {
    let ti = |e: &Expr, p: bool| translate_inner(e, p, sig_names, ir);

    let result = match expr {
        Expr::IntLiteral(n) => n.to_string(),

        Expr::VarRef(name) => {
            // Alloy's implicit receiver in `fun/pred Sig.op[...] { this... }` is
            // the binder `generate_derived_fields` emits, which is named `self`.
            // Mapped unconditionally, as the Rust backend does.
            if name == "this" {
                "self".to_string()
            } else if sig_names.contains(name) {
                lean_ident(name)
            } else {
                lean_field(name)
            }
        }

        Expr::FieldAccess { base, field } => {
            format!("{}.{}", ti(base, false), lean_field(field))
        }

        Expr::Cardinality(inner) => format!("{}.length", ti(inner, false)),

        Expr::TransitiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Relation.TransGen {} {}", closure_relation(field), ti(base, true))
            } else {
                format!("Relation.TransGen {}", ti(inner, false))
            }
        }

        Expr::ReflexiveClosure(inner) => {
            if let Expr::FieldAccess { base, field } = inner.as_ref() {
                format!("Relation.ReflTransGen (· {} ·) {}", to_lower_camel(field), ti(base, false))
            } else {
                format!("Relation.ReflTransGen {}", ti(inner, false))
            }
        }

        Expr::Comparison { op, left, right } => {
            match op {
                CompareOp::Eq | CompareOp::NotEq => {
                    let negated = matches!(op, CompareOp::NotEq);
                    // The result flows into the `parens_if_complex` wrapping
                    // below: an `∃` prefix binds looser than everything and
                    // would swallow a conjunct if it escaped unparenthesised.
                    match variant_case_test(left, right, ir, negated, &|e| ti(e, false)) {
                        Some(t) => t,
                        None => {
                            let op_str = if negated { "≠" } else { "=" };
                            format!("{} {op_str} {}", ti(left, false), ti(right, false))
                        }
                    }
                }
                CompareOp::Lt => format!("{} < {}", ti(left, false), ti(right, false)),
                CompareOp::Gt => format!("{} > {}", ti(left, false), ti(right, false)),
                CompareOp::Lte => format!("{} ≤ {}", ti(left, false), ti(right, false)),
                CompareOp::Gte => format!("{} ≥ {}", ti(left, false), ti(right, false)),
                CompareOp::In => {
                    // `x in y.^f` is reachability, not membership: `TransGen`
                    // wants the relation and *both* endpoints, so the closure
                    // cannot be translated on its own and then tested with `∈`.
                    let reachability = match right.as_ref() {
                        Expr::TransitiveClosure(c) => match c.as_ref() {
                            Expr::FieldAccess { base, field } => Some(format!(
                                "Relation.TransGen {} {} {}",
                                closure_relation(field), ti(base, true), ti(left, true))),
                            _ => None,
                        },
                        _ => None,
                    };
                    reachability.unwrap_or_else(||
                        format!("{} ∈ {}", ti(left, false), ti(right, false)))
                }
            }
        }

        Expr::BinaryLogic { op, left, right } => match op {
            LogicOp::And     => format!("{} ∧ {}", ti(left, true), ti(right, true)),
            LogicOp::Or      => format!("{} ∨ {}", ti(left, true), ti(right, true)),
            LogicOp::Implies => format!("{} → {}", ti(left, true), ti(right, false)),
            LogicOp::Iff     => format!("{} ↔ {}", ti(left, true), ti(right, true)),
        },

        Expr::Not(inner) => format!("¬{}", ti(inner, true)),

        Expr::Quantifier { kind, bindings, body } => {
            // `no x, y | P` is `¬ ∃ x, ∃ y, P` — the negation belongs to the
            // whole prefix, so only the first binder carries it.
            let (first, rest) = match kind {
                QuantKind::All => ("∀", "∀"),
                QuantKind::Some => ("∃", "∃"),
                QuantKind::No => ("¬ ∃", "∃"),
            };
            // One quantifier per variable: Lean accepts `∀ x y : T,` but not
            // `∀ x y ∈ e,`, and comma-joining binders is invalid either way.
            let mut binders: Vec<String> = Vec::new();
            for b in bindings {
                let domain = ti(&b.domain, false);
                // A sig name is a Lean type and binds with `:`; any other
                // expression is a set, which binds with `∈`.
                let sep = if matches!(&b.domain, Expr::VarRef(n) if sig_names.contains(n)) { ":" } else { "∈" };
                for v in &b.vars {
                    binders.push(format!("{} {sep} {domain}", lean_field(v)));
                }
            }
            let mut out = String::new();
            for (i, b) in binders.iter().enumerate() {
                out.push_str(if i == 0 { first } else { rest });
                out.push_str(&format!(" {b}, "));
            }
            out.push_str(&ti(body, false));
            out
        }

        Expr::MultFormula { kind, expr: inner } => {
            match kind {
                QuantKind::Some => format!("{} ≠ none", ti(inner, false)),
                QuantKind::No => format!("{} = none", ti(inner, false)),
                _ => ti(inner, false),
            }
        }

        Expr::SetOp { op, left, right } => {
            let op_str = match op {
                SetOpKind::Union => "∪",
                SetOpKind::Intersection => "∩",
                SetOpKind::Difference => "\\",
            };
            format!("{} {} {}", ti(left, true), op_str, ti(right, true))
        }

        Expr::Product { left, right } => {
            format!("{} × {}", ti(left, true), ti(right, true))
        }

        Expr::Prime(inner) => {
            format!("{}' ", ti(inner, false)).trim().to_string()
        }

        Expr::TemporalUnary { op, expr: inner } => {
            let op_str = match op {
                TemporalUnaryOp::Always => "□",
                TemporalUnaryOp::Eventually => "◇",
                TemporalUnaryOp::After => "◯",
                TemporalUnaryOp::Historically => "■",
                TemporalUnaryOp::Once => "◆",
                TemporalUnaryOp::Before => "◯⁻¹",
            };
            format!("{op_str} {}", ti(inner, false))
        }

        Expr::TemporalBinary { op, left, right } => {
            let op_str = match op {
                TemporalBinaryOp::Until => "𝒰",
                TemporalBinaryOp::Since => "𝒮",
                TemporalBinaryOp::Release => "ℛ",
                TemporalBinaryOp::Triggered => "𝒯",
            };
            format!("{} {op_str} {}", ti(left, true), ti(right, true))
        }

        Expr::FunApp { name, receiver, args } => {
            let args_str: Vec<String> = args.iter().map(|a| ti(a, true)).collect();
            let callee = lean_field(name);
            if let Some(recv) = receiver {
                if args_str.is_empty() {
                    format!("{}.{callee}", ti(recv, false))
                } else {
                    format!("{}.{callee} {}", ti(recv, false), args_str.join(" "))
                }
            } else if args_str.is_empty() {
                callee
            } else {
                format!("{callee} {}", args_str.join(" "))
            }
        }
    };

    if parens_if_complex && result.contains(' ') {
        format!("({result})")
    } else {
        result
    }
}
