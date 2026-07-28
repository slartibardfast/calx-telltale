//! Whether a declared set of interrupts keeps up.
//!
//! Two questions, cheapest first.
//!
//! **Utilisation** is `Σ C/T` over the set. A set demanding more than all of
//! the time it has cannot be rescued by any ordering, so this is answered
//! without iterating.
//!
//! **Response time** is the fixed point of
//!
//! ```text
//! R = C + B + Σ over higher priorities of ceil(R / T) * C
//! ```
//!
//! which is the standard fixed-priority analysis rather than anything invented
//! here. The blocking term `B` is the interrupts-off window this project
//! already computes, so the two halves of the tool meet at that symbol.

use crate::interrupt::{mismatch, Interrupt, InterruptId, Missing};
use crate::provenance::Provenance;
use crate::quantity::Quantity;
use crate::Count;

/// Utilisation is carried in parts per million, so it stays exact integer
/// arithmetic. Unity is a million.
pub const FULLY_UTILISED: Count = 1_000_000;

/// A bound on the recurrence. A response time that has not settled by here is
/// not going to, and the set is reported as unable to keep up rather than the
/// run reported as broken.
const MAX_ROUNDS: u32 = 64;

/// What the analysis concluded about one interrupt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Schedulability {
    /// The response time settled inside the deadline.
    Keeps {
        /// The worst-case response time the recurrence settled on.
        response: Quantity,
        /// How much deadline was left over. The difference between a verdict
        /// that holds by a hair and one that holds comfortably.
        margin: Quantity,
        /// Rounds the recurrence took to settle, which is a cheap sanity check
        /// on a surprising answer.
        rounds: u32,
    },
    /// The response time passed the deadline, or never settled at all.
    Misses { response: Quantity, rounds: u32 },
    /// The set demands more time than exists, so no ordering rescues it.
    Oversubscribed { utilisation_ppm: Count },
    /// The comparison could not be made, and this names what was missing.
    Unanswerable(Missing),
}

impl Schedulability {
    #[must_use]
    pub const fn is_answerable(self) -> bool {
        !matches!(self, Schedulability::Unanswerable(_))
    }

    #[must_use]
    pub const fn keeps_up(self) -> bool {
        matches!(self, Schedulability::Keeps { .. })
    }
}

/// A verdict and the provenance it rests on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Analysis {
    pub verdict: Schedulability,
    pub provenance: Provenance,
}

/// A declared set of interrupts, judged together.
#[derive(Clone, Debug, Default)]
pub struct Schedule {
    interrupts: Vec<Interrupt>,
}

impl Schedule {
    #[must_use]
    pub fn new(interrupts: Vec<Interrupt>) -> Self {
        Self { interrupts }
    }

    #[must_use]
    pub fn get(&self, id: InterruptId) -> Option<&Interrupt> {
        self.interrupts.iter().find(|i| i.id == id)
    }

    /// The share of available time the declared set demands, in parts per
    /// million.
    ///
    /// Every cost and every period must share a unit, because a ratio across
    /// units means nothing.
    pub fn utilisation_ppm(&self) -> Result<(Count, Provenance), Missing> {
        let mut total: Count = 0;
        let mut prov = Provenance::Derived;
        for irq in &self.interrupts {
            let period = irq.arrival.shortest_gap();
            if period.unit() != irq.cost.unit() {
                return Err(mismatch(irq.cost.unit(), period.unit()));
            }
            if period.interval().lo() == 0 {
                return Err(Missing::UnboundedArrivals);
            }
            // Worst case: the longest cost against the shortest period.
            let share = irq
                .cost
                .interval()
                .hi()
                .checked_mul(FULLY_UTILISED)
                .ok_or(Missing::Overflow)?
                / period.interval().lo();
            total = total.checked_add(share).ok_or(Missing::Overflow)?;
            prov = prov.join(irq.cost.provenance()).join(period.provenance());
        }
        Ok((total, prov))
    }

    /// The worst-case response time of one interrupt, given the longest window
    /// in which it can be held off.
    ///
    /// The blocking term is a blackout window: a span where interrupts are off
    /// is a span where even the highest priority cannot run.
    pub fn response_time(&self, target: InterruptId, blocking: Quantity) -> Analysis {
        let Some(irq) = self.get(target) else {
            return Analysis {
                verdict: Schedulability::Unanswerable(Missing::NoDeadline),
                provenance: blocking.provenance(),
            };
        };
        let unit = irq.cost.unit();
        let mut prov = irq.cost.provenance().join(blocking.provenance());

        if blocking.unit() != unit {
            return Analysis {
                verdict: Schedulability::Unanswerable(mismatch(blocking.unit(), unit)),
                provenance: prov,
            };
        }

        // The cheap check first: a set demanding more than all of the time it
        // has cannot be rescued by any ordering.
        match self.utilisation_ppm() {
            Err(missing) => {
                return Analysis {
                    verdict: Schedulability::Unanswerable(missing),
                    provenance: prov,
                }
            }
            Ok((u, up)) => {
                prov = prov.join(up);
                if u > FULLY_UTILISED {
                    return Analysis {
                        verdict: Schedulability::Oversubscribed { utilisation_ppm: u },
                        provenance: prov,
                    };
                }
            }
        }

        let Some(deadline) = irq.deadline else {
            return Analysis {
                verdict: Schedulability::Unanswerable(Missing::NoDeadline),
                provenance: prov,
            };
        };
        if deadline.unit() != unit {
            return Analysis {
                verdict: Schedulability::Unanswerable(mismatch(unit, deadline.unit())),
                provenance: prov,
            };
        }
        prov = prov.join(deadline.provenance());

        // Higher priority means a lower number, the way the hardware numbers it.
        let higher: Vec<&Interrupt> = self
            .interrupts
            .iter()
            .filter(|j| j.preempts(irq) && j.id != irq.id)
            .collect();
        for j in &higher {
            if j.cost.unit() != unit || j.arrival.shortest_gap().unit() != unit {
                return Analysis {
                    verdict: Schedulability::Unanswerable(mismatch(j.cost.unit(), unit)),
                    provenance: prov,
                };
            }
            prov = prov
                .join(j.cost.provenance())
                .join(j.arrival.shortest_gap().provenance());
        }

        let base = match irq
            .cost
            .interval()
            .hi()
            .checked_add(blocking.interval().hi())
        {
            Some(v) => v,
            None => {
                return Analysis {
                    verdict: Schedulability::Unanswerable(Missing::Overflow),
                    provenance: prov,
                }
            }
        };
        let limit = deadline.interval().lo();

        let mut r = base;
        for round in 1..=MAX_ROUNDS {
            let mut next = base;
            let mut overflowed = false;
            for j in &higher {
                let period = j.arrival.shortest_gap().interval().lo();
                if period == 0 {
                    return Analysis {
                        verdict: Schedulability::Unanswerable(Missing::UnboundedArrivals),
                        provenance: prov,
                    };
                }
                // ceil(r / period) releases of j land inside a window of r.
                let releases = r / period + Count::from(r % period != 0);
                match releases
                    .checked_mul(j.cost.interval().hi())
                    .and_then(|v| next.checked_add(v))
                {
                    Some(v) => next = v,
                    None => {
                        overflowed = true;
                        break;
                    }
                }
            }
            if overflowed || next > limit {
                // A response already past its deadline cannot be rescued by a
                // further round, so the iteration stops here.
                let response = Quantity::new(
                    crate::Interval::new(base, if overflowed { Count::MAX } else { next })
                        .expect("base is the least the response can be"),
                    unit,
                    prov,
                );
                return Analysis {
                    verdict: Schedulability::Misses {
                        response,
                        rounds: round,
                    },
                    provenance: prov,
                };
            }
            if next == r {
                let response = Quantity::new(
                    crate::Interval::new(base, r).expect("base is the least the response can be"),
                    unit,
                    prov,
                );
                let margin = Quantity::new(
                    crate::Interval::new(limit - r, deadline.interval().hi() - r)
                        .expect("r is within the deadline here"),
                    unit,
                    prov,
                );
                return Analysis {
                    verdict: Schedulability::Keeps {
                        response,
                        margin,
                        rounds: round,
                    },
                    provenance: prov,
                };
            }
            r = next;
        }

        // It never settled. A response time that grows without settling is a
        // set that cannot keep up, rather than a defect in the run.
        let response = Quantity::new(
            crate::Interval::new(base, r).expect("base is the least the response can be"),
            unit,
            prov,
        );
        Analysis {
            verdict: Schedulability::Misses {
                response,
                rounds: MAX_ROUNDS,
            },
            provenance: prov,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Schedulability, Schedule, FULLY_UTILISED};
    use crate::interrupt::{Arrival, Consequence, Interrupt, InterruptId, Missing};
    use crate::interval::Interval;
    use crate::provenance::Provenance::{Assumed, Derived, Extracted};
    use crate::quantity::{Quantity, Unit};
    use crate::Count;

    fn q(v: Count, unit: Unit, p: crate::provenance::Provenance) -> Quantity {
        Quantity::new(Interval::point(v), unit, p)
    }

    /// An interrupt costing `cost` every `period`, at `priority`.
    fn irq(id: u16, priority: u8, cost: Count, period: Count, deadline: Count) -> Interrupt {
        Interrupt {
            id: InterruptId(id),
            arrival: Arrival::MinInterarrival(q(period, Unit::Base, Extracted)),
            cost: q(cost, Unit::Base, Extracted),
            priority,
            deadline: Some(q(deadline, Unit::Base, Extracted)),
            depth: 4,
            on_drop: Consequence::LostAndLogged,
        }
    }

    #[test]
    fn utilisation_is_the_sum_of_the_shares() {
        // 100 every 1000 is a tenth; 200 every 1000 is a fifth. Three tenths.
        let s = Schedule::new(vec![
            irq(0, 0, 100, 1_000, 1_000),
            irq(1, 1, 200, 1_000, 1_000),
        ]);
        let (u, _) = s.utilisation_ppm().unwrap();
        assert_eq!(u, 300_000);
    }

    #[test]
    fn a_set_demanding_more_time_than_exists_is_refused_without_iterating() {
        let s = Schedule::new(vec![
            irq(0, 0, 600, 1_000, 1_000),
            irq(1, 1, 700, 1_000, 5_000),
        ]);
        let a = s.response_time(InterruptId(1), q(0, Unit::Base, Derived));
        assert!(matches!(
            a.verdict,
            Schedulability::Oversubscribed { utilisation_ppm } if utilisation_ppm > FULLY_UTILISED
        ));
    }

    #[test]
    fn the_highest_priority_sees_only_its_own_cost_and_the_blocking() {
        let s = Schedule::new(vec![irq(0, 0, 100, 1_000, 500), irq(1, 1, 100, 1_000, 900)]);
        let a = s.response_time(InterruptId(0), q(50, Unit::Base, Derived));
        match a.verdict {
            Schedulability::Keeps {
                response, margin, ..
            } => {
                assert_eq!(response.interval().hi(), 150);
                assert_eq!(margin.interval().lo(), 350);
            }
            other => panic!("expected it to keep up, got {other:?}"),
        }
    }

    #[test]
    fn a_lower_priority_pays_for_every_release_above_it() {
        // The low one costs 200 and is blocked 0. The high one costs 100 every
        // 1000. A response of 300 admits one release of the high one, so the
        // fixed point is 200 + 100 = 300.
        let s = Schedule::new(vec![irq(0, 0, 100, 1_000, 500), irq(1, 1, 200, 1_000, 900)]);
        let a = s.response_time(InterruptId(1), q(0, Unit::Base, Derived));
        match a.verdict {
            Schedulability::Keeps { response, .. } => assert_eq!(response.interval().hi(), 300),
            other => panic!("expected it to keep up, got {other:?}"),
        }
    }

    #[test]
    fn interference_compounds_until_it_settles() {
        // The high one costs 400 every 1000. The low one costs 300. First round
        // 300 + 400 = 700; second round still admits one release, so it settles
        // at 700 with 200 of deadline left.
        let s = Schedule::new(vec![irq(0, 0, 400, 1_000, 500), irq(1, 1, 300, 1_000, 900)]);
        let a = s.response_time(InterruptId(1), q(0, Unit::Base, Derived));
        match a.verdict {
            Schedulability::Keeps {
                response,
                margin,
                rounds,
            } => {
                assert_eq!(response.interval().hi(), 700);
                assert_eq!(margin.interval().lo(), 200);
                assert!(rounds >= 2, "it took more than one round to settle");
            }
            other => panic!("expected it to keep up, got {other:?}"),
        }
    }

    #[test]
    fn a_blackout_window_is_the_blocking_term() {
        // The same set, but interrupts are off for 300 somewhere. That pushes
        // the response past the deadline, which is the blackout mattering.
        let s = Schedule::new(vec![irq(0, 0, 400, 1_000, 500), irq(1, 1, 300, 1_000, 900)]);
        let a = s.response_time(InterruptId(1), q(300, Unit::Base, Derived));
        assert!(matches!(a.verdict, Schedulability::Misses { .. }));
    }

    #[test]
    fn a_guessed_handler_cost_makes_the_verdict_a_guess() {
        let mut low = irq(1, 1, 200, 1_000, 900);
        low.cost = q(200, Unit::Base, Assumed);
        let s = Schedule::new(vec![irq(0, 0, 100, 1_000, 500), low]);
        let a = s.response_time(InterruptId(1), q(0, Unit::Base, Derived));
        assert!(a.verdict.keeps_up());
        assert_eq!(a.provenance, Assumed, "the verdict rests on a guess");
    }

    #[test]
    fn a_cost_with_no_path_to_time_is_unanswerable() {
        let mut low = irq(1, 1, 200, 1_000, 900);
        low.cost = q(200, Unit::Iterations, Derived);
        low.arrival = Arrival::MinInterarrival(q(1_000, Unit::Iterations, Extracted));
        let s = Schedule::new(vec![low]);
        let a = s.response_time(InterruptId(1), q(0, Unit::Iterations, Derived));
        assert!(!a.verdict.is_answerable());
        assert_eq!(
            a.verdict,
            Schedulability::Unanswerable(Missing::NoPathToTime(Unit::Iterations)),
            "the honest reason is that a loop count is not a duration"
        );
    }

    #[test]
    fn priority_is_numbered_the_way_the_hardware_numbers_it() {
        let high = irq(0, 0, 100, 1_000, 500);
        let low = irq(1, 7, 100, 1_000, 900);
        assert!(high.preempts(&low));
        assert!(!low.preempts(&high));
    }
}
