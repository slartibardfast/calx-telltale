//! A number with a unit and a provenance, and the arithmetic that refuses.

use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::rate::Base;
use crate::source::{Source, SourceId, Span};
use crate::Count;

/// What a number counts.
///
/// `Ticks` and `Cycles` name the clock they were counted against rather than
/// carrying a rate, so two differently-clocked counts cannot be added and a
/// rate never enters the model as a bare integer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Unit {
    /// Loop passes. The safe default, with no conversion to time.
    Iterations,
    /// Peripheral accesses. Also no conversion to time.
    BusReads,
    /// Ticks of a declared clock.
    Ticks(SourceId),
    /// Cycles of a declared clock.
    Cycles(SourceId),
    /// Ticks of the register's derived base, in which composition across
    /// clocks is exact.
    Base,
    Nanos,
}

impl Unit {
    /// Whether this unit can reach a time at all.
    ///
    /// Iterations and bus reads cannot, by construction. A loop pass is not a
    /// duration, and no conversion exists to make it one.
    #[must_use]
    pub const fn is_temporal(self) -> bool {
        matches!(
            self,
            Unit::Ticks(_) | Unit::Cycles(_) | Unit::Base | Unit::Nanos
        )
    }
}

/// Why an operation could not be carried out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// Two quantities in different units, with no declared conversion.
    UnitMismatch { left: Unit, right: Unit },
    /// A count in a unit that has no path to a time.
    NotTemporal(Unit),
    /// The quantity does not belong to the clock it was offered against.
    WrongSource { expected: SourceId, found: Unit },
    /// The clock is not trustworthy across the whole window, so a conversion
    /// would be taken against a rate that changed underneath it.
    SourceInvalid(SourceId),
    /// The clock's period is not a whole number of base ticks, so this base was
    /// not derived from a set including it.
    NotInBase(SourceId),
    /// A tolerance of a million parts per million or more is not a tolerance.
    ToleranceTooWide(u32),
    /// The arithmetic left the width the core holds.
    Overflow,
}

/// An interval, a unit, and a provenance. The model holds no bare integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quantity {
    iv: Interval,
    unit: Unit,
    prov: Provenance,
}

impl Quantity {
    #[inline]
    #[must_use]
    pub const fn new(iv: Interval, unit: Unit, prov: Provenance) -> Self {
        Self { iv, unit, prov }
    }

    #[inline]
    #[must_use]
    pub const fn interval(self) -> Interval {
        self.iv
    }

    #[inline]
    #[must_use]
    pub const fn unit(self) -> Unit {
        self.unit
    }

    #[inline]
    #[must_use]
    pub const fn provenance(self) -> Provenance {
        self.prov
    }

    /// Sequential composition. Waits in sequence sum.
    pub fn checked_add(self, other: Self) -> Result<Self, Refusal> {
        if self.unit != other.unit {
            return Err(Refusal::UnitMismatch {
                left: self.unit,
                right: other.unit,
            });
        }
        let iv = self.iv.checked_add(other.iv).ok_or(Refusal::Overflow)?;
        Ok(Self {
            iv,
            unit: self.unit,
            prov: self.prov.join(other.prov),
        })
    }

    /// Branching. Alternatives take the more expensive one.
    pub fn checked_max(self, other: Self) -> Result<Self, Refusal> {
        if self.unit != other.unit {
            return Err(Refusal::UnitMismatch {
                left: self.unit,
                right: other.unit,
            });
        }
        Ok(Self {
            iv: self.iv.max(other.iv),
            unit: self.unit,
            prov: self.prov.join(other.prov),
        })
    }

    /// Repetition. A loop body repeated a declared number of times.
    pub fn checked_repeat(self, times: Interval, times_prov: Provenance) -> Result<Self, Refusal> {
        let iv = self.iv.checked_mul(times).ok_or(Refusal::Overflow)?;
        Ok(Self {
            iv,
            unit: self.unit,
            prov: self.prov.join(times_prov),
        })
    }

    /// Convert a count of this clock's ticks into base ticks, exactly.
    ///
    /// This is the path composition should take. Where the clock has no
    /// tolerance the conversion introduces no rounding whatsoever, so a bound
    /// built across several clocks widens only for the tolerances actually
    /// declared rather than for every division along the way.
    ///
    /// The window is required because a clock is trustworthy over a span. A
    /// conversion across a span the clock is valid for only part of is refused
    /// rather than taken against a stale rate.
    pub fn to_base(self, src: &Source, base: Base, window: Span) -> Result<Self, Refusal> {
        match self.unit {
            Unit::Ticks(id) | Unit::Cycles(id) if id == src.id => {}
            other => {
                return Err(Refusal::WrongSource {
                    expected: src.id,
                    found: other,
                })
            }
        }
        if !src.valid.covers(window) {
            return Err(Refusal::SourceInvalid(src.id));
        }
        if src.tolerance_ppm >= 1_000_000 {
            return Err(Refusal::ToleranceTooWide(src.tolerance_ppm));
        }
        let per_period = base
            .ticks_per_period(src.nominal)
            .ok_or(Refusal::NotInBase(src.id))?;

        // A slower clock has a longer period, so the tolerance maps outward:
        // the low end takes the fastest permitted clock and the high end the
        // slowest. With no tolerance declared this is exact.
        let (lo_per, hi_per) = if src.tolerance_ppm == 0 {
            (per_period, per_period)
        } else {
            let ppm = src.tolerance_ppm as Count;
            let m: Count = 1_000_000;
            let scaled = per_period.checked_mul(m).ok_or(Refusal::Overflow)?;
            (div_floor(scaled, m + ppm), div_ceil(scaled, m - ppm))
        };

        let lo = self.iv.lo().checked_mul(lo_per).ok_or(Refusal::Overflow)?;
        let hi = self.iv.hi().checked_mul(hi_per).ok_or(Refusal::Overflow)?;
        let iv = Interval::new(lo, hi).ok_or(Refusal::Overflow)?;

        Ok(Self {
            iv,
            unit: Unit::Base,
            // The rate's provenance reaches every time derived from it. A
            // frequency somebody guessed makes this Assumed with nothing
            // further wired up.
            prov: self.prov.join(src.nominal_prov),
        })
    }

    /// Convert base ticks into nanoseconds, rounding outward.
    ///
    /// This is a presentation step. Composition happens in base ticks, and the
    /// division that loses exactness is deferred to here.
    pub fn base_to_nanos(self, base: Base) -> Result<Self, Refusal> {
        if self.unit != Unit::Base {
            return Err(Refusal::UnitMismatch {
                left: self.unit,
                right: Unit::Base,
            });
        }
        let b = base.hz();
        const NANOS_PER_SEC: Count = 1_000_000_000;
        let lo = div_floor(
            self.iv
                .lo()
                .checked_mul(NANOS_PER_SEC)
                .ok_or(Refusal::Overflow)?,
            b,
        );
        let hi = div_ceil(
            self.iv
                .hi()
                .checked_mul(NANOS_PER_SEC)
                .ok_or(Refusal::Overflow)?,
            b,
        );
        let iv = Interval::new(lo, hi).ok_or(Refusal::Overflow)?;
        Ok(Self {
            iv,
            unit: Unit::Nanos,
            prov: self.prov,
        })
    }
}

#[inline]
const fn div_floor(a: Count, b: Count) -> Count {
    a / b
}

/// Ceiling division without the overflow that `(a + b - 1) / b` invites.
#[inline]
const fn div_ceil(a: Count, b: Count) -> Count {
    let q = a / b;
    if a % b == 0 {
        q
    } else {
        q + 1
    }
}

#[cfg(test)]
mod tests {
    use super::{Quantity, Refusal, Unit};
    use crate::interval::Interval;
    use crate::provenance::Provenance::{Assumed, Derived, Extracted, Measured};
    use crate::rate::{Base, Rate};
    use crate::source::{Source, SourceId, Span, Validity};
    use crate::Count;

    fn pit_rate() -> Rate {
        Rate::new(105_000_000, 88).unwrap()
    }
    fn hpet_rate() -> Rate {
        Rate::new(4 * 315_000_000, 88).unwrap()
    }

    fn source(
        id: u16,
        rate: Rate,
        ppm: u32,
        valid: Validity,
        prov: crate::provenance::Provenance,
    ) -> Source {
        Source {
            id: SourceId(id),
            origin: crate::source::Origin::Root,
            nominal: rate,
            nominal_prov: prov,
            tolerance_ppm: ppm,
            width_bits: 32,
            read_cost: Quantity::new(Interval::point(1), Unit::BusReads, Measured),
            valid,
        }
    }

    fn q(v: Count, unit: Unit, p: crate::provenance::Provenance) -> Quantity {
        Quantity::new(Interval::point(v), unit, p)
    }

    #[test]
    fn adding_across_units_is_refused() {
        let a = q(1, Unit::Iterations, Derived);
        let b = q(1, Unit::BusReads, Derived);
        assert!(matches!(
            a.checked_add(b),
            Err(Refusal::UnitMismatch { .. })
        ));
    }

    #[test]
    fn two_clocks_are_not_the_same_unit() {
        let a = q(1, Unit::Ticks(SourceId(0)), Derived);
        let b = q(1, Unit::Ticks(SourceId(1)), Derived);
        assert!(matches!(
            a.checked_add(b),
            Err(Refusal::UnitMismatch { .. })
        ));
    }

    #[test]
    fn one_assumed_input_makes_the_sum_assumed() {
        let a = q(1, Unit::Iterations, Derived);
        let b = q(1, Unit::Iterations, Assumed);
        assert_eq!(a.checked_add(b).unwrap().provenance(), Assumed);
    }

    #[test]
    fn the_two_named_timers_compose_exactly_in_the_base() {
        let base = Base::derive(&[pit_rate(), hpet_rate()]).unwrap();
        let pit = source(0, pit_rate(), 0, Validity::Always, Extracted);
        let hpet = source(1, hpet_rate(), 0, Validity::Always, Extracted);
        let w = Span::new(0, 100).unwrap();

        // 10 interval-timer ticks and 10 event-timer ticks, which cannot be
        // added to each other until both are in the base.
        let a = q(10, Unit::Ticks(SourceId(0)), Extracted);
        let b = q(10, Unit::Ticks(SourceId(1)), Extracted);
        assert!(a.checked_add(b).is_err());

        let a = a.to_base(&pit, base, w).unwrap();
        let b = b.to_base(&hpet, base, w).unwrap();
        assert_eq!(a.interval(), Interval::point(1320)); // 10 * 132, exact
        assert_eq!(b.interval(), Interval::point(110)); // 10 * 11, exact

        let total = a.checked_add(b).unwrap();
        assert_eq!(total.interval(), Interval::point(1430));
        assert_eq!(total.unit(), Unit::Base);
    }

    #[test]
    fn a_guessed_frequency_makes_every_time_derived_from_it_assumed() {
        let base = Base::derive(&[pit_rate()]).unwrap();
        // The count is swept and solid; the clock's rate is a guess.
        let pit = source(0, pit_rate(), 0, Validity::Always, Assumed);
        let w = Span::new(0, 10).unwrap();
        let converted = q(100, Unit::Ticks(SourceId(0)), Derived)
            .to_base(&pit, base, w)
            .unwrap();
        assert_eq!(converted.provenance(), Assumed);
    }

    #[test]
    fn a_clock_being_reconfigured_refuses_rather_than_guessing() {
        let base = Base::derive(&[pit_rate()]).unwrap();
        // Trustworthy only after the clock tree settles at position 50.
        let pit = source(
            0,
            pit_rate(),
            0,
            Validity::Over(Span::new(50, 1000).unwrap()),
            Extracted,
        );
        let across_reconfig = Span::new(0, 100).unwrap();
        let after = Span::new(60, 100).unwrap();

        let count = q(100, Unit::Ticks(SourceId(0)), Extracted);
        assert_eq!(
            count.to_base(&pit, base, across_reconfig),
            Err(Refusal::SourceInvalid(SourceId(0)))
        );
        assert!(count.to_base(&pit, base, after).is_ok());
    }

    #[test]
    fn a_tolerance_widens_outward_and_never_narrows() {
        let base = Base::derive(&[pit_rate()]).unwrap();
        let exact = source(0, pit_rate(), 0, Validity::Always, Extracted);
        let loose = source(0, pit_rate(), 10_000, Validity::Always, Extracted); // 1%
        let w = Span::new(0, 10).unwrap();
        let count = q(1000, Unit::Ticks(SourceId(0)), Extracted);

        let tight = count.to_base(&exact, base, w).unwrap().interval();
        let wide = count.to_base(&loose, base, w).unwrap().interval();
        assert!(wide.lo() <= tight.lo(), "the low end may only move down");
        assert!(wide.hi() >= tight.hi(), "the high end may only move up");
        assert!(wide.lo() < tight.lo() && wide.hi() > tight.hi());
    }

    #[test]
    fn nanoseconds_are_a_presentation_step_and_round_outward() {
        let base = Base::derive(&[hpet_rate()]).unwrap();
        // One base tick at 157500000 Hz is 6.349... ns, so it brackets.
        let one = Quantity::new(Interval::point(1), Unit::Base, Extracted);
        let ns = one.base_to_nanos(base).unwrap();
        assert_eq!(ns.unit(), Unit::Nanos);
        assert_eq!((ns.interval().lo(), ns.interval().hi()), (6, 7));
    }

    #[test]
    fn a_count_that_cannot_reach_a_time_says_so() {
        assert!(!Unit::Iterations.is_temporal());
        assert!(!Unit::BusReads.is_temporal());
        assert!(Unit::Base.is_temporal());
    }
}
