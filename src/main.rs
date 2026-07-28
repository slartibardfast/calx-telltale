//! The command line.
//!
//! Two surfaces over one run, because an interface built for a human does not
//! serve an agent and a stable structured one does not serve a human reading by
//! eye (call/0003). The human surface is prose; the agent surface is `--json`,
//! and it carries the same facts as fields, provenance among them.
//!
//! The tool describes itself. An agent that has only the binary can reach the
//! commands, the exit codes, and the register grammar without reading anything
//! else, because a surface an agent cannot discover is one it will guess at.

use calx_telltale::expr::{CounterFit, Expr, Shape, Termination, Wait};
use calx_telltale::interrupt::Verdict;
use calx_telltale::limits::LIMITS;
use calx_telltale::quantity::Unit;
use calx_telltale::register::{Citation, Register};
use calx_telltale::Provenance;

const NAME: &str = env!("CARGO_PKG_NAME");
const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "\
calx-telltale — a verified calculator for waits, interrupts-off windows, and the
compositions built out of them.

usage:
  calx-telltale check <register> [--json]   hold each declaration to its obligations
  calx-telltale limits [--json]             the failure classes this tool does not model
  calx-telltale grammar                     the register format, for authoring one
  calx-telltale version [--json]
  calx-telltale help

options:
  --json    emit the same facts as a single JSON object rather than as prose.
            Every value carries the provenance it rests on, and the limits are
            included whether or not anything failed.

exit codes:
  0  every declaration held, or the command only reported something
  1  a declaration failed an obligation
  2  the register could not be read, or the command line made no sense

obligations checked:
  termination   the measure is well-founded, so a declared timeout can fire
  counter fit   the budget fits the counter holding it

provenance, weakest first:
  assumed < measured < extracted < derived
  A result carries the weakest provenance of anything it derives from, so one
  guessed input makes the whole answer a guess.
";

const GRAMMAR: &str = "\
A register is line-oriented. Blank lines are ignored, and everything after a `#`
is a comment, so a register can carry the commentary that makes it reviewable.

Each line is a kind followed by `key=value` fields in any order.

  source id=<n> hz=<n> [per=<n>] width=<bits> [ppm=<n>] from=<provenance>

    A declared clock. `hz` over `per` is an exact rational, because a frequency
    such as 105/88 MHz has no representation as a whole number of hertz. `per`
    defaults to 1 and `ppm` to 0. `width` is the counter's width in bits.

  wait id=<n> budget=<n> cost=<n> unit=<unit> counter=<u8|u16|u32|u64>
       measure=<pre-decrement|post-decrement|increment [limit=<n>]>
       on-exhaustion=<reports-error|asserts|silently-continues> from=<provenance>
       [file=<path>] [symbol=<name>]

    A polling wait. `measure` decides whether the loop terminates at all: a
    counter tested before it moves never sees zero, so it wraps and the declared
    budget bounds nothing.

  interrupt id=<n> priority=<n> cost=<n> unit=<unit> every=<n> depth=<n>
            on-drop=<lost-silently|lost-and-logged|retried> from=<provenance>
            [deadline=<n>] [armed=<from>..<to>] [jitter=<n>] [reenables=yes]

    A declared interrupt. A lower priority number preempts a higher one, the way
    the hardware numbers it. `armed` limits when the deadline is in force: a
    window outside it is reported unarmed rather than passed, because an
    unbounded stretch is worse than a bounded one. `jitter` is how late a
    release may be, which lets a burst land that even spacing would not.
    `reenables=yes` marks a handler its own priority level can preempt.

  window id=<n> cost=<n> unit=<unit> from=<provenance> [at=<from>..<to>]

    A span where interrupts are off. `at` places it on the timeline, so an
    armed deadline can be judged against it.

  compose id=<n> form=<seq|alt> of=<w0,c1,...>
  compose id=<n> form=repeat body=<ref> times=<n>
  compose id=<n> form=short-circuit guard=<ref> then=<ref>

    A composition. An operand names a wait as `wN` or an earlier composition as
    `cN`, and resolution runs top down, so a reference always points at
    something already declared.

values:
  <n>           decimal, or 0x-prefixed hex; `_` separators are ignored
  <unit>        iterations | bus-reads | base | nanos | ticks:<source id>
                A tick count must name the clock it was counted against, so a
                rate can never enter the model as a bare number.
  <provenance>  derived | extracted | measured | assumed
  file, symbol  where the declaration was read from. Optional, because a value
                that was swept or guessed has no site to name, and demanding one
                would push an author into inventing it. A verdict reports the
                site so a reader can open it rather than look up an identifier.
  ?             a blank an adapter left for a human. Reported as a blank rather
                than as a missing field, because one wants a decision and the
                other wants a correction.

example:
  source id=0 hz=105000000 per=88 width=32 ppm=100 from=extracted
  wait id=1 budget=10000 cost=1 unit=ticks:0 counter=u32 \\
       measure=post-decrement on-exhaustion=silently-continues from=extracted \\
       file=drivers/chan.c symbol=chan_teardown
";

/// Exit codes an agent acts on without reading prose.
mod code {
    pub const HELD: i32 = 0;
    pub const FAILED: i32 = 1;
    pub const UNREADABLE: i32 = 2;
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&str> = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with("--"))
        .collect();

    let status = match positional.first().copied() {
        // An agent guesses at a help flag before it guesses at a subcommand, so
        // every spelling it is likely to try has to arrive somewhere useful.
        Some("help" | "-h") | None => {
            print!("{USAGE}");
            code::HELD
        }
        Some("version") => {
            if json {
                println!("{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\"}}");
            } else {
                println!("{NAME} {VERSION}");
            }
            code::HELD
        }
        Some("grammar") => {
            print!("{GRAMMAR}");
            code::HELD
        }
        Some("limits") => {
            if json {
                println!(
                    "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"limits\":{}}}",
                    limits_json()
                );
            } else {
                print_limits();
            }
            code::HELD
        }
        Some("project") => match positional.get(1) {
            Some(path) => projection(path, json, false),
            None => want("project", "a register"),
        },
        Some("attain") => match positional.get(1) {
            Some(path) => projection(path, json, true),
            None => want("attain", "a register"),
        },
        Some("deadline") => match positional.get(1) {
            Some(path) => interrupts(path, json, false),
            None => want("deadline", "a register"),
        },
        Some("overrun") => match positional.get(1) {
            Some(path) => interrupts(path, json, true),
            None => want("overrun", "a register"),
        },
        Some("diff") => match (positional.get(1), positional.get(2)) {
            (Some(a), Some(b)) => diff(a, b, json),
            _ => want("diff", "two registers"),
        },
        Some("check") => match positional.get(1) {
            Some(path) => check(path, json),
            None => want("check", "a register"),
        },
        Some(other) => {
            eprintln!("no such command: {other}\n\n{USAGE}");
            code::UNREADABLE
        }
    };
    std::process::ExitCode::from(u8::try_from(status).unwrap_or(2))
}

fn want(verb: &str, what: &str) -> i32 {
    eprintln!("{verb} needs {what} to read\n\n{USAGE}");
    code::UNREADABLE
}

/// JSON string escaping, which is all the encoding this crate needs and less
/// than a dependency would cost. The offline reproducible build depends on the
/// dependency list staying empty.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn limits_json() -> String {
    let items: Vec<String> = LIMITS
        .iter()
        .map(|l| {
            format!(
                "{{\"name\":\"{}\",\"because\":\"{}\"}}",
                esc(l.name),
                esc(l.because)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

fn print_limits() {
    println!(
        "limits: {} class(es) this tool does not model",
        LIMITS.len()
    );
    for l in LIMITS {
        println!("  {}\n    {}", l.name, l.because);
    }
}

/// One obligation held against one declaration.
struct Finding {
    wait: u16,
    /// Where the declaration was read from, so a reader can open the site
    /// rather than look up an identifier the register alone explains.
    site: String,
    provenance: Provenance,
    obligation: &'static str,
    held: bool,
    /// The number the verdict turned on, as a string because a count is wider
    /// than a JSON number is guaranteed to carry.
    detail: String,
    says: String,
}

fn findings_for(w: &Wait, citation: &Citation) -> Vec<Finding> {
    let provenance = w.cost_per_iter.provenance();
    let site = citation.site();
    let mut out = Vec::new();

    let (held, detail, says) = match w.termination() {
        Termination::WellFounded => (true, String::new(), "the measure is well-founded".into()),
        Termination::Wraps => (
            false,
            String::new(),
            "the counter is tested before it moves, so it passes zero untested and wraps".into(),
        ),
        Termination::NeverReaches => (
            false,
            String::new(),
            "the counter rises to a limit it never reaches".into(),
        ),
    };
    out.push(Finding {
        wait: w.id.0,
        site: site.clone(),
        provenance,
        obligation: "termination",
        held,
        detail,
        says,
    });

    let (held, detail, says) = match w.counter_fit() {
        CounterFit::Fits { headroom } => (
            true,
            headroom.to_string(),
            format!("the budget fits, with {headroom} of headroom"),
        ),
        CounterFit::Overruns { budget, holds } => (
            false,
            budget.to_string(),
            format!("budget {budget} exceeds the {holds} its counter holds"),
        ),
    };
    out.push(Finding {
        wait: w.id.0,
        site,
        provenance,
        obligation: "counter-fit",
        held,
        detail,
        says,
    });

    out
}

/// The unit a composition counts in, taken from the first wait inside it. A
/// composition mixing units is refused by the arithmetic rather than here.
fn unit_of(e: &Expr) -> Option<Unit> {
    match e {
        Expr::Leaf(w) => Some(w.cost_per_iter.unit()),
        Expr::Seq(parts) | Expr::Alt(parts) => parts.iter().find_map(unit_of),
        Expr::Repeat { body, .. } => unit_of(body),
        Expr::ShortCircuit { guard, then } => unit_of(guard).or_else(|| unit_of(then)),
    }
}

fn read(path: &str, json: bool) -> Result<Register, i32> {
    let text = std::fs::read_to_string(path).map_err(|e| unreadable(path, &e.to_string(), json))?;
    Register::parse(&text).map_err(|e| unreadable(path, &e.to_string(), json))
}

/// `project` and `attain` read the same search and report different halves of
/// it, so they share one walk.
fn projection(path: &str, json: bool, witness: bool) -> i32 {
    let register = match read(path, json) {
        Ok(r) => r,
        Err(c) => return c,
    };
    let mut rows: Vec<String> = Vec::new();
    let mut human: Vec<String> = Vec::new();
    for (id, expr) in &register.compositions {
        let Some(u) = unit_of(expr) else {
            human.push(format!(
                "  composition {id}: empty, so there is nothing to price"
            ));
            rows.push(format!("{{\"composition\":{id},\"verdict\":\"empty\"}}"));
            continue;
        };
        match expr.attain(u) {
            Err(e) => {
                human.push(format!("  composition {id}: refused, {e:?}"));
                rows.push(format!(
                    "{{\"composition\":{id},\"verdict\":\"refused\",\"says\":\"{}\"}}",
                    esc(&format!("{e:?}"))
                ));
            }
            Ok(a) => {
                let cost = a.cost.interval().hi();
                let prov = a.cost.provenance().as_str();
                if witness {
                    human.push(format!(
                        "  composition {id} [{prov}] worst case {cost}, attained at latency {}{}",
                        a.witness,
                        match (a.shape, a.interior) {
                            (Shape::Monotone, _) =>
                                " (established: the cost cannot fall, so no search was needed)",
                            (Shape::EarlyExit, true) =>
                                " (searched, interior: a sweep of the extremes would have missed it)",
                            (Shape::EarlyExit, false) => " (searched, at the boundary)",
                        }
                    ));
                } else {
                    human.push(format!("  composition {id} [{prov}] worst case {cost}"));
                }
                rows.push(format!(
                    "{{\"composition\":{id},\"verdict\":\"priced\",\"cost\":\"{cost}\",\"provenance\":\"{prov}\",\"witness\":\"{}\",\"interior\":{},\"how\":\"{}\"}}",
                    a.witness,
                    a.interior,
                    match a.shape {
                        Shape::Monotone => "established",
                        Shape::EarlyExit => "searched",
                    }
                ));
            }
        }
    }
    if json {
        println!(
            "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"register\":\"{}\",\"verb\":\"{}\",\"compositions\":[{}],\"limits\":{}}}",
            esc(path),
            if witness { "attain" } else { "project" },
            rows.join(","),
            limits_json()
        );
    } else {
        println!("register {path}");
        if human.is_empty() {
            println!("  no compositions declared");
        }
        for l in &human {
            println!("{l}");
        }
        print_limits();
    }
    code::HELD
}

/// `deadline` and `overrun` ask different questions of the same pairing, so
/// they share one walk too.
fn interrupts(path: &str, json: bool, overrun: bool) -> i32 {
    let register = match read(path, json) {
        Ok(r) => r,
        Err(c) => return c,
    };
    let mut rows: Vec<String> = Vec::new();
    let mut human: Vec<String> = Vec::new();
    let mut missed = 0usize;
    let mut withheld = 0usize;

    for irq in &register.interrupts {
        for (wid, window, span) in &register.windows {
            let j = if overrun {
                irq.overrun(*window)
            } else {
                irq.latency(*window, *span)
            };
            let (verdict, says) = match j.verdict {
                Verdict::Met => ("met", "the bound holds".to_string()),
                Verdict::Missed => {
                    missed += 1;
                    ("missed", "the bound can be breached".to_string())
                }
                Verdict::Unarmed => {
                    withheld += 1;
                    (
                        "unarmed",
                        "no deadline is in force across this span, so nothing bounds it"
                            .to_string(),
                    )
                }
                Verdict::Unanswerable(m) => {
                    withheld += 1;
                    ("withheld", format!("{m:?}"))
                }
            };
            let measured = j
                .measured
                .map(|q| q.interval().hi().to_string())
                .unwrap_or_default();
            human.push(format!(
                "  interrupt {} against window {wid} [{}] {verdict}: {says}{}",
                irq.id.0,
                j.provenance.as_str(),
                if measured.is_empty() {
                    String::new()
                } else {
                    format!(" ({measured})")
                }
            ));
            rows.push(format!(
                "{{\"interrupt\":{},\"window\":{wid},\"verdict\":\"{verdict}\",\"provenance\":\"{}\",\"measured\":\"{measured}\",\"says\":\"{}\"}}",
                irq.id.0,
                j.provenance.as_str(),
                esc(&says)
            ));
        }
    }
    let status = if missed == 0 {
        code::HELD
    } else {
        code::FAILED
    };
    if json {
        println!(
            "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"register\":\"{}\",\"verb\":\"{}\",\"verdicts\":[{}],\"missed\":{missed},\"withheld\":{withheld},\"exit\":{status},\"limits\":{}}}",
            esc(path),
            if overrun { "overrun" } else { "deadline" },
            rows.join(","),
            limits_json()
        );
    } else {
        println!("register {path}");
        if human.is_empty() {
            println!("  no interrupt and window pair declared");
        }
        for l in &human {
            println!("{l}");
        }
        // A withheld verdict is not a pass, so the count is stated even when
        // nothing was missed.
        println!("  {missed} missed, {withheld} withheld");
        print_limits();
    }
    status
}

/// `diff` reports what moved between two registers, so a reviewer sees the
/// change rather than the state.
fn diff(a_path: &str, b_path: &str, json: bool) -> i32 {
    let (a, b) = match (read(a_path, json), read(b_path, json)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(c), _) | (_, Err(c)) => return c,
    };
    let mut rows: Vec<String> = Vec::new();
    let mut human: Vec<String> = Vec::new();
    for wa in &a.waits {
        match b.waits.iter().find(|w| w.id == wa.id) {
            None => {
                human.push(format!("  wait {} removed", wa.id.0));
                rows.push(format!("{{\"wait\":{},\"change\":\"removed\"}}", wa.id.0));
            }
            Some(wb) => {
                if wa.budget != wb.budget {
                    human.push(format!(
                        "  wait {} budget moved, {} to {}",
                        wa.id.0, wa.budget, wb.budget
                    ));
                    rows.push(format!(
                        "{{\"wait\":{},\"change\":\"budget\",\"was\":\"{}\",\"now\":\"{}\"}}",
                        wa.id.0, wa.budget, wb.budget
                    ));
                }
                if wa.measure != wb.measure {
                    human.push(format!("  wait {} measure changed", wa.id.0));
                    rows.push(format!("{{\"wait\":{},\"change\":\"measure\"}}", wa.id.0));
                }
                if wa.counter != wb.counter {
                    human.push(format!("  wait {} counter width changed", wa.id.0));
                    rows.push(format!("{{\"wait\":{},\"change\":\"counter\"}}", wa.id.0));
                }
            }
        }
    }
    for wb in &b.waits {
        if !a.waits.iter().any(|w| w.id == wb.id) {
            human.push(format!("  wait {} added", wb.id.0));
            rows.push(format!("{{\"wait\":{},\"change\":\"added\"}}", wb.id.0));
        }
    }
    if json {
        println!(
            "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"from\":\"{}\",\"to\":\"{}\",\"changes\":[{}]}}",
            esc(a_path),
            esc(b_path),
            rows.join(",")
        );
    } else {
        println!("{a_path} -> {b_path}");
        if human.is_empty() {
            println!("  no declaration moved");
        }
        for l in &human {
            println!("{l}");
        }
    }
    code::HELD
}

fn check(path: &str, json: bool) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return unreadable(path, &e.to_string(), json),
    };
    let register = match Register::parse(&text) {
        Ok(r) => r,
        Err(e) => return unreadable(path, &e.to_string(), json),
    };

    let findings: Vec<Finding> = register
        .waits
        .iter()
        .enumerate()
        .flat_map(|(i, w)| findings_for(w, &register.citation_for(i)))
        .collect();
    let failed = findings.iter().filter(|f| !f.held).count();
    let standing = register.waits.iter().fold(Provenance::Derived, |acc, w| {
        acc.join(w.cost_per_iter.provenance())
    });
    let status = if failed == 0 {
        code::HELD
    } else {
        code::FAILED
    };

    if json {
        let items: Vec<String> = findings
            .iter()
            .map(|f| {
                format!(
                    "{{\"wait\":{},\"site\":\"{}\",\"obligation\":\"{}\",\"verdict\":\"{}\",\"provenance\":\"{}\",\"detail\":\"{}\",\"says\":\"{}\"}}",
                    f.wait,
                    esc(&f.site),
                    f.obligation,
                    if f.held { "held" } else { "failed" },
                    f.provenance.as_str(),
                    esc(&f.detail),
                    esc(&f.says)
                )
            })
            .collect();
        println!(
            "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"register\":\"{}\",\
             \"clocks\":{},\"waits\":{},\"findings\":[{}],\"standing\":\"{}\",\
             \"failed\":{},\"verdict\":\"{}\",\"exit\":{},\"limits\":{}}}",
            esc(path),
            register.sources.len(),
            register.waits.len(),
            items.join(","),
            standing.as_str(),
            failed,
            if failed == 0 { "held" } else { "failed" },
            status,
            limits_json()
        );
        return status;
    }

    println!("register {path}");
    println!(
        "  {} clock(s), {} wait(s)",
        register.sources.len(),
        register.waits.len()
    );
    for f in &findings {
        println!(
            "  wait {} ({}) [{}] {}: {}",
            f.wait,
            f.site,
            f.provenance.as_str(),
            if f.held {
                f.obligation.to_string()
            } else {
                format!("{} FAILED", f.obligation)
            },
            f.says
        );
    }
    // The weakest input reaches the verdict, so a report resting on a guess
    // says so before anyone acts on it.
    println!("  standing: {}", standing.as_str());
    print_limits();
    if failed == 0 {
        println!("held: every declaration met its obligations, within the limits above");
    } else {
        println!("failed: {failed} obligation(s) not met");
    }
    status
}

fn unreadable(path: &str, why: &str, json: bool) -> i32 {
    if json {
        println!(
            "{{\"tool\":\"{NAME}\",\"version\":\"{VERSION}\",\"register\":\"{}\",\
             \"verdict\":\"unreadable\",\"says\":\"{}\",\"exit\":{}}}",
            esc(path),
            esc(why),
            code::UNREADABLE
        );
    } else {
        eprintln!("{path}: {why}");
    }
    code::UNREADABLE
}
