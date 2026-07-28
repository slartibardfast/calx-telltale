//! calx-telltale: a verified calculator for waits, interrupts-off windows, and
//! the compositions built out of them.
//!
//! The name is the watchman's tell-tale clock: an instrument fitted because a
//! report cannot be trusted.
//!
//! # What this crate holds
//!
//! The verified core. It is arithmetic over integer intervals carrying units
//! and provenance, and it has no dependencies, because a dependency here would
//! be a dependency inside the proof boundary.
//!
//! # The rules it exists to keep
//!
//! - **No bare integers.** Every number is a [`Quantity`]: an interval, a unit,
//!   and a provenance.
//! - **The weakest input wins.** A result carries the weakest provenance of
//!   anything in its derivation, so one guess makes the whole answer a guess.
//! - **Refuse rather than invent.** A count of loop passes has no conversion to
//!   time, and asking for one is an error rather than an estimate.
//! - **Exact rates.** A frequency is a reduced rational, so a clock at
//!   `105/88` MHz is held as it is rather than as the nearest integer hertz.
//! - **Compose in the base.** Where a register declares several clocks, the
//!   tool derives the frequency in which every declared period is a whole
//!   number of ticks, and composition there introduces no rounding at all.
//!
//! The core knows no particular clock. Interval timers, event timers and a
//! part-specific timer an adopter alone has are all declarations of the same
//! shape.

#![forbid(unsafe_code)]

/// The width every stored count carries.
///
/// Held in one place because the first choice was wrong. A narrower store was
/// justified on the grounds that it would keep the proofs cheaper, and the
/// obligation that width was supposed to help turned out to need a bounded
/// domain at any width, so the saving never existed. Meanwhile the narrower
/// store shortened the span a fine base can express and made the base
/// derivation give up on registers it should have served.
pub type Count = u128;

pub mod interrupt;
pub mod interval;
pub mod limits;
pub mod provenance;
pub mod quantity;
pub mod rate;
pub mod schedule;
pub mod source;

#[cfg(kani)]
mod proofs;

pub use interrupt::{
    Arrival, Consequence, Interrupt, InterruptId, Judgement, Missing, Sweep, Verdict,
};
pub use interval::Interval;
pub use limits::{Limit, LIMITS};
pub use provenance::Provenance;
pub use quantity::{Quantity, Refusal, Unit};
pub use rate::{Base, NoBase, Rate};
pub use schedule::{Analysis, Schedulability, Schedule};
pub use source::{Delay, Rounding, Source, SourceId, Span, Validity};
