//! Declared clocks, when they can be trusted, and the delays that ride on them.
//!
//! The core knows no particular clock. An interval timer, an event timer and a
//! part-specific timer an adopter alone has are all declarations of the same
//! shape, referenced by identifier.

use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::quantity::Quantity;
use crate::rate::Rate;
use crate::Count;

/// A declared clock, by index into the register's source table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct SourceId(pub u16);

/// A position range on the register's abstract timeline.
///
/// Windows, deadlines and source validity are all spans, so overlap between
/// them is one comparison rather than three conventions.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Span {
    from: Count,
    to: Count,
}

impl Span {
    #[must_use]
    pub const fn new(from: Count, to: Count) -> Option<Self> {
        if from <= to {
            Some(Self { from, to })
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub const fn from(self) -> Count {
        self.from
    }

    #[inline]
    #[must_use]
    pub const fn to(self) -> Count {
        self.to
    }

    /// Whether `inner` lies wholly inside this span.
    #[inline]
    #[must_use]
    pub const fn covers(self, inner: Span) -> bool {
        self.from <= inner.from && inner.to <= self.to
    }
}

/// When a clock can be trusted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Validity {
    /// Trustworthy throughout.
    Always,
    /// Trustworthy only across this span. Outside it the clock is running at a
    /// rate the register does not know, which is the clock-tree case.
    Over(Span),
}

impl Validity {
    /// Whether the clock can be trusted across the whole of `span`.
    ///
    /// Partial cover is not enough. A conversion taken across a span the clock
    /// is trustworthy for only part of is a conversion against a rate that
    /// changed underneath it.
    #[inline]
    #[must_use]
    pub const fn covers(self, span: Span) -> bool {
        match self {
            Validity::Always => true,
            Validity::Over(v) => v.covers(span),
        }
    }
}

/// Where a clock's rate comes from.
///
/// Real parts rarely have independent clocks. A core clock comes off a
/// synthesiser, a counter runs at the core clock, a kernel tick comes off a
/// compare on that counter, and a sleep primitive rides the tick. That is one
/// root with everything hanging beneath it, and it has two consequences the
/// flat view misses: the rates are exact multiples of one another, and
/// reconfiguring the root takes every descendant with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Origin {
    /// An independent input: a crystal, an external reference, a synthesiser
    /// the register treats as given.
    Root,
    /// Derived from another clock by an exact ratio. A divide-by-seven is a
    /// ratio of `1/7`, and a multiplier is a ratio greater than one.
    Derived { parent: SourceId, ratio: Rate },
}

/// A declared clock.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub id: SourceId,
    /// Where this clock's rate comes from.
    pub origin: Origin,
    /// The nominal frequency, exactly. For a derived clock this is the
    /// frequency the declaration claims, and [`SourceTable::resolve`] checks it
    /// against what the parent and the ratio actually give.
    pub nominal: Rate,
    /// Where the frequency came from. This reaches every time derived from it.
    pub nominal_prov: Provenance,
    /// Crystal tolerance in parts per million, which widens every conversion.
    pub tolerance_ppm: u32,
    /// The counter's width in bits. A counter that wraps inside a window under
    /// study is a hazard rather than a detail.
    pub width_bits: u8,
    /// What one read of this clock costs. Reading a clock is itself work, and
    /// on these parts it can be a bus access.
    pub read_cost: Quantity,
    pub valid: Validity,
}

impl Source {
    /// The largest count this clock's counter holds before it wraps.
    #[must_use]
    pub const fn counter_span(&self) -> Count {
        if self.width_bits >= 128 {
            Count::MAX
        } else {
            ((1 as Count) << self.width_bits) - 1
        }
    }

    /// Whether `count` ticks fit in the counter without wrapping.
    #[must_use]
    pub const fn holds(&self, count: Count) -> bool {
        count <= self.counter_span()
    }
}

/// Why a declared tree of clocks does not hold together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeFault {
    /// A clock names a parent the register does not declare.
    UnknownParent { child: SourceId, parent: SourceId },
    /// The parent chain does not reach a root.
    Cycle(SourceId),
    /// The parent's rate times the declared ratio leaves the width the core
    /// holds.
    RateOverflow(SourceId),
    /// The declared frequency disagrees with the parent and the ratio. One of
    /// the two is wrong, and the register has to say which.
    Disagrees {
        child: SourceId,
        declared: Rate,
        implied: Rate,
    },
    /// No clock with that identifier is declared.
    Unknown(SourceId),
}

/// The register's declared clocks, as a tree.
#[derive(Clone, Debug, Default)]
pub struct SourceTable {
    sources: Vec<Source>,
}

/// A depth beyond which a parent chain is treated as a cycle. Real trees are a
/// handful of levels; this is a backstop rather than a limit anyone meets.
const MAX_DEPTH: usize = 32;

impl SourceTable {
    #[must_use]
    pub fn new(sources: Vec<Source>) -> Self {
        Self { sources }
    }

    #[must_use]
    pub fn get(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    fn require(&self, id: SourceId) -> Result<&Source, TreeFault> {
        self.get(id).ok_or(TreeFault::Unknown(id))
    }

    /// The frequency implied by walking to the root, and the check that the
    /// declared frequency agrees with it.
    pub fn resolve(&self, id: SourceId) -> Result<Rate, TreeFault> {
        let mut cursor = self.require(id)?;
        let mut ratio = Rate::hz(1).expect("one is a rate");
        for _ in 0..MAX_DEPTH {
            match cursor.origin {
                Origin::Root => {
                    let implied = cursor
                        .nominal
                        .checked_mul(ratio)
                        .ok_or(TreeFault::RateOverflow(id))?;
                    let declared = self.require(id)?.nominal;
                    return if implied == declared {
                        Ok(implied)
                    } else {
                        Err(TreeFault::Disagrees {
                            child: id,
                            declared,
                            implied,
                        })
                    };
                }
                Origin::Derived { parent, ratio: r } => {
                    ratio = ratio.checked_mul(r).ok_or(TreeFault::RateOverflow(id))?;
                    cursor = self.get(parent).ok_or(TreeFault::UnknownParent {
                        child: cursor.id,
                        parent,
                    })?;
                }
            }
        }
        Err(TreeFault::Cycle(id))
    }

    /// Whether this clock can be trusted across the whole of `span`, taking its
    /// ancestors into account.
    ///
    /// A clock is no more trustworthy than the clock it hangs off. Where the
    /// root is being reconfigured, nothing beneath it can measure anything,
    /// including the reconfiguration.
    pub fn valid_over(&self, id: SourceId, span: Span) -> Result<bool, TreeFault> {
        let mut cursor = self.require(id)?;
        for _ in 0..MAX_DEPTH {
            if !cursor.valid.covers(span) {
                return Ok(false);
            }
            match cursor.origin {
                Origin::Root => return Ok(true),
                Origin::Derived { parent, .. } => {
                    cursor = self.get(parent).ok_or(TreeFault::UnknownParent {
                        child: cursor.id,
                        parent,
                    })?;
                }
            }
        }
        Err(TreeFault::Cycle(id))
    }

    /// The root this clock hangs off.
    pub fn root_of(&self, id: SourceId) -> Result<SourceId, TreeFault> {
        let mut cursor = self.require(id)?;
        for _ in 0..MAX_DEPTH {
            match cursor.origin {
                Origin::Root => return Ok(cursor.id),
                Origin::Derived { parent, .. } => {
                    cursor = self.get(parent).ok_or(TreeFault::UnknownParent {
                        child: cursor.id,
                        parent,
                    })?;
                }
            }
        }
        Err(TreeFault::Cycle(id))
    }

    /// Whether two clocks fail together.
    ///
    /// Clocks sharing a root are one correlation class. Worst-casing them
    /// independently understates a composition that rides both, because the
    /// thing that moves one moves the other.
    pub fn correlated(&self, a: SourceId, b: SourceId) -> Result<bool, TreeFault> {
        Ok(self.root_of(a)? == self.root_of(b)?)
    }
}

/// How a sleep primitive turns a requested delay into a whole number of ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Rounding {
    /// Rounds up to a whole tick. The worst case takes the ceiling.
    Up,
    /// Returns as asked, to the resolution of the clock.
    Exact,
}

/// What a wait does between polls, and therefore which clock it rides on.
#[derive(Clone, Copy, Debug)]
pub enum Delay {
    /// A bare spin. Nothing here measures time, so the cost stays a count.
    None,
    /// A calibrated busy loop, with the clock the calibration was taken
    /// against. Where that clock is absent the loop's period is unknown.
    Spin {
        per_iter: Quantity,
        calibrated_against: Option<SourceId>,
    },
    /// A timer-backed sleep.
    Sleep {
        source: SourceId,
        /// One tick, in the source's own ticks. A primitive that rounds up to a
        /// whole tick costs up to a full granularity more than it was asked
        /// for, which over thousands of calls is the difference between a bound
        /// and a wish.
        granularity: Interval,
        rounding: Rounding,
    },
}

impl Delay {
    /// The clock this delay rides on, where it rides on one.
    #[must_use]
    pub const fn source(&self) -> Option<SourceId> {
        match self {
            Delay::None => None,
            Delay::Spin {
                calibrated_against, ..
            } => *calibrated_against,
            Delay::Sleep { source, .. } => Some(*source),
        }
    }

    /// The cost of asking this delay for `asked`, in the unit `asked` is in.
    ///
    /// A sleep that rounds up takes the ceiling at both endpoints, which is
    /// exact rather than conservative, because rounding up is monotone.
    #[must_use]
    pub fn cost_of(&self, asked: Interval) -> Option<Interval> {
        match self {
            Delay::None | Delay::Spin { .. } => Some(asked),
            Delay::Sleep {
                granularity,
                rounding,
                ..
            } => match rounding {
                Rounding::Exact => Some(asked),
                // The widest tick gives the largest ceiling, so it bounds.
                Rounding::Up => asked.checked_round_up_to_multiple(granularity.hi()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Delay, Origin, Rounding, Source, SourceId, SourceTable, Span, TreeFault, Validity,
    };
    use crate::interval::Interval;
    use crate::provenance::Provenance;
    use crate::quantity::{Quantity, Unit};
    use crate::rate::Rate;
    use crate::Count;

    fn a_source() -> Source {
        Source {
            id: SourceId(0),
            origin: Origin::Root,
            nominal: Rate::new(105_000_000, 88).unwrap(),
            nominal_prov: Provenance::Extracted,
            tolerance_ppm: 100,
            width_bits: 16,
            read_cost: Quantity::new(Interval::point(1), Unit::BusReads, Provenance::Measured),
            valid: Validity::Always,
        }
    }

    #[test]
    fn a_sixteen_bit_counter_wraps_where_it_says_it_does() {
        let s = a_source();
        assert_eq!(s.counter_span(), 65_535);
        assert!(s.holds(65_535));
        assert!(!s.holds(65_536));
    }

    #[test]
    fn validity_needs_the_whole_window_not_an_overlap() {
        let v = Validity::Over(Span::new(10, 20).unwrap());
        assert!(v.covers(Span::new(12, 18).unwrap()));
        assert!(v.covers(Span::new(10, 20).unwrap()));
        // Reaches past the end, so the rate changed underneath it.
        assert!(!v.covers(Span::new(15, 25).unwrap()));
        assert!(!v.covers(Span::new(5, 15).unwrap()));
    }

    #[test]
    fn a_sleep_that_rounds_up_costs_up_to_a_whole_tick_more() {
        let d = Delay::Sleep {
            source: SourceId(0),
            granularity: Interval::point(10),
            rounding: Rounding::Up,
        };
        // Asking for 1 to 10 costs a whole tick either way.
        let cost = d.cost_of(Interval::new(1, 10).unwrap()).unwrap();
        assert_eq!((cost.lo(), cost.hi()), (10, 10));
        // Asking for 11 costs two.
        let cost = d.cost_of(Interval::point(11)).unwrap();
        assert_eq!((cost.lo(), cost.hi()), (20, 20));
    }

    #[test]
    fn a_bare_spin_rides_on_no_clock() {
        assert_eq!(Delay::None.source(), None);
    }

    // A part with one root and everything hanging off it. A synthesiser feeds
    // the core clock, the only hardware counter runs at the core rate, and a
    // kernel tick comes off a compare on that counter. Frequencies here are
    // round numbers chosen to make the arithmetic legible.
    const CORE: SourceId = SourceId(0);
    const COUNTER: SourceId = SourceId(1);
    const TICK: SourceId = SourceId(2);

    fn one_root_part(core_valid: Validity) -> SourceTable {
        let cost = Quantity::new(Interval::point(1), Unit::BusReads, Provenance::Measured);
        let mk = |id, origin, nominal, width, valid| Source {
            id,
            origin,
            nominal,
            nominal_prov: Provenance::Extracted,
            tolerance_ppm: 0,
            width_bits: width,
            read_cost: cost,
            valid,
        };
        SourceTable::new(vec![
            mk(
                CORE,
                Origin::Root,
                Rate::hz(400_000_000).unwrap(),
                0,
                core_valid,
            ),
            mk(
                COUNTER,
                Origin::Derived {
                    parent: CORE,
                    ratio: Rate::hz(1).unwrap(),
                },
                Rate::hz(400_000_000).unwrap(),
                32,
                Validity::Always,
            ),
            mk(
                TICK,
                Origin::Derived {
                    parent: COUNTER,
                    ratio: Rate::new(1, 400_000).unwrap(),
                },
                Rate::hz(1_000).unwrap(),
                64,
                Validity::Always,
            ),
        ])
    }

    #[test]
    fn a_derived_rate_resolves_through_the_tree() {
        let t = one_root_part(Validity::Always);
        assert_eq!(t.resolve(CORE).unwrap(), Rate::hz(400_000_000).unwrap());
        assert_eq!(t.resolve(COUNTER).unwrap(), Rate::hz(400_000_000).unwrap());
        // Two levels down, and exact.
        assert_eq!(t.resolve(TICK).unwrap(), Rate::hz(1_000).unwrap());
    }

    #[test]
    fn a_declared_rate_that_disagrees_with_its_parent_is_caught() {
        let mut t = one_root_part(Validity::Always);
        let bad = Source {
            nominal: Rate::hz(100).unwrap(), // the tree implies 1000
            ..*t.get(TICK).unwrap()
        };
        t = SourceTable::new(vec![*t.get(CORE).unwrap(), *t.get(COUNTER).unwrap(), bad]);
        assert!(matches!(t.resolve(TICK), Err(TreeFault::Disagrees { .. })));
    }

    #[test]
    fn reconfiguring_the_root_takes_every_clock_beneath_it() {
        // The root settles only at position 50, which is the clock tree being
        // reconfigured while the part runs on it.
        let t = one_root_part(Validity::Over(Span::new(50, 1_000).unwrap()));
        let during = Span::new(0, 100).unwrap();
        let after = Span::new(60, 100).unwrap();

        // Nothing on the part can measure the reconfiguration, including the
        // clocks that declare themselves always valid.
        assert!(!t.valid_over(CORE, during).unwrap());
        assert!(!t.valid_over(COUNTER, during).unwrap());
        assert!(!t.valid_over(TICK, during).unwrap());

        assert!(t.valid_over(TICK, after).unwrap());
    }

    #[test]
    fn clocks_sharing_a_root_fail_together() {
        let t = one_root_part(Validity::Always);
        assert!(t.correlated(TICK, COUNTER).unwrap());
        assert_eq!(t.root_of(TICK).unwrap(), CORE);
    }

    #[test]
    fn a_kilohertz_tick_that_rounds_up_puts_a_floor_under_every_sleep() {
        // A sleep primitive on a 1 kHz tick has a granularity of one
        // millisecond. Asking it for a microsecond gets a millisecond, because
        // there is nothing smaller it can return.
        let sleep = Delay::Sleep {
            source: TICK,
            granularity: Interval::point(1_000_000), // one tick, in nanoseconds
            rounding: Rounding::Up,
        };
        let asked_one_microsecond = Interval::point(1_000);
        let cost = sleep.cost_of(asked_one_microsecond).unwrap();
        assert_eq!(cost, Interval::point(1_000_000));
        // Three orders of magnitude, and structural rather than a defect.
        assert_eq!(cost.hi() / asked_one_microsecond.hi(), 1_000);
    }

    #[test]
    fn a_thirty_two_bit_counter_bounds_the_window_it_can_time() {
        let t = one_root_part(Validity::Always);
        let counter = t.get(COUNTER).unwrap();
        // 2^32 ticks at 400 MHz is about 10.7 seconds, so a ten-second window
        // fits with headroom and an eleven-second one does not.
        let ten_seconds: Count = 10 * 400_000_000;
        let eleven_seconds: Count = 11 * 400_000_000;
        assert!(counter.holds(ten_seconds));
        assert!(!counter.holds(eleven_seconds));
        let headroom_ticks = counter.counter_span() - ten_seconds;
        assert!(headroom_ticks < 400_000_000, "under a second of headroom");
    }
}
