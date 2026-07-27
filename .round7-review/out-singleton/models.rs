/// Invariant: SingletonStep
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct C {
    /// MUTABLE: this field changes across state transitions
    pub v: i64,
}

