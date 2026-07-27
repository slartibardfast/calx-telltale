//! Integer intervals, and arithmetic that refuses rather than wraps.

use crate::Count;

/// A closed interval of `Count`, with `lo <= hi` held as an invariant.
///
/// Every number in the model is an interval. An exact value is a point, where
/// `lo == hi`. Arithmetic here is conservative, so the result contains the
/// result of every point-valued evaluation within the inputs, and it returns
/// `None` rather than wrapping where it would overflow.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Interval {
    lo: Count,
    hi: Count,
}

impl Interval {
    /// An interval from `lo` to `hi`, or `None` if they are the wrong way round.
    #[inline]
    #[must_use]
    pub const fn new(lo: Count, hi: Count) -> Option<Self> {
        if lo <= hi {
            Some(Self { lo, hi })
        } else {
            None
        }
    }

    /// An exact value.
    #[inline]
    #[must_use]
    pub const fn point(v: Count) -> Self {
        Self { lo: v, hi: v }
    }

    #[inline]
    #[must_use]
    pub const fn lo(self) -> Count {
        self.lo
    }

    #[inline]
    #[must_use]
    pub const fn hi(self) -> Count {
        self.hi
    }

    /// Whether this interval is a single value.
    #[inline]
    #[must_use]
    pub const fn is_exact(self) -> bool {
        self.lo == self.hi
    }

    #[inline]
    #[must_use]
    pub const fn contains(self, v: Count) -> bool {
        self.lo <= v && v <= self.hi
    }

    /// Sum, or `None` on overflow.
    #[inline]
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match (self.lo.checked_add(other.lo), self.hi.checked_add(other.hi)) {
            (Some(lo), Some(hi)) => Some(Self { lo, hi }),
            _ => None,
        }
    }

    /// Product, or `None` on overflow.
    ///
    /// Both operands are non-negative, so the endpoints of the product are the
    /// products of the endpoints and no cross-term can be smaller or larger.
    #[inline]
    #[must_use]
    pub const fn checked_mul(self, other: Self) -> Option<Self> {
        match (self.lo.checked_mul(other.lo), self.hi.checked_mul(other.hi)) {
            (Some(lo), Some(hi)) => Some(Self { lo, hi }),
            _ => None,
        }
    }

    /// The pointwise maximum, which is the cost of the more expensive branch.
    #[inline]
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        Self {
            lo: if self.lo >= other.lo {
                self.lo
            } else {
                other.lo
            },
            hi: if self.hi >= other.hi {
                self.hi
            } else {
                other.hi
            },
        }
    }

    /// Round each endpoint up to the next multiple of `g`, or `None` if `g` is
    /// zero or the rounding overflows.
    ///
    /// This is what a sleep primitive does when it rounds up to a whole tick.
    /// Rounding up is monotone, so mapping the endpoints maps the interval.
    #[must_use]
    pub const fn checked_round_up_to_multiple(self, g: Count) -> Option<Self> {
        match (ceil_to_multiple(self.lo, g), ceil_to_multiple(self.hi, g)) {
            (Some(lo), Some(hi)) => Some(Self { lo, hi }),
            _ => None,
        }
    }
}

/// The least multiple of `g` that is at least `v`. `None` if `g` is zero or the
/// result overflows.
#[inline]
const fn ceil_to_multiple(v: Count, g: Count) -> Option<Count> {
    if g == 0 {
        return None;
    }
    let rem = v % g;
    if rem == 0 {
        return Some(v);
    }
    v.checked_add(g - rem)
}

#[cfg(test)]
mod tests {
    use super::Interval;
    use crate::Count;

    #[test]
    fn an_inverted_interval_is_refused() {
        assert!(Interval::new(5, 4).is_none());
        assert!(Interval::new(4, 4).is_some());
    }

    #[test]
    fn addition_refuses_rather_than_wrapping() {
        let big = Interval::point(Count::MAX);
        assert!(big.checked_add(Interval::point(1)).is_none());
    }

    #[test]
    fn rounding_up_takes_the_ceiling() {
        // A sleep asking for 1 to 10 units on a 4-unit tick costs 4 to 12.
        let asked = Interval::new(1, 10).unwrap();
        let cost = asked.checked_round_up_to_multiple(4).unwrap();
        assert_eq!((cost.lo(), cost.hi()), (4, 12));
    }

    #[test]
    fn rounding_a_whole_multiple_changes_nothing() {
        let asked = Interval::new(8, 16).unwrap();
        let cost = asked.checked_round_up_to_multiple(4).unwrap();
        assert_eq!((cost.lo(), cost.hi()), (8, 16));
    }

    #[test]
    fn a_zero_granularity_is_refused() {
        assert!(Interval::point(3).checked_round_up_to_multiple(0).is_none());
    }
}
