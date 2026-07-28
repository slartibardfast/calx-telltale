//! What this tool does not model.
//!
//! Reported on every run, because an exclusion that is not stated reads as
//! coverage. A verdict withheld for a missing declaration is one kind of
//! honesty; naming the failure classes that sit outside the model entirely is
//! the other, and only this half can be written down in advance.

/// A failure class the model does not reach.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Limit {
    /// A short name a report can group by.
    pub name: &'static str,
    /// Why it sits outside, in the terms an operator would use.
    pub because: &'static str,
}

/// Every exclusion this tool carries.
///
/// A run prints these whether or not anything went wrong, so a clean sweep is
/// read as clean within a stated boundary rather than as clean absolutely.
pub const LIMITS: &[Limit] = &[
    Limit {
        name: "execute-in-place",
        because: "instruction fetch over serial flash makes execution time depend on flash \
                  latency and cache state, and no conversion from a code span to a time is \
                  available on such a part",
    },
    Limit {
        name: "hangs that are not loops",
        because: "a take on a semaphore nobody gives, a mutex nobody unlocks, or a work item \
                  never submitted hangs without a back edge, so no census of loops will find it",
    },
    Limit {
        name: "unresolved indirect calls",
        because: "calls through ops tables and device interfaces leave reachability unresolved, \
                  so any set of reachable waits is a lower bound rather than a census",
    },
    Limit {
        name: "windows across stack frames",
        because: "an interrupts-off region opened in one frame and closed in another is invisible \
                  to a per-function analysis, and the locked-wrapper pattern makes those common",
    },
    Limit {
        name: "correlated failure",
        because: "each clock's tolerance is modelled on its own, so a fault moving several \
                  clocks together is understated; the shared root is recorded, and the \
                  correlation is not yet carried into the arithmetic",
    },
    Limit {
        name: "nested budgets",
        because: "a wait knows its own bound and not what its callers have already spent, so a \
                  per-item bound holds while a walk over items has none",
    },
    Limit {
        name: "worst-case execution time",
        because: "a handler cost is declared rather than derived; this tool composes the number \
                  it is given and carries its provenance, and it does not compute one",
    },
];

/// How many exclusions the tool carries, for a report that wants the count
/// before the list.
#[must_use]
pub fn count() -> usize {
    LIMITS.len()
}

#[cfg(test)]
mod tests {
    use super::{count, LIMITS};

    #[test]
    fn every_limit_says_why() {
        assert!(count() >= 7);
        for l in LIMITS {
            assert!(!l.name.is_empty());
            assert!(
                l.because.len() > 40,
                "an exclusion with no reason is a shrug: {}",
                l.name
            );
        }
    }

    #[test]
    fn the_declared_scope_boundaries_are_present() {
        let names: Vec<&str> = LIMITS.iter().map(|l| l.name).collect();
        for expected in [
            "execute-in-place",
            "unresolved indirect calls",
            "correlated failure",
            "worst-case execution time",
        ] {
            assert!(names.contains(&expected), "missing exclusion: {expected}");
        }
    }
}
