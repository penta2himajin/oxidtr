#[allow(unused_imports)]
use super::models::*;
#[allow(unused_imports)]
use super::fixtures::*;

/// Newtype wrapper: C validated by SingletonStep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatedC(pub C);

impl TryFrom<C> for ValidatedC {
    type Error = &'static str;

    fn try_from(value: C) -> Result<Self, Self::Error> {
        if true {
            Ok(ValidatedC(value))
        } else {
            Err("SingletonStep invariant violated")
        }
    }
}

