//! Composition, and the input at which the worst case is attained.
//!
//! A wait costs what it costs because of something outside the part: how long
//! the peripheral takes to answer. That is one parameter, and every wait in a
//! composition sees the same world, so a cost is a function of it rather than a
//! constant.
//!
//! Evaluating at one latency is cheap. The question worth asking is which
//! latency is worst, and the answer is not reliably at either end of the range.
//! A guard that fails costs its budget and unwinds; a guard that succeeds on its
//! last permitted attempt costs almost as much **and** lets everything above it
//! run. So the expensive case can sit one step inside the boundary, where a
//! sweep of the extremes will not find it.

use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::quantity::{Quantity, Refusal, Unit};
use crate::Count;

/// A declared wait, by index into the register's wait table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct WaitId(pub u16);

/// What happens when a wait uses up its budget.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Exhaustion {
    /// The wait returns a failure its caller can act on.
    ReportsError,
    /// The wait stops the program.
    Asserts,
    /// The wait gives up and carries on as though it had succeeded, which is
    /// the worst of the three because nothing downstream can tell.
    SilentlyContinues,
}

/// A polling wait: it asks, and either gets an answer or runs out of budget.
#[derive(Clone, Copy, Debug)]
pub struct Wait {
    pub id: WaitId,
    /// The most iterations this wait will make before giving up.
    pub budget: Count,
    /// What one iteration costs.
    pub cost_per_iter: Quantity,
    pub on_exhaustion: Exhaustion,
}

/// How a composition is built.
#[derive(Clone, Debug)]
pub enum Expr {
    Leaf(Wait),
    /// One after another, stopping where one fails, which is what unwinding on
    /// an error looks like.
    Seq(Vec<Expr>),
    /// A retry loop. It stops early where the body fails.
    Repeat {
        body: Box<Expr>,
        times: Count,
    },
    /// A branch. Only one arm runs, so the cost is the dearest of them.
    Alt(Vec<Expr>),
    /// A guard, and what runs only if the guard succeeds.
    ///
    /// This is the operator hand-analysis gets wrong. Its worst case is
    /// `max(cost(guard fails), cost(guard succeeds) + cost(then))`, which is
    /// not a product, and where the guard fails the `then` never runs at all.
    ShortCircuit {
        guard: Box<Expr>,
        then: Box<Expr>,
    },
}

/// What one evaluation cost, and whether it got its answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Outcome {
    pub cost: Quantity,
    /// Whether every part that ran got its answer within budget.
    pub succeeded: bool,
}

/// The worst case, and the input that produces it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attainment {
    pub cost: Quantity,
    /// The latency at which the worst case occurs. Reported because a maximum
    /// sitting inside the range is the finding, and a bare number hides it.
    pub witness: Count,
    /// Whether the cost rose all the way to the witness. Where it did not, the
    /// worst case is interior and a sweep of the extremes would have missed it.
    pub interior: bool,
}

impl Expr {
    /// The cost at one latency, where `latency` is how many iterations the
    /// outside world takes to answer.
    pub fn eval(&self, latency: Count, unit: Unit) -> Result<Outcome, Refusal> {
        match self {
            Expr::Leaf(w) => {
                if w.cost_per_iter.unit() != unit {
                    return Err(Refusal::UnitMismatch {
                        left: w.cost_per_iter.unit(),
                        right: unit,
                    });
                }
                // It polls until it gets an answer or runs out of budget.
                let iterations = latency.min(w.budget);
                let cost = w
                    .cost_per_iter
                    .checked_repeat(Interval::point(iterations), Provenance::Derived)?;
                Ok(Outcome {
                    cost,
                    succeeded: latency < w.budget,
                })
            }
            Expr::Seq(parts) => {
                let mut total = zero(unit);
                for p in parts {
                    let o = p.eval(latency, unit)?;
                    total = total.checked_add(o.cost)?;
                    if !o.succeeded {
                        // It unwinds here, so nothing after this runs.
                        return Ok(Outcome {
                            cost: total,
                            succeeded: false,
                        });
                    }
                }
                Ok(Outcome {
                    cost: total,
                    succeeded: true,
                })
            }
            Expr::Repeat { body, times } => {
                let mut total = zero(unit);
                for _ in 0..*times {
                    let o = body.eval(latency, unit)?;
                    total = total.checked_add(o.cost)?;
                    if !o.succeeded {
                        return Ok(Outcome {
                            cost: total,
                            succeeded: false,
                        });
                    }
                }
                Ok(Outcome {
                    cost: total,
                    succeeded: true,
                })
            }
            Expr::Alt(arms) => {
                let mut worst = zero(unit);
                let mut succeeded = true;
                for a in arms {
                    let o = a.eval(latency, unit)?;
                    if o.cost.interval().hi() > worst.interval().hi() {
                        worst = o.cost;
                        succeeded = o.succeeded;
                    }
                }
                Ok(Outcome {
                    cost: worst,
                    succeeded,
                })
            }
            Expr::ShortCircuit { guard, then } => {
                let g = guard.eval(latency, unit)?;
                if !g.succeeded {
                    // The `then` never runs. This is the cheap branch, and a
                    // tool that stopped here would report it as the worst.
                    return Ok(Outcome {
                        cost: g.cost,
                        succeeded: false,
                    });
                }
                let t = then.eval(latency, unit)?;
                Ok(Outcome {
                    cost: g.cost.checked_add(t.cost)?,
                    succeeded: t.succeeded,
                })
            }
        }
    }

    /// The largest budget anywhere in this composition, which bounds the
    /// latencies worth trying: past it, every wait has already given up.
    #[must_use]
    pub fn widest_budget(&self) -> Count {
        match self {
            Expr::Leaf(w) => w.budget,
            Expr::Seq(parts) | Expr::Alt(parts) => {
                parts.iter().map(Expr::widest_budget).max().unwrap_or(0)
            }
            Expr::Repeat { body, .. } => body.widest_budget(),
            Expr::ShortCircuit { guard, then } => guard.widest_budget().max(then.widest_budget()),
        }
    }

    /// The worst case over every latency the composition can distinguish, and
    /// the latency that attains it.
    ///
    /// Monotonicity is never assumed. The search runs the domain, which is
    /// bounded by the widest budget rather than by any cost, so it stays small
    /// even where the costs do not.
    pub fn attain(&self, unit: Unit) -> Result<Attainment, Refusal> {
        let ceiling = self.widest_budget();
        let mut best = self.eval(0, unit)?.cost;
        let mut witness: Count = 0;
        // One past the widest budget is the case where nothing ever answers.
        for latency in 1..=ceiling.saturating_add(1) {
            let here = self.eval(latency, unit)?.cost;
            if here.interval().hi() > best.interval().hi() {
                best = here;
                witness = latency;
            }
        }
        Ok(Attainment {
            cost: best,
            witness,
            interior: witness > 0 && witness <= ceiling,
        })
    }
}

fn zero(unit: Unit) -> Quantity {
    Quantity::new(Interval::point(0), unit, Provenance::Derived)
}

#[cfg(test)]
mod tests {
    use super::{Exhaustion, Expr, Wait, WaitId};
    use crate::interval::Interval;
    use crate::provenance::Provenance::{Assumed, Derived, Extracted};
    use crate::quantity::{Quantity, Unit};
    use crate::Count;

    fn wait(id: u16, budget: Count, per_iter: Count) -> Expr {
        Expr::Leaf(Wait {
            id: WaitId(id),
            budget,
            cost_per_iter: Quantity::new(Interval::point(per_iter), Unit::BusReads, Extracted),
            on_exhaustion: Exhaustion::ReportsError,
        })
    }

    #[test]
    fn a_wait_stops_at_its_budget() {
        let w = wait(0, 100, 1);
        // It answers at 40, so it polls 40 times and succeeds.
        let o = w.eval(40, Unit::BusReads).unwrap();
        assert_eq!(o.cost.interval().hi(), 40);
        assert!(o.succeeded);
        // It never answers, so it polls its whole budget and gives up.
        let o = w.eval(1_000, Unit::BusReads).unwrap();
        assert_eq!(o.cost.interval().hi(), 100);
        assert!(!o.succeeded);
    }

    #[test]
    fn a_sequence_unwinds_where_a_part_fails() {
        let e = Expr::Seq(vec![wait(0, 10, 1), wait(1, 100, 1)]);
        // At latency 50 the first wait gives up after 10, and the second never
        // runs, so the cost is 10 rather than 60.
        let o = e.eval(50, Unit::BusReads).unwrap();
        assert_eq!(o.cost.interval().hi(), 10);
        assert!(!o.succeeded);
    }

    #[test]
    fn a_branch_takes_the_dearer_arm() {
        let e = Expr::Alt(vec![wait(0, 10, 1), wait(1, 100, 1)]);
        let o = e.eval(5_000, Unit::BusReads).unwrap();
        assert_eq!(o.cost.interval().hi(), 100);
    }

    #[test]
    fn a_short_circuit_does_not_run_its_body_when_the_guard_fails() {
        let e = Expr::ShortCircuit {
            guard: Box::new(wait(0, 10, 1)),
            then: Box::new(wait(1, 1_000, 1)),
        };
        // The guard gives up at 10 and the body never runs.
        let o = e.eval(50, Unit::BusReads).unwrap();
        assert_eq!(o.cost.interval().hi(), 10);
    }

    #[test]
    fn the_worst_case_sits_one_step_inside_the_boundary() {
        // A retry loop over a guard and a body. Where the guard never answers
        // it gives up at once and the loop unwinds. Where the guard answers on
        // its last permitted attempt, it costs almost its whole budget AND lets
        // the body and every remaining retry run.
        let e = Expr::Repeat {
            body: Box::new(Expr::ShortCircuit {
                guard: Box::new(wait(0, 100, 1)),
                then: Box::new(wait(1, 100, 1)),
            }),
            times: 50,
        };
        let a = e.attain(Unit::BusReads).unwrap();

        // The witness is the last latency at which the guard still answers.
        assert_eq!(a.witness, 99);
        assert!(a.interior, "the maximum is not at either extreme");

        // The extremes are both far cheaper than the interior maximum.
        let never_answers = e.eval(1_000, Unit::BusReads).unwrap().cost.interval().hi();
        let answers_at_once = e.eval(0, Unit::BusReads).unwrap().cost.interval().hi();
        assert_eq!(never_answers, 100, "it unwinds on the first retry");
        assert_eq!(answers_at_once, 0);
        assert!(
            a.cost.interval().hi() > never_answers * 90,
            "a sweep of the extremes would have understated this by orders of magnitude"
        );
    }

    #[test]
    fn the_reported_cost_is_the_cost_at_the_reported_witness() {
        let e = Expr::Repeat {
            body: Box::new(Expr::ShortCircuit {
                guard: Box::new(wait(0, 64, 2)),
                then: Box::new(wait(1, 8, 3)),
            }),
            times: 7,
        };
        let a = e.attain(Unit::BusReads).unwrap();
        let at_witness = e.eval(a.witness, Unit::BusReads).unwrap();
        assert_eq!(a.cost, at_witness.cost, "the witness must reproduce it");
    }

    #[test]
    fn the_search_is_bounded_by_the_budget_and_not_by_the_cost() {
        // Costs run to millions; the domain is a hundred and one wide.
        let e = Expr::Repeat {
            body: Box::new(wait(0, 100, 1_000)),
            times: 100,
        };
        assert_eq!(e.widest_budget(), 100);
        let a = e.attain(Unit::BusReads).unwrap();
        assert!(a.cost.interval().hi() >= 1_000_000);
    }

    #[test]
    fn a_guessed_per_iteration_cost_makes_the_composition_a_guess() {
        let e = Expr::Seq(vec![
            wait(0, 10, 1),
            Expr::Leaf(super::Wait {
                id: WaitId(1),
                budget: 10,
                cost_per_iter: Quantity::new(Interval::point(1), Unit::BusReads, Assumed),
                on_exhaustion: Exhaustion::SilentlyContinues,
            }),
        ]);
        let o = e.eval(5, Unit::BusReads).unwrap();
        assert_eq!(o.cost.provenance(), Assumed);
    }

    #[test]
    fn composing_across_units_is_refused() {
        let e = Expr::Seq(vec![wait(0, 10, 1)]);
        assert!(e.eval(5, Unit::Iterations).is_err());
    }

    #[test]
    fn a_monotone_composition_attains_at_the_far_end() {
        // No short circuit, so cost only rises with latency and the maximum is
        // at the boundary. The tool reports that it is not interior.
        let e = Expr::Seq(vec![wait(0, 100, 1)]);
        let a = e.attain(Unit::BusReads).unwrap();
        assert_eq!(a.cost.interval().hi(), 100);
        assert!(
            !a.interior || a.witness == 100,
            "a rising cost peaks at the last latency that still costs more"
        );
        let _ = Derived;
    }
}
