//! What each backend did with each element of the model.
//!
//! `check` used to answer "was this fact verified?" by searching the generated
//! source for the fact's name. That counts a comment as a proof: a fact whose
//! only trace is `// oxidtr: skipped — …` satisfied the search, so the
//! diagnostic announcing that the guarantee had been dropped was itself the
//! evidence that it had not (#97).
//!
//! The answer belongs where it is known — in the emitter, at the moment it
//! decides — not in a later reading of the text it produced. Each backend
//! records one status per model element and renders a manifest beside the code;
//! `check` reads that instead of guessing.

use std::collections::BTreeMap;
use std::fmt;

/// The kind of model element a status is about.
///
/// Facts sort before asserts in the manifest: a fact constrains every instance,
/// an assert is a claim about them, and reading the stronger obligation first
/// matches how the model is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ElementKind {
    Fact,
    Assert,
}

impl ElementKind {
    fn tag(self) -> &'static str {
        match self {
            ElementKind::Fact => "fact",
            ElementKind::Assert => "assert",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(ElementKind::Fact),
            "assert" => Some(ElementKind::Assert),
            _ => None,
        }
    }
}

/// What a backend did with one model element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// A real assertion exists for it.
    Verified,
    /// The language's type system encodes it, so no test is needed. This is
    /// `Guarantee::FullyByType` — not a gap, and the reason the guarantee
    /// budget balances across languages of different type strength.
    ByType,
    /// oxidtr cannot express it in this language. The reason is prose for a
    /// human; `check` compares the element, not the wording.
    ///
    /// A quantifier over an empty domain lands here too. It is true whatever
    /// the implementation does, so it verifies nothing.
    Declined(String),
}

impl Verification {
    /// Higher wins when an element is recorded more than once.
    ///
    /// One element draws several emitters — an invariant test, boundary tests,
    /// a cross-test scaffold — and only one of them needs to do the work. A
    /// `Declined` from the pairwise scaffold must not mask the invariant test
    /// that actually asserts.
    fn rank(&self) -> u8 {
        match self {
            Verification::Verified => 2,
            Verification::ByType => 1,
            Verification::Declined(_) => 0,
        }
    }

    fn tag(&self) -> &'static str {
        match self {
            Verification::Verified => "verified",
            Verification::ByType => "by-type",
            Verification::Declined(_) => "declined",
        }
    }
}

/// One model element and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageEntry {
    pub kind: ElementKind,
    pub name: String,
    pub status: Verification,
}

impl CoverageEntry {
    /// Why this element was declined, or `None` if it was not.
    pub fn reason(&self) -> Option<&str> {
        match &self.status {
            Verification::Declined(r) => Some(r.as_str()),
            _ => None,
        }
    }
}

/// The manifest a backend renders beside the code it generated.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    entries: BTreeMap<(ElementKind, String), Verification>,
}

/// A manifest that could not be read. `check` reports this rather than
/// treating an unreadable manifest as "nothing was declined".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageParseError {
    pub line_number: usize,
    pub line: String,
    pub problem: String,
}

impl fmt::Display for CoverageParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "coverage manifest line {}: {} — {:?}",
            self.line_number, self.problem, self.line)
    }
}

/// Separates the element from a decline's reason. Chosen so a name can never
/// contain it: Alloy identifiers have no spaces or hyphens.
const REASON_SEP: &str = " -- ";

impl Coverage {
    pub fn new() -> Self {
        Coverage { entries: BTreeMap::new() }
    }

    /// Record what happened to one element. The strongest status wins, so an
    /// emitter may report its own outcome without knowing what the others did.
    pub fn record(&mut self, kind: ElementKind, name: &str, status: Verification) {
        let key = (kind, name.to_string());
        match self.entries.get(&key) {
            Some(existing) if existing.rank() >= status.rank() => {}
            _ => { self.entries.insert(key, status); }
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = CoverageEntry> + '_ {
        self.entries.iter().map(|((kind, name), status)| CoverageEntry {
            kind: *kind,
            name: name.clone(),
            status: status.clone(),
        })
    }

    /// The elements this backend could not express — the gap, in full.
    pub fn declined(&self) -> impl Iterator<Item = CoverageEntry> + '_ {
        self.entries().filter(|e| matches!(e.status, Verification::Declined(_)))
    }

    /// The manifest, sorted so the bytes depend on the model rather than on the
    /// order the backend visited it in.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "# oxidtr coverage manifest\n\
             # <status> <kind> <name> [-- why it was declined]\n\
             # status: verified | by-type | declined\n");
        for entry in self.entries() {
            out.push_str(&format!("{:<8} {:<6} {}",
                entry.status.tag(), entry.kind.tag(), entry.name));
            if let Some(reason) = entry.reason() {
                // A reason is free prose an emitter wrote. Flattened, it cannot
                // end the line early and forge a second entry.
                out.push_str(REASON_SEP);
                out.push_str(&flatten(reason));
            }
            out.push('\n');
        }
        out
    }

    /// Read back a manifest `render` wrote.
    pub fn parse(text: &str) -> Result<Coverage, CoverageParseError> {
        let mut cov = Coverage::new();
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') { continue; }

            let err = |problem: &str| CoverageParseError {
                line_number: i + 1,
                line: line.to_string(),
                problem: problem.to_string(),
            };

            let (head, reason) = match trimmed.split_once(REASON_SEP) {
                Some((h, r)) => (h, Some(r.trim().to_string())),
                None => (trimmed, None),
            };
            let mut parts = head.split_whitespace();
            let status_tag = parts.next().ok_or_else(|| err("no status"))?;
            let kind_tag = parts.next().ok_or_else(|| err("no kind"))?;
            let name = parts.next().ok_or_else(|| err("no element name"))?;
            if parts.next().is_some() {
                return Err(err("trailing text after the element name"));
            }

            let kind = ElementKind::parse(kind_tag)
                .ok_or_else(|| err("kind is neither `fact` nor `assert`"))?;
            let status = match status_tag {
                "verified" => Verification::Verified,
                "by-type" => Verification::ByType,
                "declined" => Verification::Declined(reason.unwrap_or_default()),
                _ => return Err(err("unknown status")),
            };
            cov.record(kind, name, status);
        }
        Ok(cov)
    }
}

/// Collapse a reason onto one line so it cannot break the manifest's framing.
fn flatten(reason: &str) -> String {
    let collapsed: String = reason
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    collapsed.split_whitespace().collect::<Vec<_>>().join(" ")
}
