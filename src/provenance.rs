//! Where a number came from, and the rule that carries it through arithmetic.

/// How a value was arrived at.
///
/// The discriminants are ordered by strength so that the weakest is the
/// smallest. `Derived` was swept against a model, `Extracted` was read out of
/// source or an image, `Measured` was observed on hardware, and `Assumed` is a
/// guess that carries its reason.
///
/// The ordering is the one the founding design settled on. It is load-bearing:
/// [`Provenance::join`] is a minimum over it, so reordering these variants
/// relabels every result the tool prints.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
#[repr(u8)]
pub enum Provenance {
    /// A guess. The weakest, and the one the tool exists to make visible.
    Assumed = 0,
    /// Observed on hardware, through some instrument, on some run.
    Measured = 1,
    /// Read out of source or an image, at some file and symbol.
    Extracted = 2,
    /// Swept against a model. The strongest.
    Derived = 3,
}

impl Provenance {
    /// The rank used by [`join`](Self::join). Larger is stronger.
    #[inline]
    #[must_use]
    pub const fn strength(self) -> u8 {
        self as u8
    }

    /// The weaker of two provenances.
    ///
    /// This is the whole of provenance monotonicity. A result carries the weakest provenance of
    /// anything in its derivation, so one `Assumed` input makes the answer
    /// `Assumed` wherever it is printed.
    #[inline]
    #[must_use]
    pub const fn join(self, other: Self) -> Self {
        if (self as u8) <= (other as u8) {
            self
        } else {
            other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Provenance::{Assumed, Derived, Extracted, Measured};

    #[test]
    fn one_assumed_input_makes_the_answer_assumed() {
        assert_eq!(Derived.join(Assumed), Assumed);
        assert_eq!(Assumed.join(Derived), Assumed);
        assert_eq!(Extracted.join(Measured), Measured);
        assert_eq!(Derived.join(Derived), Derived);
    }

    #[test]
    fn the_ordering_is_the_declared_one() {
        assert!(Assumed < Measured);
        assert!(Measured < Extracted);
        assert!(Extracted < Derived);
    }
}
