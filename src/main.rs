//! The command line.
//!
//! Two surfaces, one run. A human reads the report; an agent reads the same
//! facts as fields and the exit code. Every result carries the provenance it
//! rests on, and the limits are stated whether or not anything went wrong.

use calx_telltale::expr::{CounterFit, Termination};
use calx_telltale::limits::LIMITS;
use calx_telltale::register::Register;
use calx_telltale::Provenance;

const USAGE: &str = "\
calx-telltale — a verified calculator for waits and interrupts-off windows

usage:
  calx-telltale check <register>   hold each declaration to its obligations
  calx-telltale limits             what this tool does not model
  calx-telltale help

exit codes:
  0  every declaration held
  1  a declaration failed an obligation
  2  the register could not be read
";

/// Exit codes an agent can act on without parsing prose.
mod code {
    pub const HELD: i32 = 0;
    pub const FAILED: i32 = 1;
    pub const UNREADABLE: i32 = 2;
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let status = match args.first().map(String::as_str) {
        Some("check") => match args.get(1) {
            Some(path) => check(path),
            None => {
                eprintln!("check needs a register to read");
                code::UNREADABLE
            }
        },
        Some("limits") => {
            print_limits();
            code::HELD
        }
        Some("help") | None => {
            print!("{USAGE}");
            code::HELD
        }
        Some(other) => {
            eprintln!("no such command: {other}\n\n{USAGE}");
            code::UNREADABLE
        }
    };
    std::process::ExitCode::from(u8::try_from(status).unwrap_or(2))
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

fn check(path: &str) -> i32 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return code::UNREADABLE;
        }
    };
    let register = match Register::parse(&text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{path}: {e}");
            return code::UNREADABLE;
        }
    };

    let mut failed = 0usize;
    let mut weakest = Provenance::Derived;

    println!("register {path}");
    println!(
        "  {} clock(s), {} wait(s)",
        register.sources.len(),
        register.waits.len()
    );

    for w in &register.waits {
        weakest = weakest.join(w.cost_per_iter.provenance());
        let term = w.termination();
        let fit = w.counter_fit();

        let term_note = match term {
            Termination::WellFounded => "termination held".to_string(),
            Termination::Wraps => {
                failed += 1;
                "termination FAILED: the counter is tested before it moves, so it \
                 passes zero untested and wraps"
                    .to_string()
            }
            Termination::NeverReaches => {
                failed += 1;
                "termination FAILED: the counter rises to a limit it never reaches".to_string()
            }
        };
        let fit_note = match fit {
            CounterFit::Fits { headroom } => format!("counter fit held, headroom {headroom}"),
            CounterFit::Overruns { budget, holds } => {
                failed += 1;
                format!("counter fit FAILED: budget {budget} exceeds the {holds} its counter holds")
            }
        };
        println!(
            "  wait {} [{:?}] {}; {}",
            w.id.0,
            w.cost_per_iter.provenance(),
            term_note,
            fit_note
        );
    }

    // The weakest input reaches the verdict, so a report resting on a guess
    // says so before anyone acts on it.
    println!("  standing: {weakest:?}");
    print_limits();

    if failed == 0 {
        println!("held: every declaration met its obligations, within the limits above");
        code::HELD
    } else {
        println!("failed: {failed} obligation(s) not met");
        code::FAILED
    }
}
