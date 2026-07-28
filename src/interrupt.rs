//! Declared interrupts, and what a blackout window does to them.
//!
//! A register measures how long interrupts are off. On its own that says
//! nothing about whether any interrupt missed anything, which needs the arrival
//! side declared as well. Two questions follow once it is.
//!
//! **Latency** asks whether the blackout outlasts a deadline. It is the question
//! people ask first.
//!
//! **Overrun** asks whether more arrivals land during the blackout than the
//! buffer holds. It is the question that bites, because a handler that drops
//! entries when its ring fills and only logs the fact fails without ever missing
//! a latency figure anyone was watching.
//!
//! Both can be unanswerable, and that is a verdict rather than an error. A
//! window whose cost is denominated in loop passes cannot be compared against a
//! deadline in nanoseconds, and saying so is worth more than a number obtained
//! by assuming a rate.

use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::quantity::{Quantity, Unit};
use crate::source::{SourceId, Span, Validity};
use crate::Count;

/// A declared interrupt, by index into the register's interrupt table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct InterruptId(pub u16);

/// How often an interrupt can arrive.
#[derive(Clone, Copy, Debug)]
pub enum Arrival {
    /// Periodic, clocked by a declared source. A timer tick is this.
    Periodic { source: SourceId, period: Quantity },
    /// Bounded only: no two arrivals land closer together than this. A line fed
    /// by an external peer is this, where the peer's rate is known and its
    /// phase is not.
    MinInterarrival(Quantity),
}

impl Arrival {
    /// The shortest gap between two arrivals, which is what a blackout has to
    /// be measured against.
    #[must_use]
    pub const fn shortest_gap(&self) -> Quantity {
        match self {
            Arrival::Periodic { period, .. } => *period,
            Arrival::MinInterarrival(q) => *q,
        }
    }

    /// The clock this arrival is counted against, where it has one.
    #[must_use]
    pub const fn source(&self) -> Option<SourceId> {
        match self {
            Arrival::Periodic { source, .. } => Some(*source),
            Arrival::MinInterarrival(_) => None,
        }
    }
}

/// What becomes of an arrival the buffer had no room for.
///
/// Recorded so that an overrun verdict can say what was lost rather than only
/// that something was. The register holds the prose; this is the part the core
/// reasons about.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Consequence {
    /// Lost with no record kept. The worst of the three, because nothing
    /// downstream can tell that it happened.
    LostSilently,
    /// Lost, and the loss recorded somewhere a reader can find it.
    LostAndLogged,
    /// The source retries, so the arrival is delayed rather than lost.
    Retried,
}

/// A latency bound, and the span it is in force over.
///
/// Arming is first-class because a watchdog is typically enabled part-way
/// through a boot. Everything before that point has no deadline at all, which
/// is a different situation from having one and a worse one, and a tool that
/// reported it as a pass would be saying the opposite of what is true.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Deadline {
    pub budget: Quantity,
    pub armed: Validity,
}

/// A declared interrupt.
#[derive(Clone, Copy, Debug)]
pub struct Interrupt {
    pub id: InterruptId,
    pub arrival: Arrival,
    /// How long the handler runs. Declared rather than derived: this tool does
    /// not compute a worst-case execution time, and it does not pretend the
    /// quantity is absent either. A cost somebody guessed makes every verdict
    /// resting on it a guess.
    pub cost: Quantity,
    /// Priority, numbered the way the hardware numbers it: a lower number
    /// preempts a higher one.
    pub priority: u8,
    /// The latency this interrupt can absorb before it has missed, and when
    /// that bound is in force. An interrupt with none declared can still be
    /// judged on overrun.
    pub deadline: Option<Deadline>,
    /// How late a release may be against its declared gap. A source that is
    /// periodic in the long run can still bunch, and a burst delivers more
    /// arrivals into a window than an even spacing would.
    pub jitter: Option<Quantity>,
    /// Whether the handler re-enables interrupts inside itself. One that does
    /// can be preempted by its own priority level, which the flat model would
    /// otherwise miss.
    pub reenables: bool,
    /// How many arrivals can queue before one is lost.
    pub depth: u32,
    /// What a dropped arrival costs.
    pub on_drop: Consequence,
}

impl Interrupt {
    /// Whether this interrupt preempts `other`.
    #[must_use]
    pub const fn preempts(&self, other: &Interrupt) -> bool {
        self.priority < other.priority
    }
}

/// What a comparison was missing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Missing {
    /// The blackout is counted in something that has no path to a time at all,
    /// which is the honest state of a window measured while its clock was being
    /// reconfigured.
    NoPathToTime(Unit),
    /// Both sides reach a time, and no declared conversion connects them.
    NoConversion { from: Unit, to: Unit },
    /// The interrupt declares no deadline, so there is no latency to judge.
    NoDeadline,
    /// An arrival gap of zero bounds nothing: an interrupt that can arrive
    /// arbitrarily fast overruns any buffer.
    UnboundedArrivals,
    /// The arithmetic left the width the core holds.
    Overflow,
}

/// The result of one comparison.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// The bound holds across the whole declared range.
    Met,
    /// The bound can be breached. A worst-case tool reports the breach as soon
    /// as one is reachable, rather than waiting for it to be certain.
    Missed,
    /// No deadline is in force across the span under study. This is not a pass:
    /// an unbounded stretch is worse than a bounded one, and reporting it as
    /// met would say the opposite of what is true.
    Unarmed,
    /// The comparison cannot be made, and this names what is missing.
    Unanswerable(Missing),
}

impl Verdict {
    #[must_use]
    pub const fn is_answerable(self) -> bool {
        !matches!(self, Verdict::Unanswerable(_))
    }

    /// Whether this verdict is a clean pass. An unarmed span is not one.
    #[must_use]
    pub const fn is_met(self) -> bool {
        matches!(self, Verdict::Met)
    }
}

/// A verdict, what it rests on, and what was measured to reach it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Judgement {
    pub verdict: Verdict,
    /// The weakest provenance among everything the verdict derives from. A
    /// verdict resting on a guessed arrival rate says so.
    pub provenance: Provenance,
    /// The arrival count for an overrun judgement, or the blackout for a
    /// latency one. Absent where the comparison could not be made.
    pub measured: Option<Quantity>,
}

impl Judgement {
    const fn withheld(missing: Missing, provenance: Provenance) -> Self {
        Self {
            verdict: Verdict::Unanswerable(missing),
            provenance,
            measured: None,
        }
    }
}

/// Why two units will not meet.
///
/// A unit with no path to a time is a different finding from two temporal units
/// with no conversion between them, and an operator acts on them differently.
/// One decision point keeps the two apart everywhere.
pub(crate) fn mismatch(from: Unit, to: Unit) -> Missing {
    if from.is_temporal() {
        Missing::NoConversion { from, to }
    } else {
        Missing::NoPathToTime(from)
    }
}

/// How many arrivals can land inside a window.
///
/// A window of length `b` admits at most `floor(b / gap) + 1` arrivals, because
/// one can land at the instant the window opens and the rest follow no faster
/// than the shortest gap. The bound uses the longest blackout against the
/// shortest gap, which is the worst pairing.
pub fn arrivals_during(blackout: Quantity, gap: Quantity) -> Result<Quantity, Missing> {
    if blackout.unit() != gap.unit() {
        return Err(mismatch(blackout.unit(), gap.unit()));
    }
    if gap.interval().lo() == 0 {
        return Err(Missing::UnboundedArrivals);
    }

    let most = blackout
        .interval()
        .hi()
        .checked_div(gap.interval().lo())
        .and_then(|n| n.checked_add(1))
        .ok_or(Missing::Overflow)?;
    // The fewest is the shortest blackout against the longest gap, and it can
    // be none at all where the window closes before a second arrival is due.
    let fewest = blackout
        .interval()
        .lo()
        .checked_div(gap.interval().hi())
        .ok_or(Missing::Overflow)?;

    let iv = Interval::new(fewest.min(most), most).ok_or(Missing::Overflow)?;
    Ok(Quantity::new(
        iv,
        Unit::Iterations,
        blackout.provenance().join(gap.provenance()),
    ))
}

impl Interrupt {
    /// Whether this blackout can outlast the declared deadline.
    ///
    /// The comparison is made against the tightest end of the deadline and the
    /// longest end of the blackout, so a deadline that can be breached is
    /// reported as breached.
    #[must_use]
    pub fn latency(&self, blackout: Quantity, over: Span) -> Judgement {
        let Some(deadline) = self.deadline else {
            return Judgement::withheld(Missing::NoDeadline, blackout.provenance());
        };
        let bound = deadline.budget;
        // A bound not in force across the whole span bounds nothing across it.
        if !deadline.armed.covers(over) {
            return Judgement {
                verdict: Verdict::Unarmed,
                provenance: blackout.provenance().join(bound.provenance()),
                measured: Some(blackout),
            };
        }
        if blackout.unit() != bound.unit() {
            return Judgement::withheld(
                mismatch(blackout.unit(), bound.unit()),
                blackout.provenance().join(bound.provenance()),
            );
        }
        let provenance = blackout.provenance().join(bound.provenance());
        let verdict = if blackout.interval().hi() > bound.interval().lo() {
            Verdict::Missed
        } else {
            Verdict::Met
        };
        Judgement {
            verdict,
            provenance,
            measured: Some(blackout),
        }
    }

    /// Whether this blackout can deliver more arrivals than the buffer holds.
    #[must_use]
    pub fn overrun(&self, blackout: Quantity) -> Judgement {
        let gap = self.arrival.shortest_gap();
        // Release jitter widens the window arrivals can bunch into: a source
        // periodic in the long run still delivers a burst if a release is late.
        let effective = match self.jitter {
            None => blackout,
            Some(j) => match blackout.checked_add(j) {
                Ok(q) => q,
                Err(_) => {
                    return Judgement::withheld(
                        Missing::Overflow,
                        blackout.provenance().join(j.provenance()),
                    )
                }
            },
        };
        match arrivals_during(effective, gap) {
            Err(missing) => {
                Judgement::withheld(missing, blackout.provenance().join(gap.provenance()))
            }
            Ok(arrivals) => {
                let verdict = if arrivals.interval().hi() > self.depth as Count {
                    Verdict::Missed
                } else {
                    Verdict::Met
                };
                Judgement {
                    verdict,
                    provenance: arrivals.provenance(),
                    measured: Some(arrivals),
                }
            }
        }
    }
}

/// What a run of comparisons found.
///
/// The withheld count is reported alongside the rest so that an exclusion never
/// reads as a clean sweep.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Sweep {
    pub met: u32,
    pub missed: u32,
    pub withheld: u32,
}

impl Sweep {
    pub fn record(&mut self, j: Judgement) {
        match j.verdict {
            Verdict::Met => self.met += 1,
            Verdict::Missed => self.missed += 1,
            // An unarmed span is counted with the withheld rather than with
            // the passes, because nothing bounded it.
            Verdict::Unarmed | Verdict::Unanswerable(_) => self.withheld += 1,
        }
    }

    /// Whether every comparison in the run was made.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.withheld == 0
    }
}

#[cfg(test)]
mod tests {
    use super::{Arrival, Consequence, Deadline, Interrupt, InterruptId, Missing, Sweep, Verdict};
    use crate::interval::Interval;
    use crate::provenance::Provenance::{Assumed, Derived, Extracted};
    use crate::quantity::{Quantity, Unit};
    use crate::source::{SourceId, Span, Validity};
    use crate::Count;

    /// A span every always-armed deadline covers.
    fn whole() -> Span {
        Span::new(0, 0).unwrap()
    }

    fn q(lo: Count, hi: Count, unit: Unit, p: crate::provenance::Provenance) -> Quantity {
        Quantity::new(Interval::new(lo, hi).unwrap(), unit, p)
    }

    /// A line fed at one arrival every 1000 base ticks, buffered four deep,
    /// with a deadline of 5000 base ticks.
    fn line(depth: u32) -> Interrupt {
        Interrupt {
            id: InterruptId(0),
            arrival: Arrival::MinInterarrival(q(1_000, 1_000, Unit::Base, Extracted)),
            cost: q(10, 10, Unit::Base, Extracted),
            priority: 0,
            deadline: Some(Deadline {
                budget: q(5_000, 5_000, Unit::Base, Extracted),
                armed: Validity::Always,
            }),
            jitter: None,
            reenables: false,
            depth,
            on_drop: Consequence::LostAndLogged,
        }
    }

    #[test]
    fn a_blackout_inside_the_deadline_is_met() {
        let j = line(4).latency(q(0, 4_000, Unit::Base, Derived), whole());
        assert_eq!(j.verdict, Verdict::Met);
    }

    #[test]
    fn a_blackout_that_can_breach_is_reported_as_breaching() {
        // The worst case reaches past the deadline even though the best case
        // does not, and a worst-case tool reports the breach.
        let j = line(4).latency(q(0, 5_001, Unit::Base, Derived), whole());
        assert_eq!(j.verdict, Verdict::Missed);
    }

    #[test]
    fn occupancy_counts_the_arrival_at_the_instant_the_window_opens() {
        // A 3000-tick blackout at one arrival per 1000 ticks admits four: one
        // as it opens and three more inside it.
        let j = line(4).overrun(q(3_000, 3_000, Unit::Base, Derived));
        assert_eq!(j.measured.unwrap().interval().hi(), 4);
        assert_eq!(j.verdict, Verdict::Met);
    }

    #[test]
    fn one_more_arrival_than_the_buffer_holds_is_a_miss() {
        // A ring four deep overruns on the fifth arrival, and the latency
        // verdict on the same window is comfortably met, which is the whole
        // point of asking the overrun question separately.
        let blackout = q(4_000, 4_000, Unit::Base, Derived);
        let irq = line(4);
        assert_eq!(irq.overrun(blackout).measured.unwrap().interval().hi(), 5);
        assert_eq!(irq.overrun(blackout).verdict, Verdict::Missed);
        assert_eq!(irq.latency(blackout, whole()).verdict, Verdict::Met);
    }

    #[test]
    fn a_window_with_no_timebase_is_unanswerable_rather_than_passed() {
        // The clock tree is being reconfigured, so the window is honestly
        // denominated in loop passes and no conversion exists. The tool
        // declines both questions rather than assuming a rate.
        let blackout = q(0, 8_000, Unit::Iterations, Derived);
        let irq = line(4);
        assert_eq!(
            irq.latency(blackout, whole()).verdict,
            Verdict::Unanswerable(Missing::NoPathToTime(Unit::Iterations))
        );
        assert_eq!(
            irq.overrun(blackout).verdict,
            Verdict::Unanswerable(Missing::NoPathToTime(Unit::Iterations))
        );
    }

    #[test]
    fn an_interrupt_with_no_deadline_still_answers_on_occupancy() {
        let mut irq = line(4);
        irq.deadline = None;
        let blackout = q(0, 500, Unit::Base, Derived);
        assert_eq!(
            irq.latency(blackout, whole()).verdict,
            Verdict::Unanswerable(Missing::NoDeadline)
        );
        assert_eq!(irq.overrun(blackout).verdict, Verdict::Met);
    }

    #[test]
    fn an_arbitrarily_fast_line_bounds_nothing() {
        let mut irq = line(4);
        irq.arrival = Arrival::MinInterarrival(q(0, 10, Unit::Base, Assumed));
        assert_eq!(
            irq.overrun(q(0, 100, Unit::Base, Derived)).verdict,
            Verdict::Unanswerable(Missing::UnboundedArrivals)
        );
    }

    #[test]
    fn a_guessed_arrival_rate_makes_the_verdict_a_guess() {
        let mut irq = line(4);
        irq.arrival = Arrival::MinInterarrival(q(1_000, 1_000, Unit::Base, Assumed));
        let j = irq.overrun(q(3_000, 3_000, Unit::Base, Derived));
        assert_eq!(j.verdict, Verdict::Met);
        assert_eq!(j.provenance, Assumed, "the verdict rests on a guess");
    }

    #[test]
    fn a_periodic_arrival_reads_its_period_as_the_gap() {
        let irq = Interrupt {
            id: InterruptId(1),
            arrival: Arrival::Periodic {
                source: SourceId(0),
                period: q(1_000, 1_000, Unit::Base, Extracted),
            },
            cost: q(10, 10, Unit::Base, Extracted),
            priority: 1,
            deadline: None,
            jitter: None,
            reenables: false,
            depth: 2,
            on_drop: Consequence::Retried,
        };
        assert_eq!(irq.arrival.source(), Some(SourceId(0)));
        assert_eq!(
            irq.overrun(q(3_000, 3_000, Unit::Base, Derived)).verdict,
            Verdict::Missed
        );
    }

    #[test]
    fn a_span_with_no_deadline_armed_is_not_a_pass() {
        // A watchdog enabled part-way through a boot leaves everything before
        // it unbounded. Reporting that as met would say the opposite of what
        // is true, so it gets its own verdict.
        let mut irq = line(4);
        irq.deadline = Some(Deadline {
            budget: q(5_000, 5_000, Unit::Base, Extracted),
            armed: Validity::Over(Span::new(50, 1_000).unwrap()),
        });
        let tiny = q(0, 10, Unit::Base, Derived);
        // Inside the armed span the bound is judged as usual.
        assert_eq!(
            irq.latency(tiny, Span::new(60, 100).unwrap()).verdict,
            Verdict::Met
        );
        // Before it is armed, nothing bounds the window at all.
        let before = irq.latency(tiny, Span::new(0, 40).unwrap());
        assert_eq!(before.verdict, Verdict::Unarmed);
        assert!(!before.verdict.is_met(), "an unarmed span is not a pass");
    }

    #[test]
    fn jitter_lets_a_burst_land_that_even_spacing_would_not() {
        // Three thousand ticks at one arrival per thousand admits four. Allow a
        // release to be two thousand late and the same window admits six, which
        // overruns a ring that the even case fits.
        let even = line(5);
        assert_eq!(
            even.overrun(q(3_000, 3_000, Unit::Base, Derived)).verdict,
            Verdict::Met
        );
        let mut bunched = line(5);
        bunched.jitter = Some(q(2_000, 2_000, Unit::Base, Extracted));
        let j = bunched.overrun(q(3_000, 3_000, Unit::Base, Derived));
        assert_eq!(j.measured.unwrap().interval().hi(), 6);
        assert_eq!(j.verdict, Verdict::Missed);
    }

    #[test]
    fn a_sweep_reports_what_it_withheld() {
        let irq = line(4);
        let mut sweep = Sweep::default();
        sweep.record(irq.latency(q(0, 4_000, Unit::Base, Derived), whole()));
        sweep.record(irq.latency(q(0, 9_000, Unit::Base, Derived), whole()));
        sweep.record(irq.latency(q(0, 8_000, Unit::Iterations, Derived), whole()));
        assert_eq!((sweep.met, sweep.missed, sweep.withheld), (1, 1, 1));
        assert!(!sweep.is_complete(), "a withheld verdict is not a pass");
    }
}
