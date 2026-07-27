/// Invariant: NestedStep
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct C {
    /// MUTABLE: this field changes across state transitions
    pub v: i64,
}

/// Invariant: NestedStep
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct D {
    /// MUTABLE: this field changes across state transitions
    pub v: i64,
}

