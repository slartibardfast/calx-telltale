//! The declaration everything else computes over, and its file format.
//!
//! Line-oriented on purpose. The register is the artefact that gets committed
//! beside the image it describes, so it has to diff cleanly, review in a pull
//! request, and be editable by hand where an adapter left a blank. A nested
//! format would read better and diff worse, and diffing is the operation that
//! happens most.
//!
//! The parser takes no dependency, for the same reason the rest of the crate
//! takes none: a format this small is not worth widening the surface for.

use crate::expr::{Counter, Exhaustion, Expr, Measure, Wait, WaitId};
use crate::interrupt::{Arrival, Consequence, Deadline, Interrupt, InterruptId};
use crate::interval::Interval;
use crate::provenance::Provenance;
use crate::quantity::{Quantity, Unit};
use crate::rate::Rate;
use crate::source::{Origin, Source, SourceId, Span, Validity};
use crate::Count;

/// Why a register could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    /// The line it happened on, counted from one, so a message can point at it.
    pub line: usize,
    pub what: Fault,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// A line the grammar has no rule for.
    Unrecognised,
    /// A field the declaration requires and this line does not carry.
    Missing(&'static str),
    /// A field that is present and unreadable.
    Malformed(&'static str),
    /// A blank the adapter left for a human, still blank.
    StillBlank(&'static str),
    /// A tick or cycle count that names no declared clock, which would put a
    /// rate into the model as a bare number.
    UnnamedClock,
    /// An operand naming a declaration this register has not made yet. A
    /// composition reads top down, so what it refers to comes first.
    UnknownOperand(&'static str),
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.what {
            Fault::Unrecognised => write!(f, "no rule for this line"),
            Fault::Missing(k) => write!(f, "missing {k}"),
            Fault::Malformed(k) => write!(f, "malformed {k}"),
            Fault::StillBlank(k) => write!(f, "{k} is still blank; a human has to supply it"),
            Fault::UnnamedClock => {
                write!(f, "a tick count must name the clock it was counted against")
            }
            Fault::UnknownOperand(k) => write!(
                f,
                "{k} names a declaration this register has not made yet; a composition reads top down"
            ),
        }
    }
}

/// Where a declaration was read from.
///
/// Held here rather than inside `Provenance`, because a citation is free text
/// and [call/0006] keeps free text outside the proof boundary. `Provenance`
/// says how a value was arrived at; this says where it was found, and a verdict
/// wants both: one to know what the number is worth, the other to know what to
/// open.
///
/// Optional, because a value that was swept or guessed has no symbol to name,
/// and demanding one would push an author into inventing it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Citation {
    pub file: Option<String>,
    pub symbol: Option<String>,
}

impl Citation {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.file.is_none() && self.symbol.is_none()
    }

    /// How a report names the site, or that there is none to name.
    #[must_use]
    pub fn site(&self) -> String {
        match (&self.file, &self.symbol) {
            (Some(f), Some(s)) => format!("{f}:{s}"),
            (Some(f), None) => f.clone(),
            (None, Some(s)) => s.clone(),
            (None, None) => "no citation".to_string(),
        }
    }
}

/// A parsed register.
///
/// Citations sit alongside the declarations rather than inside them, so the
/// verified types stay free of text and the reporting layer still has what it
/// needs. The index of a citation matches the index of its declaration.
#[derive(Clone, Debug, Default)]
pub struct Register {
    pub sources: Vec<Source>,
    pub waits: Vec<Wait>,
    /// One per wait, in the same order.
    pub wait_citations: Vec<Citation>,
    /// Compositions, each with the identifier it was declared under.
    pub compositions: Vec<(u16, Expr)>,
    pub interrupts: Vec<Interrupt>,
    /// Blackout windows: spans where interrupts are off.
    pub windows: Vec<(u16, Quantity, Span)>,
}

impl Register {
    /// The citation for a wait, by its position.
    #[must_use]
    pub fn citation_for(&self, index: usize) -> Citation {
        self.wait_citations.get(index).cloned().unwrap_or_default()
    }
}

/// One `key = value` field from a line.
fn fields(rest: &str) -> Vec<(&str, &str)> {
    rest.split_whitespace()
        .filter_map(|tok| tok.split_once('='))
        .collect()
}

fn get<'a>(fs: &[(&'a str, &'a str)], key: &'static str) -> Result<&'a str, Fault> {
    match fs.iter().find(|(k, _)| *k == key) {
        None => Err(Fault::Missing(key)),
        // A blank an adapter left is a different finding from a missing field:
        // one is an incomplete declaration, the other a malformed one.
        Some((_, v)) if *v == "?" => Err(Fault::StillBlank(key)),
        Some((_, v)) => Ok(*v),
    }
}

fn number(fs: &[(&str, &str)], key: &'static str) -> Result<Count, Fault> {
    let raw = get(fs, key)?;
    let (digits, radix) = match raw.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None => (raw, 10),
    };
    Count::from_str_radix(&digits.replace('_', ""), radix).map_err(|_| Fault::Malformed(key))
}

fn citation(fs: &[(&str, &str)]) -> Citation {
    let pick = |k: &str| {
        fs.iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| (*v).to_string())
            .filter(|v| v != "?")
    };
    Citation {
        file: pick("file"),
        symbol: pick("symbol"),
    }
}

fn provenance(fs: &[(&str, &str)]) -> Result<Provenance, Fault> {
    match get(fs, "from")? {
        "derived" => Ok(Provenance::Derived),
        "extracted" => Ok(Provenance::Extracted),
        "measured" => Ok(Provenance::Measured),
        "assumed" => Ok(Provenance::Assumed),
        _ => Err(Fault::Malformed("from")),
    }
}

fn unit(fs: &[(&str, &str)], sources: &[Source]) -> Result<Unit, Fault> {
    let raw = get(fs, "unit")?;
    match raw {
        "iterations" => Ok(Unit::Iterations),
        "bus-reads" => Ok(Unit::BusReads),
        "base" => Ok(Unit::Base),
        "nanos" => Ok(Unit::Nanos),
        other => {
            // `ticks:<clock>` names the clock, because a tick count that named
            // none would be a rate entering the model as a bare number.
            let Some(name) = other.strip_prefix("ticks:") else {
                return Err(Fault::Malformed("unit"));
            };
            let idx = name.parse::<u16>().map_err(|_| Fault::UnnamedClock)?;
            if sources.iter().any(|s| s.id == SourceId(idx)) {
                Ok(Unit::Ticks(SourceId(idx)))
            } else {
                Err(Fault::UnnamedClock)
            }
        }
    }
}

impl Register {
    /// Read a register from its file form.
    ///
    /// Blank lines and everything after a `#` are ignored, so a register can
    /// carry the commentary that makes it reviewable.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut reg = Register::default();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (kind, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let fs = fields(rest);
            let at = |what: Fault| ParseError { line: n + 1, what };

            match kind {
                "source" => {
                    let id = number(&fs, "id").map_err(at)?;
                    let hz_num = number(&fs, "hz").map_err(at)?;
                    let hz_den = match number(&fs, "per") {
                        Ok(v) => v,
                        Err(Fault::Missing(_)) => 1,
                        Err(e) => return Err(at(e)),
                    };
                    let rate =
                        Rate::new(hz_num, hz_den).ok_or_else(|| at(Fault::Malformed("hz")))?;
                    let width = number(&fs, "width").map_err(at)?;
                    let ppm = match number(&fs, "ppm") {
                        Ok(v) => v,
                        Err(Fault::Missing(_)) => 0,
                        Err(e) => return Err(at(e)),
                    };
                    reg.sources.push(Source {
                        id: SourceId(u16::try_from(id).map_err(|_| at(Fault::Malformed("id")))?),
                        origin: Origin::Root,
                        nominal: rate,
                        nominal_prov: provenance(&fs).map_err(at)?,
                        tolerance_ppm: u32::try_from(ppm)
                            .map_err(|_| at(Fault::Malformed("ppm")))?,
                        width_bits: u8::try_from(width)
                            .map_err(|_| at(Fault::Malformed("width")))?,
                        read_cost: Quantity::new(
                            Interval::point(1),
                            Unit::BusReads,
                            Provenance::Assumed,
                        ),
                        valid: Validity::Always,
                    });
                }
                "wait" => {
                    let id = number(&fs, "id").map_err(at)?;
                    let budget = number(&fs, "budget").map_err(at)?;
                    let cost = number(&fs, "cost").map_err(at)?;
                    let u = unit(&fs, &reg.sources).map_err(at)?;
                    let counter = match get(&fs, "counter").map_err(at)? {
                        "u8" => Counter::U8,
                        "u16" => Counter::U16,
                        "u32" => Counter::U32,
                        "u64" => Counter::U64,
                        _ => return Err(at(Fault::Malformed("counter"))),
                    };
                    let measure = match get(&fs, "measure").map_err(at)? {
                        "pre-decrement" => Measure::PreDecrement,
                        "post-decrement" => Measure::PostDecrement,
                        "increment" => Measure::Increment {
                            limit: number(&fs, "limit").map_err(at)?,
                        },
                        _ => return Err(at(Fault::Malformed("measure"))),
                    };
                    let on_exhaustion = match get(&fs, "on-exhaustion").map_err(at)? {
                        "reports-error" => Exhaustion::ReportsError,
                        "asserts" => Exhaustion::Asserts,
                        "silently-continues" => Exhaustion::SilentlyContinues,
                        _ => return Err(at(Fault::Malformed("on-exhaustion"))),
                    };
                    reg.wait_citations.push(citation(&fs));
                    reg.waits.push(Wait {
                        id: WaitId(u16::try_from(id).map_err(|_| at(Fault::Malformed("id")))?),
                        budget,
                        counter,
                        measure,
                        cost_per_iter: Quantity::new(
                            Interval::point(cost),
                            u,
                            provenance(&fs).map_err(at)?,
                        ),
                        on_exhaustion,
                    });
                }
                "window" => {
                    let id = number(&fs, "id").map_err(at)?;
                    let cost = number(&fs, "cost").map_err(at)?;
                    let u = unit(&fs, &reg.sources).map_err(at)?;
                    let span = match get(&fs, "at") {
                        Err(Fault::Missing(_)) => Span::new(0, 0).expect("zero is a span"),
                        Err(e) => return Err(at(e)),
                        Ok(raw) => {
                            let (from, to) = raw
                                .split_once("..")
                                .ok_or(Fault::Malformed("at"))
                                .map_err(at)?;
                            let parse = |v: &str| {
                                v.parse::<Count>().map_err(|_| at(Fault::Malformed("at")))
                            };
                            Span::new(parse(from)?, parse(to)?)
                                .ok_or(Fault::Malformed("at"))
                                .map_err(at)?
                        }
                    };
                    reg.windows.push((
                        u16::try_from(id).map_err(|_| at(Fault::Malformed("id")))?,
                        Quantity::new(Interval::point(cost), u, provenance(&fs).map_err(at)?),
                        span,
                    ));
                }
                "compose" => {
                    let id = number(&fs, "id").map_err(at)?;
                    let form = get(&fs, "form").map_err(at)?;
                    // An operand names a wait as `wN` or an earlier composition
                    // as `cN`. Resolution is top down, so a reference always
                    // points at something already declared.
                    let resolve = |name: &str| -> Option<Expr> {
                        let (kind, n) = name.split_at(1);
                        let n: u16 = n.parse().ok()?;
                        match kind {
                            "w" => reg
                                .waits
                                .iter()
                                .find(|w| w.id == WaitId(n))
                                .map(|w| Expr::Leaf(*w)),
                            "c" => reg
                                .compositions
                                .iter()
                                .find(|(cid, _)| *cid == n)
                                .map(|(_, e)| e.clone()),
                            _ => None,
                        }
                    };
                    let list = |key: &'static str| -> Result<Vec<Expr>, Fault> {
                        get(&fs, key)?
                            .split(',')
                            .map(|n| resolve(n).ok_or(Fault::UnknownOperand(key)))
                            .collect()
                    };
                    let expr = match form {
                        "seq" => Expr::Seq(list("of").map_err(at)?),
                        "alt" => Expr::Alt(list("of").map_err(at)?),
                        "repeat" => Expr::Repeat {
                            body: Box::new(
                                resolve(get(&fs, "body").map_err(at)?)
                                    .ok_or(Fault::UnknownOperand("body"))
                                    .map_err(at)?,
                            ),
                            times: number(&fs, "times").map_err(at)?,
                        },
                        "short-circuit" => Expr::ShortCircuit {
                            guard: Box::new(
                                resolve(get(&fs, "guard").map_err(at)?)
                                    .ok_or(Fault::UnknownOperand("guard"))
                                    .map_err(at)?,
                            ),
                            then: Box::new(
                                resolve(get(&fs, "then").map_err(at)?)
                                    .ok_or(Fault::UnknownOperand("then"))
                                    .map_err(at)?,
                            ),
                        },
                        _ => return Err(at(Fault::Malformed("form"))),
                    };
                    reg.compositions.push((
                        u16::try_from(id).map_err(|_| at(Fault::Malformed("id")))?,
                        expr,
                    ));
                }
                "interrupt" => {
                    let id = number(&fs, "id").map_err(at)?;
                    let u = unit(&fs, &reg.sources).map_err(at)?;
                    let p = provenance(&fs).map_err(at)?;
                    let q = |v: Count| Quantity::new(Interval::point(v), u, p);
                    // `armed=<from>..<to>` limits when the bound is in force.
                    // Absent, it is in force throughout.
                    let armed = match get(&fs, "armed") {
                        Err(Fault::Missing(_)) => Validity::Always,
                        Err(e) => return Err(at(e)),
                        Ok(raw) => {
                            let (from, to) = raw
                                .split_once("..")
                                .ok_or(Fault::Malformed("armed"))
                                .map_err(at)?;
                            let parse = |v: &str| {
                                v.parse::<Count>()
                                    .map_err(|_| at(Fault::Malformed("armed")))
                            };
                            Validity::Over(
                                Span::new(parse(from)?, parse(to)?)
                                    .ok_or(Fault::Malformed("armed"))
                                    .map_err(at)?,
                            )
                        }
                    };
                    let deadline = match number(&fs, "deadline") {
                        Ok(v) => Some(Deadline {
                            budget: q(v),
                            armed,
                        }),
                        Err(Fault::Missing(_)) => None,
                        Err(e) => return Err(at(e)),
                    };
                    let jitter = match number(&fs, "jitter") {
                        Ok(v) => Some(q(v)),
                        Err(Fault::Missing(_)) => None,
                        Err(e) => return Err(at(e)),
                    };
                    let reenables = matches!(get(&fs, "reenables"), Ok("yes"));
                    reg.interrupts.push(Interrupt {
                        id: InterruptId(u16::try_from(id).map_err(|_| at(Fault::Malformed("id")))?),
                        arrival: Arrival::MinInterarrival(q(number(&fs, "every").map_err(at)?)),
                        cost: q(number(&fs, "cost").map_err(at)?),
                        priority: u8::try_from(number(&fs, "priority").map_err(at)?)
                            .map_err(|_| at(Fault::Malformed("priority")))?,
                        deadline,
                        jitter,
                        reenables,
                        depth: u32::try_from(number(&fs, "depth").map_err(at)?)
                            .map_err(|_| at(Fault::Malformed("depth")))?,
                        on_drop: match get(&fs, "on-drop").map_err(at)? {
                            "lost-silently" => Consequence::LostSilently,
                            "lost-and-logged" => Consequence::LostAndLogged,
                            "retried" => Consequence::Retried,
                            _ => return Err(at(Fault::Malformed("on-drop"))),
                        },
                    });
                }
                _ => return Err(at(Fault::Unrecognised)),
            }
        }
        Ok(reg)
    }
}

#[cfg(test)]
mod tests {
    use super::{Fault, Register};
    use crate::expr::{Counter, Measure, Termination};
    use crate::provenance::Provenance;
    use crate::quantity::Unit;
    use crate::source::SourceId;

    const SAMPLE: &str = "\
# A part with one clock and two waits.
source id=0 hz=105000000 per=88 width=32 ppm=100 from=extracted

# A poll that gives up properly.
wait id=0 budget=8192 cost=1 unit=bus-reads counter=u32 measure=pre-decrement \
on-exhaustion=reports-error from=derived

# A teardown that does not.
wait id=1 budget=10000 cost=1 unit=ticks:0 counter=u32 measure=post-decrement \
on-exhaustion=silently-continues from=extracted
";

    #[test]
    fn a_register_round_trips_its_declarations() {
        let r = Register::parse(SAMPLE).unwrap();
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.waits.len(), 2);
        // The clock is held exactly rather than rounded to whole hertz.
        assert_eq!(
            (r.sources[0].nominal.num(), r.sources[0].nominal.den()),
            (13_125_000, 11)
        );
        assert_eq!(r.sources[0].tolerance_ppm, 100);
        assert_eq!(r.waits[0].counter, Counter::U32);
        assert_eq!(r.waits[1].measure, Measure::PostDecrement);
        assert_eq!(r.waits[1].cost_per_iter.unit(), Unit::Ticks(SourceId(0)));
        assert_eq!(r.waits[0].cost_per_iter.provenance(), Provenance::Derived);
    }

    #[test]
    fn the_parsed_register_carries_the_finding_through() {
        // The point of parsing is to reach a verdict, so the wait that cannot
        // terminate is visible from the file alone.
        let r = Register::parse(SAMPLE).unwrap();
        assert_eq!(r.waits[0].termination(), Termination::WellFounded);
        assert_eq!(r.waits[1].termination(), Termination::Wraps);
    }

    #[test]
    fn a_declaration_carries_where_it_was_read_from() {
        let text = "wait id=0 budget=8192 cost=1 unit=bus-reads counter=u32 \
                    measure=pre-decrement on-exhaustion=asserts from=extracted \
                    file=drivers/bus.c symbol=bus_wait_idle";
        let r = Register::parse(text).unwrap();
        let c = r.citation_for(0);
        assert_eq!(c.file.as_deref(), Some("drivers/bus.c"));
        assert_eq!(c.symbol.as_deref(), Some("bus_wait_idle"));
        assert_eq!(c.site(), "drivers/bus.c:bus_wait_idle");
    }

    #[test]
    fn a_declaration_without_a_citation_still_parses() {
        // A swept or guessed value has no symbol to name, so demanding one
        // would push an author into inventing it.
        let r = Register::parse(SAMPLE).unwrap();
        assert!(r.citation_for(0).is_empty());
        assert_eq!(r.citation_for(0).site(), "no citation");
    }

    #[test]
    fn a_blank_citation_is_absent_rather_than_the_word_it_was_written_as() {
        // A census emits `?` where it could not determine a site. That reaches
        // the reader as absent rather than as a literal question mark.
        let text = "wait id=0 budget=1 cost=1 unit=bus-reads counter=u32 \
                    measure=pre-decrement on-exhaustion=asserts from=assumed \
                    file=? symbol=?";
        let r = Register::parse(text).unwrap();
        assert!(r.citation_for(0).is_empty());
    }

    #[test]
    fn a_tick_count_naming_no_clock_is_refused() {
        let bad = "wait id=0 budget=1 cost=1 unit=ticks:7 counter=u32 \
                   measure=pre-decrement on-exhaustion=asserts from=derived";
        assert_eq!(Register::parse(bad).unwrap_err().what, Fault::UnnamedClock);
    }

    #[test]
    fn a_blank_the_adapter_left_is_named_as_a_blank() {
        // A census emits `?` where a human has to decide. That is a different
        // finding from a field nobody wrote, and the message says which.
        let blank = "wait id=0 budget=? cost=1 unit=bus-reads counter=u32 \
                     measure=pre-decrement on-exhaustion=asserts from=derived";
        assert_eq!(
            Register::parse(blank).unwrap_err().what,
            Fault::StillBlank("budget")
        );
        let absent = "wait id=0 cost=1 unit=bus-reads counter=u32 \
                      measure=pre-decrement on-exhaustion=asserts from=derived";
        assert_eq!(
            Register::parse(absent).unwrap_err().what,
            Fault::Missing("budget")
        );
    }

    #[test]
    fn an_error_points_at_the_line_it_happened_on() {
        let text = "source id=0 hz=1000 width=32 from=derived\nnonsense here\n";
        let e = Register::parse(text).unwrap_err();
        assert_eq!(e.line, 2);
        assert_eq!(e.what, Fault::Unrecognised);
        assert!(e.to_string().starts_with("line 2:"));
    }

    #[test]
    fn hexadecimal_and_underscores_are_read_the_way_they_are_written() {
        let text = "wait id=0 budget=0x2000 cost=1_000 unit=bus-reads counter=u16 \
                    measure=pre-decrement on-exhaustion=asserts from=extracted";
        let r = Register::parse(text).unwrap();
        assert_eq!(r.waits[0].budget, 0x2000);
        assert_eq!(r.waits[0].cost_per_iter.interval().hi(), 1_000);
    }

    #[test]
    fn commentary_survives_because_a_register_has_to_be_reviewable() {
        let text = "# why this budget\nsource id=0 hz=1000 width=32 from=derived # inline\n";
        assert_eq!(Register::parse(text).unwrap().sources.len(), 1);
    }
}
