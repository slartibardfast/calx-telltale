//! Exact rational frequencies, and the base a register composes in.
//!
//! A rate is held as a reduced fraction of hertz. The Programmable Interval
//! Timer runs at `105/88` MHz, which is `13125000/11` Hz, and no integer count
//! of hertz represents it. Storing a rounded frequency would put an error into
//! the one value that turns a count into a time, so nothing here rounds.

use crate::Count;

/// A frequency in hertz, as a reduced fraction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Rate {
    num: Count,
    den: Count,
}

impl Rate {
    /// A frequency of `num / den` hertz, reduced. `None` if either part is zero.
    #[must_use]
    pub const fn new(num: Count, den: Count) -> Option<Self> {
        if num == 0 || den == 0 {
            return None;
        }
        let g = gcd(num, den);
        Some(Self {
            num: num / g,
            den: den / g,
        })
    }

    /// A whole number of hertz.
    #[must_use]
    pub const fn hz(hz: Count) -> Option<Self> {
        Self::new(hz, 1)
    }

    #[inline]
    #[must_use]
    pub const fn num(self) -> Count {
        self.num
    }

    #[inline]
    #[must_use]
    pub const fn den(self) -> Count {
        self.den
    }

    /// This rate scaled by `factor`, exactly.
    ///
    /// A clock derived from another by a divider or a multiplier is this
    /// product, so a tree of derived clocks stays exact all the way down. The
    /// cross terms are reduced before multiplying, which keeps the product as
    /// small as it can be before the overflow check.
    #[must_use]
    pub const fn checked_mul(self, factor: Self) -> Option<Self> {
        let g1 = gcd(self.num, factor.den);
        let g2 = gcd(factor.num, self.den);
        match (
            (self.num / g1).checked_mul(factor.num / g2),
            (self.den / g2).checked_mul(factor.den / g1),
        ) {
            (Some(n), Some(d)) => Self::new(n, d),
            _ => None,
        }
    }
}

/// The greatest common divisor, by Euclid.
#[must_use]
pub const fn gcd(a: Count, b: Count) -> Count {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// The least common multiple, or `None` on overflow.
#[must_use]
pub const fn lcm(a: Count, b: Count) -> Option<Count> {
    if a == 0 || b == 0 {
        return None;
    }
    // a/gcd first, so the product is as small as it can be before the check.
    (a / gcd(a, b)).checked_mul(b)
}

/// Why a set of sources has no base this core can hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoBase {
    /// No sources were declared, so there is nothing to derive a base from.
    NoSources,
    /// The least common multiple of the declared numerators exceeds `Count`.
    TooWide,
}

/// The frequency a register composes in.
///
/// Every declared source's period is a whole number of base ticks, so
/// composition across sources is exact integer arithmetic and introduces no
/// rounding. This is `flicks` applied at the scope where the set of rates is
/// actually finite, which is one register rather than the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Base {
    hz: Count,
}

impl Base {
    /// The coarsest base in which every rate's period is a whole number of
    /// ticks.
    ///
    /// A rate `n/d` has period `d/n` seconds, which is `d * B / n` base ticks.
    /// That is a whole number for every reduced rate exactly when `n` divides
    /// `B`, so the least common multiple of the numerators is the answer. It is
    /// already at least as fine as the finest source, so taking the least of the
    /// common multiples costs no resolution.
    pub fn derive(rates: &[Rate]) -> Result<Self, NoBase> {
        let mut acc: Count = 1;
        if rates.is_empty() {
            return Err(NoBase::NoSources);
        }
        for r in rates {
            acc = lcm(acc, r.num).ok_or(NoBase::TooWide)?;
        }
        Ok(Self { hz: acc })
    }

    #[inline]
    #[must_use]
    pub const fn hz(self) -> Count {
        self.hz
    }

    /// How many base ticks one period of `rate` lasts, exactly.
    ///
    /// `None` when this base was not derived from a set including `rate`, or
    /// when the result exceeds `Count`.
    #[must_use]
    pub fn ticks_per_period(self, rate: Rate) -> Option<Count> {
        // d * B / n, exact whenever n divides B.
        let numer = rate.den.checked_mul(self.hz)?;
        if numer % rate.num != 0 {
            return None;
        }
        Some(numer / rate.num)
    }

    /// The span this base can represent in `Count` base ticks, in whole seconds.
    ///
    /// The finer the base, the shorter the span. A register whose windows do
    /// not fit inside this is told so rather than left to find out.
    #[must_use]
    pub const fn representable_seconds(self) -> Count {
        Count::MAX / self.hz
    }
}

#[cfg(test)]
mod tests {
    use super::{Base, NoBase, Rate};
    use crate::Count;

    /// The interval timer runs at a third of the colour burst: 105/88 MHz.
    fn pit() -> Rate {
        Rate::new(105_000_000, 88).unwrap()
    }

    /// An event timer at four times the colour burst.
    fn hpet() -> Rate {
        Rate::new(4 * 315_000_000, 88).unwrap()
    }

    #[test]
    fn a_rate_reduces() {
        assert_eq!((pit().num(), pit().den()), (13_125_000, 11));
        assert_eq!((hpet().num(), hpet().den()), (157_500_000, 11));
    }

    #[test]
    fn the_two_named_timers_share_an_exact_base() {
        let base = Base::derive(&[pit(), hpet()]).unwrap();
        assert_eq!(base.hz(), 157_500_000);
        // Both periods are whole numbers of base ticks, with nothing rounded.
        assert_eq!(base.ticks_per_period(pit()), Some(132));
        assert_eq!(base.ticks_per_period(hpet()), Some(11));
    }

    #[test]
    fn the_base_reports_what_it_can_represent() {
        let base = Base::derive(&[pit(), hpet()]).unwrap();
        // Thousands of years at this base, against windows measured in milliseconds.
        assert!(base.representable_seconds() > 100_000_000_000);

        // A femtosecond base is the unit an event timer reports its own period
        // in. At the stored width it still spans longer than anything a boot
        // sequence will ask of it, which is the point of holding the width
        // where it is: the finest base a register can want stays usable.
        let fine = Base::derive(&[Rate::hz(1_000_000_000_000_000).unwrap()]).unwrap();
        assert!(fine.representable_seconds() > 100_000_000_000_000_000_000_000);
    }

    #[test]
    fn an_empty_register_has_no_base() {
        assert_eq!(Base::derive(&[]), Err(NoBase::NoSources));
    }

    #[test]
    fn a_base_that_will_not_fit_is_refused_rather_than_wrapped() {
        let wide = [
            Rate::hz(Count::MAX / 3).unwrap(),
            Rate::hz(Count::MAX / 5 - 1).unwrap(),
        ];
        assert_eq!(Base::derive(&wide), Err(NoBase::TooWide));
    }

    #[test]
    fn a_rate_outside_the_derived_set_has_no_whole_period() {
        let base = Base::derive(&[pit()]).unwrap();
        // The base factors as 2^3 * 3 * 5^7 * 7, so 7 Hz divides it and has a
        // whole period even though it was never declared.
        assert_eq!(base.ticks_per_period(Rate::hz(7).unwrap()), Some(1_875_000));
        // 11 does not divide it, so there is no whole number of base ticks and
        // the tool says so rather than rounding.
        assert_eq!(base.ticks_per_period(Rate::hz(11).unwrap()), None);
    }
}
