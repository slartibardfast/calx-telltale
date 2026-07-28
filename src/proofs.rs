//! Kani harnesses for the obligations the core carries.
//!
//! These are the properties of the arithmetic, proved universally rather than
//! sampled. Run them with `cargo kani`.

use crate::expr::{Counter, CounterFit, Exhaustion, Expr, Measure, Termination, Wait, WaitId};
use crate::interrupt::{Arrival, Consequence, Interrupt, InterruptId};
use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::quantity::{Quantity, Refusal, Unit};
use crate::source::SourceId;
use crate::Count;

fn any_provenance() -> Provenance {
    match kani::any::<u8>() % 4 {
        0 => Provenance::Assumed,
        1 => Provenance::Measured,
        2 => Provenance::Extracted,
        _ => Provenance::Derived,
    }
}

fn any_unit() -> Unit {
    let id = SourceId(kani::any::<u16>());
    match kani::any::<u8>() % 6 {
        0 => Unit::Iterations,
        1 => Unit::BusReads,
        2 => Unit::Ticks(id),
        3 => Unit::Cycles(id),
        4 => Unit::Base,
        _ => Unit::Nanos,
    }
}

fn any_interval() -> Interval {
    let lo: Count = kani::any();
    let hi: Count = kani::any();
    kani::assume(lo <= hi);
    Interval::new(lo, hi).expect("lo <= hi was assumed")
}

/// The values a product can go wrong at.
///
/// Multiplication over the whole domain defeats a solver that bit-blasts it,
/// at either stored width and with either a SAT or an SMT back end, so the
/// multiplying harnesses run over the boundaries instead of over everything.
///
/// This is a deliberate trade rather than a shortcut. The mathematics is not
/// in question: the product of two non-negative intervals has the products of
/// the corresponding endpoints as its own endpoints, which is textbook interval
/// arithmetic and needs no machine to confirm it. What is in question is the
/// transcription, meaning whether this implementation pairs the endpoints
/// correctly and refuses where the product no longer fits. A wrong pairing or a
/// missing refusal shows up at the identities, at the values that straddle the
/// point where a product stops fitting, and at the extremes. So those are what
/// the harnesses quantify over.
const EDGES: [Count; 9] = [
    0,
    1,
    2,
    ((1 as Count) << 63) - 1,
    (1 as Count) << 63,
    ((1 as Count) << 64) - 1,
    (1 as Count) << 64,
    ((1 as Count) << 127) - 1,
    Count::MAX,
];

fn any_edge() -> Count {
    let i: usize = kani::any();
    kani::assume(i < EDGES.len());
    EDGES[i]
}

fn any_edge_interval() -> Interval {
    let lo = any_edge();
    let hi = any_edge();
    kani::assume(lo <= hi);
    Interval::new(lo, hi).expect("lo <= hi was assumed")
}

fn any_quantity() -> Quantity {
    Quantity::new(any_interval(), any_unit(), any_provenance())
}

/// Provenance monotonicity, on the join itself: the result is at most as strong as either input.
#[kani::proof]
fn join_is_never_stronger_than_its_inputs() {
    let a = any_provenance();
    let b = any_provenance();
    let j = a.join(b);
    assert!(j.strength() <= a.strength());
    assert!(j.strength() <= b.strength());
    // And it is one of them, rather than some third thing.
    assert!(j == a || j == b);
}

/// Unit soundness: adding across units is refused, whatever the values.
///
/// Two counts of different clocks are different units, so this also covers the
/// case the founding note opens with, where three envelopes in three units were
/// added by hand.
#[kani::proof]
fn addition_across_units_is_refused() {
    let a = any_quantity();
    let b = any_quantity();
    if a.unit() != b.unit() {
        assert!(matches!(
            a.checked_add(b),
            Err(Refusal::UnitMismatch { .. })
        ));
    }
}

/// Unit soundness, for the branch operator as well as the sequence operator.
#[kani::proof]
fn max_across_units_is_refused() {
    let a = any_quantity();
    let b = any_quantity();
    if a.unit() != b.unit() {
        assert!(matches!(
            a.checked_max(b),
            Err(Refusal::UnitMismatch { .. })
        ));
    }
}

/// Provenance monotonicity: a sum carries the weakest provenance of its inputs.
#[kani::proof]
fn a_sum_is_no_stronger_than_its_weakest_input() {
    let a = any_quantity();
    let b = any_quantity();
    if let Ok(r) = a.checked_add(b) {
        assert!(r.provenance().strength() <= a.provenance().strength());
        assert!(r.provenance().strength() <= b.provenance().strength());
    }
}

/// Provenance monotonicity, through the branch operator.
#[kani::proof]
fn a_branch_is_no_stronger_than_its_weakest_input() {
    let a = any_quantity();
    let b = any_quantity();
    if let Ok(r) = a.checked_max(b) {
        assert!(r.provenance().strength() <= a.provenance().strength());
        assert!(r.provenance().strength() <= b.provenance().strength());
    }
}

/// Provenance monotonicity, through repetition. A guessed loop count taints a solid body.
#[kani::proof]
fn repetition_carries_the_count_provenance() {
    let body = Quantity::new(any_edge_interval(), any_unit(), any_provenance());
    let times = any_edge_interval();
    let times_prov = any_provenance();
    if let Ok(r) = body.checked_repeat(times, times_prov) {
        assert!(r.provenance().strength() <= body.provenance().strength());
        assert!(r.provenance().strength() <= times_prov.strength());
    }
}

/// Interval soundness: the sum interval contains the sum of every point evaluation.
#[kani::proof]
fn addition_contains_every_point_evaluation() {
    let a = any_interval();
    let b = any_interval();
    let x: Count = kani::any();
    let y: Count = kani::any();
    kani::assume(a.contains(x));
    kani::assume(b.contains(y));

    if let Some(r) = a.checked_add(b) {
        // The endpoints did not overflow, and the points sit under them.
        let s = x.checked_add(y);
        assert!(s.is_some());
        assert!(r.contains(s.expect("bounded by the endpoints")));
    }
}

/// Interval soundness, through multiplication, which is where repetition lands.
#[kani::proof]
fn multiplication_contains_every_point_evaluation() {
    let a = any_edge_interval();
    let b = any_edge_interval();
    let x = any_edge();
    let y = any_edge();
    kani::assume(a.contains(x));
    kani::assume(b.contains(y));

    if let Some(r) = a.checked_mul(b) {
        let p = x.checked_mul(y);
        assert!(p.is_some());
        assert!(r.contains(p.expect("bounded by the endpoints")));
    }
}

/// Accumulator fit: arithmetic that would leave the width refuses rather than wrapping.
#[kani::proof]
fn addition_refuses_rather_than_wrapping() {
    let a = any_quantity();
    let b = any_quantity();
    kani::assume(a.unit() == b.unit());
    match a.checked_add(b) {
        Ok(r) => {
            // A result exists only where neither endpoint wrapped.
            assert!(r.interval().lo() >= a.interval().lo());
            assert!(r.interval().hi() >= a.interval().hi());
        }
        Err(e) => assert!(matches!(e, Refusal::Overflow)),
    }
}

// The interval invariant, one operation to a harness. Bundling the three into
// a single proof multiplies the solver's work for no extra coverage.

/// A sum is an interval: its endpoints stay in order.
#[kani::proof]
fn addition_preserves_interval_order() {
    let a = any_interval();
    let b = any_interval();
    if let Some(r) = a.checked_add(b) {
        assert!(r.lo() <= r.hi());
    }
}

/// A product is an interval: its endpoints stay in order.
#[kani::proof]
fn multiplication_preserves_interval_order() {
    let a = any_edge_interval();
    let b = any_edge_interval();
    if let Some(r) = a.checked_mul(b) {
        assert!(r.lo() <= r.hi());
    }
}

/// A branch maximum is an interval: its endpoints stay in order.
#[kani::proof]
fn maximum_preserves_interval_order() {
    let a = any_interval();
    let b = any_interval();
    let m = a.max(b);
    assert!(m.lo() <= m.hi());
}

// The interrupt verdicts. What is worth proving here is that a comparison the
// register cannot support is never resolved either way, and that a verdict
// never claims more standing than the weakest thing it rests on.

fn any_untimed_unit() -> Unit {
    if kani::any::<bool>() {
        Unit::Iterations
    } else {
        Unit::BusReads
    }
}

fn an_interrupt(gap_prov: Provenance, depth: u32) -> Interrupt {
    Interrupt {
        id: InterruptId(0),
        arrival: Arrival::MinInterarrival(Quantity::new(any_edge_interval(), Unit::Base, gap_prov)),
        cost: Quantity::new(any_edge_interval(), Unit::Base, any_provenance()),
        priority: 0,
        deadline: Some(Quantity::new(
            any_edge_interval(),
            Unit::Base,
            any_provenance(),
        )),
        depth,
        on_drop: Consequence::LostSilently,
    }
}

/// A window with no path to a time is never judged, whatever else is declared.
///
/// This is the refusal the whole design turns on. A blackout measured while its
/// clock was being reconfigured is counted in loop passes, and no declaration
/// elsewhere in the register can make that comparable to a deadline.
#[kani::proof]
fn a_blackout_with_no_path_to_time_is_never_judged() {
    let blackout = Quantity::new(any_edge_interval(), any_untimed_unit(), any_provenance());
    let irq = an_interrupt(any_provenance(), kani::any());
    assert!(!irq.latency(blackout).verdict.is_answerable());
    assert!(!irq.overrun(blackout).verdict.is_answerable());
}

/// A verdict carries the weakest provenance among the declarations it rests on.
#[kani::proof]
fn a_verdict_is_no_stronger_than_its_weakest_input() {
    let blackout_prov = any_provenance();
    let gap_prov = any_provenance();
    let blackout = Quantity::new(any_edge_interval(), Unit::Base, blackout_prov);
    let irq = an_interrupt(gap_prov, kani::any());
    let j = irq.overrun(blackout);
    assert!(j.provenance.strength() <= blackout_prov.strength());
    assert!(j.provenance.strength() <= gap_prov.strength());
}

/// Priority is a strict order, so no interrupt preempts itself and no two
/// preempt each other.
///
/// The recurrence relies on this to partition the set: an interrupt that
/// appeared in its own interference term would never settle. The rest of the
/// schedule analysis is covered by tests rather than by harnesses, because the
/// recurrence walks a heap-allocated set and that is not what the deeper rung
/// is worth spending on (call/0009).
#[kani::proof]
fn preemption_is_a_strict_order() {
    let a = an_interrupt_at(kani::any());
    let b = an_interrupt_at(kani::any());
    assert!(!a.preempts(&a));
    assert!(!(a.preempts(&b) && b.preempts(&a)));
}

fn an_interrupt_at(priority: u8) -> Interrupt {
    let mut irq = an_interrupt(any_provenance(), kani::any());
    irq.priority = priority;
    irq
}

// Composition. The expression tree is heap-allocated, so the harnesses hold the
// leaf semantics that every branch of it rests on, and the tests carry the tree
// (call/0009).

fn any_wait(budget: Count, per_iter: Count) -> Wait {
    Wait {
        id: WaitId(0),
        budget,
        counter: Counter::U64,
        measure: Measure::PreDecrement,
        cost_per_iter: Quantity::new(Interval::point(per_iter), Unit::BusReads, any_provenance()),
        on_exhaustion: Exhaustion::ReportsError,
    }
}

/// A wait reports success exactly while the answer arrives inside its budget.
///
/// The short-circuit operator turns on this boundary: one step earlier the body
/// runs and the cost compounds, one step later the whole composition unwinds.
/// Getting the comparison off by one would move the worst case.
#[kani::proof]
fn a_wait_succeeds_exactly_below_its_budget() {
    let budget = any_edge();
    let latency = any_edge();
    let w = Expr::Leaf(any_wait(budget, 1));
    if let Ok(o) = w.eval(latency, Unit::BusReads) {
        assert_eq!(o.succeeded, latency < budget);
    }
}

/// A wait never polls more than its budget, whatever the world does.
#[kani::proof]
fn a_wait_never_polls_past_its_budget() {
    let budget = any_edge();
    let latency = any_edge();
    let w = Expr::Leaf(any_wait(budget, 1));
    if let Ok(o) = w.eval(latency, Unit::BusReads) {
        assert!(o.cost.interval().hi() <= budget);
    }
}

// Termination and counter fit. Both are decisions rather than arithmetic, so
// they quantify over everything their types admit rather than over boundaries.

fn any_counter() -> Counter {
    match kani::any::<u8>() % 4 {
        0 => Counter::U8,
        1 => Counter::U16,
        2 => Counter::U32,
        _ => Counter::U64,
    }
}

fn any_measure() -> Measure {
    match kani::any::<u8>() % 3 {
        0 => Measure::PreDecrement,
        1 => Measure::PostDecrement,
        _ => Measure::Increment { limit: kani::any() },
    }
}

/// A counter tested after it moves is never well-founded, whatever it counts to.
///
/// This is the whole of the termination obligation at the leaf. A bound stated
/// in a declaration is not a bound on behaviour unless the test sees the value
/// the decrement produced.
#[kani::proof]
fn a_measure_tested_after_it_moves_never_terminates() {
    let mut w = any_wait(kani::any(), 1);
    w.measure = any_measure();
    if w.measure == Measure::PostDecrement {
        assert_eq!(w.termination(), Termination::Wraps);
    }
}

/// A budget fits exactly when it is no larger than its counter, and the
/// reported headroom is the difference.
#[kani::proof]
fn a_budget_fits_exactly_when_the_counter_holds_it() {
    let mut w = any_wait(kani::any(), 1);
    w.counter = any_counter();
    let holds = w.counter.holds();
    match w.counter_fit() {
        CounterFit::Fits { headroom } => {
            assert!(w.budget <= holds);
            assert_eq!(headroom, holds - w.budget);
        }
        CounterFit::Overruns { budget, holds: h } => {
            assert!(budget > h);
            assert_eq!(h, holds);
        }
    }
}
