//! The typing layer every backend's `expr_translator` resolves fields through.
//!
//! Alloy expressions carry no types of their own, so a translator has to derive
//! them. The tempting shortcut — scan every sig for a field of the given name —
//! is unsound: two sigs may declare the same field name with different targets
//! and different multiplicities, and the scan then returns whichever the IR
//! happens to list first. That silently produces non-compiling closures, and
//! worse, membership and emptiness tests that compile clean and are always
//! false (#90, #93, #95, #108, #111, #115).
//!
//! The only sound route is through the *binding*: a quantifier binder records
//! which sig its variable ranges over, and `base.field` is resolved against
//! `base`'s sig. This module holds that logic once, in language-agnostic form,
//! so a new backend inherits it rather than re-deriving it.

use crate::ir::nodes::{IRField, OxidtrIR};
use crate::parser::ast::{Expr, QuantBinding};
use std::collections::{HashMap, HashSet};

/// The set of sig names in a model. A bare `VarRef` matching one denotes the
/// sig itself (`all x: Foo | ..` has `Foo` as a domain, not as a variable).
pub fn collect_sig_names(ir: &OxidtrIR) -> HashSet<String> {
    ir.structures.iter().map(|s| s.name.clone()).collect()
}

/// Maps each in-scope quantifier-bound variable to the sig it ranges over.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeEnv {
    vars: HashMap<String, String>,
}

impl TypeEnv {
    pub fn new() -> Self {
        Self::default()
    }

    /// The sig `var` ranges over, if it is bound in this scope.
    pub fn sig_of(&self, var: &str) -> Option<&str> {
        self.vars.get(var).map(|s| s.as_str())
    }

    pub fn bind(&mut self, var: &str, sig: &str) {
        self.vars.insert(var.to_string(), sig.to_string());
    }

    /// A child scope with `bindings` added.
    ///
    /// Bindings are sequential — `all b: Box, x: b.items | ..` binds `b` before
    /// `x`'s domain is resolved — so each one is entered before the next is
    /// typed. A domain that resolves to no sig *removes* the variable instead of
    /// leaving it: an enclosing binding of the same name is shadowed by this
    /// one, and inheriting its sig would type the variable as something it is
    /// demonstrably not.
    pub fn extended(&self, bindings: &[QuantBinding], sig_names: &HashSet<String>, ir: &OxidtrIR) -> Self {
        let mut inner = self.clone();
        for b in bindings {
            let sig = expr_sig(&b.domain, sig_names, ir, &inner);
            for v in &b.vars {
                match &sig {
                    Some(s) => inner.bind(v, s),
                    None => { inner.vars.remove(v); }
                }
            }
        }
        inner
    }
}

/// The sig an expression denotes, or `None` when it denotes no sig — a literal,
/// a native scalar, or a domain this layer cannot resolve.
///
/// `None` is a real answer, not a failure: callers must fall back to whatever
/// their language does for untyped expressions rather than guess a sig.
pub fn expr_sig(expr: &Expr, sig_names: &HashSet<String>, ir: &OxidtrIR, env: &TypeEnv) -> Option<String> {
    match expr {
        Expr::VarRef(name) => env
            .sig_of(name)
            .map(|s| s.to_string())
            .or_else(|| sig_names.contains(name).then(|| name.clone())),
        Expr::FieldAccess { base, field } => {
            resolve_field(base, field, sig_names, ir, env).map(|f| f.target.clone())
        }
        // `^f` and `*f` range over the same sig as `f` itself, so closure is
        // transparent to typing.
        Expr::TransitiveClosure(inner) | Expr::ReflexiveClosure(inner) => {
            expr_sig(inner, sig_names, ir, env)
        }
        _ => None,
    }
}

/// Resolve `base.field` to the field declaration on `base`'s sig.
///
/// The inheritance chain is walked because a field declared on an abstract
/// parent is reachable through any child (#93).
pub fn resolve_field<'a>(
    base: &Expr,
    field: &str,
    sig_names: &HashSet<String>,
    ir: &'a OxidtrIR,
    env: &TypeEnv,
) -> Option<&'a IRField> {
    resolve_field_owner(base, field, sig_names, ir, env).map(|(_, f)| f)
}

/// As `resolve_field`, but also naming the sig that *declares* the field.
///
/// For an inherited field that is the parent, not the child the expression went
/// through — anything recorded per declaration site, such as Rust's boxing of
/// cyclic fields, has to be keyed on the declaring sig to be found.
pub fn resolve_field_owner<'a>(
    base: &Expr,
    field: &str,
    sig_names: &HashSet<String>,
    ir: &'a OxidtrIR,
    env: &TypeEnv,
) -> Option<(String, &'a IRField)> {
    let mut sig = expr_sig(base, sig_names, ir, env)?;
    loop {
        let st = ir.structures.iter().find(|s| s.name == sig)?;
        if let Some(f) = st.fields.iter().find(|f| f.name == field) {
            return Some((st.name.clone(), f));
        }
        sig = st.parent.clone()?;
    }
}
