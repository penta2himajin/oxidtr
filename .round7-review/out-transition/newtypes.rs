#[allow(unused_imports)]
use super::models::*;
#[allow(unused_imports)]
use super::fixtures::*;

/// Newtype wrapper: C validated by NestedStep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatedC(pub C);

impl TryFrom<C> for ValidatedC {
    type Error = &'static str;

    fn try_from(value: C) -> Result<Self, Self::Error> {
        if true {
            Ok(ValidatedC(value))
        } else {
            Err("NestedStep invariant violated")
        }
    }
}

/// Newtype wrapper: D validated by NestedStep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidatedD(pub D);

impl TryFrom<D> for ValidatedD {
    type Error = &'static str;

    fn try_from(value: D) -> Result<Self, Self::Error> {
        if true {
            Ok(ValidatedD(value))
        } else {
            Err("NestedStep invariant violated")
        }
    }
}

