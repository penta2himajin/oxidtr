//! The declines we already know about, and are not failing the build over.
//!
//! `check` reports every element a backend declined to express (#97). On
//! models/oxidtr.als the Lean backend declines sixteen of them, so failing on
//! any decline would fail the build on the day the check landed — and a check
//! that fails on day one gets turned off, which leaves the gap unmeasured
//! again.
//!
//! A baseline file names the declines already known. Only a decline it does
//! not name fails. What makes it a ratchet rather than an amnesty is the other
//! direction: an entry that is no longer declined is reported too, so closing
//! a gap forces the file to shrink and the permission cannot outlive the
//! limitation it was granted for.
//!
//! This is a policy over the result, deliberately not part of computing it.
//! What the implementation did and what we are willing to live with are
//! separate questions, and only the first belongs in a diff.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use super::differ::DiffItem;

/// The elements whose declines are accepted, as `(kind, name)`.
#[derive(Debug, Clone, Default)]
pub struct AcceptedDeclines {
    entries: BTreeSet<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedParseError {
    pub line_number: usize,
    pub line: String,
    pub problem: String,
}

impl fmt::Display for AcceptedParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "accepted-declines line {}: {} — {:?}",
            self.line_number, self.problem, self.line)
    }
}

/// Reading a baseline can fail for two unrelated reasons, and the message
/// should say which.
#[derive(Debug)]
pub enum AcceptedLoadError {
    Io(std::io::Error),
    Parse(AcceptedParseError),
}

impl fmt::Display for AcceptedLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptedLoadError::Io(e) => write!(f, "{e}"),
            AcceptedLoadError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcceptedLoadError {}

impl AcceptedDeclines {
    pub fn load(path: &Path) -> Result<Self, AcceptedLoadError> {
        let text = std::fs::read_to_string(path).map_err(AcceptedLoadError::Io)?;
        Self::parse(&text).map_err(AcceptedLoadError::Parse)
    }

    /// One `<kind> <name>` per line; `#` starts a comment.
    ///
    /// A malformed line is an error rather than a skipped line: a typo that
    /// silently dropped an entry would fail the build on an element that was
    /// meant to be accepted, and the file would grow a duplicate instead of
    /// being fixed.
    pub fn parse(text: &str) -> Result<Self, AcceptedParseError> {
        let mut entries = BTreeSet::new();
        for (i, line) in text.lines().enumerate() {
            let trimmed = match line.split_once('#') {
                Some((before, _)) => before.trim(),
                None => line.trim(),
            };
            if trimmed.is_empty() { continue; }

            let err = |problem: &str| AcceptedParseError {
                line_number: i + 1,
                line: line.to_string(),
                problem: problem.to_string(),
            };

            let mut parts = trimmed.split_whitespace();
            let kind = parts.next().ok_or_else(|| err("no kind"))?;
            let name = parts.next().ok_or_else(|| err("no element name"))?;
            if parts.next().is_some() {
                return Err(err("trailing text after the element name"));
            }
            if kind != "fact" && kind != "assert" {
                return Err(err("kind is neither `fact` nor `assert`"));
            }
            entries.insert((kind.to_string(), name.to_string()));
        }
        Ok(AcceptedDeclines { entries })
    }

    pub fn is_empty(&self) -> bool { self.entries.is_empty() }

    pub fn len(&self) -> usize { self.entries.len() }

    /// Drop the declines this baseline accepts, and report the entries it
    /// accepts that are no longer declined.
    ///
    /// Only `DeclinedCoverage` is affected. The baseline records what oxidtr
    /// cannot express; it is not a way to silence a structural diff, and every
    /// other kind passes through however the file is written.
    pub fn apply(&self, diffs: Vec<DiffItem>) -> Vec<DiffItem> {
        let still_declined: BTreeSet<(String, String)> = diffs.iter()
            .filter_map(|d| match d {
                DiffItem::DeclinedCoverage { kind, name, .. } =>
                    Some((kind.clone(), name.clone())),
                _ => None,
            })
            .collect();

        let mut out: Vec<DiffItem> = diffs.into_iter()
            .filter(|d| match d {
                DiffItem::DeclinedCoverage { kind, name, .. } =>
                    !self.entries.contains(&(kind.clone(), name.clone())),
                _ => true,
            })
            .collect();

        for (kind, name) in &self.entries {
            if !still_declined.contains(&(kind.clone(), name.clone())) {
                out.push(DiffItem::StaleAcceptance {
                    kind: kind.clone(),
                    name: name.clone(),
                });
            }
        }
        out
    }
}
