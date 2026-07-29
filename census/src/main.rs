//! Draft a register from an image.
//!
//! The adapter supplies what it can read and leaves a blank wherever a human
//! has to decide. A blank is written `?`, which the core reports as a blank
//! rather than as a missing field, because one wants a decision and the other
//! wants a correction.
//!
//! What it emits is a list of candidates rather than a census of waits, and the
//! naming says so throughout. `loop-candidates` reads the back edges out of a
//! toolchain listing (call/0012); whether one of them is a wait is a judgement
//! nothing here can make. Saying so on every run matters more than the omission
//! does.

mod elf;
mod listing;

use elf::{encoding, functions, NotAnImage};
use listing::{back_edges, NotAListing};

const USAGE: &str = "\
calx-telltale-census — draft a register from an image

usage:
  calx-telltale-census loop-candidates <listing>
                                                draft a register from a disassembly listing
  calx-telltale-census draft <image> [--min-size <n>]
                                                list candidate functions from an image
  calx-telltale-census help

`loop-candidates` is the one that reaches waits. It reads a listing produced by
a toolchain disassembler, takes instruction boundaries from its address column,
and reports the backward branches inside each function. A backward branch is
evidence of a loop rather than a loop, and a loop is not yet a wait, so what
comes back is a set to curate. Produce a listing with, for example:

  <target>-objdump -d firmware.elf > firmware.lst

`draft` sees only function symbols, so what it offers are candidates rather than
waits, and it says so.

The draft goes to standard output. Every value it can read carries `extracted`
provenance and a citation; everything a human must decide is written `?`.

exit codes:
  0  a draft was written
  2  the image could not be read
";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&str> = args.iter().map(String::as_str).collect();
    let status = match positional.first().copied() {
        Some("loop-candidates") => match positional.get(1) {
            Some(path) => loop_candidates(path),
            None => {
                eprintln!("loop-candidates needs a listing to read\n\n{USAGE}");
                2
            }
        },
        // Retired rather than dropped, and the message names its replacement.
        // The old name claimed a set this command reports a lower bound of, and
        // a caller that learned the old one deserves the new one by name rather
        // than an unknown-command error to interpret.
        Some("loops") => {
            eprintln!(
                "`loops` is now `loop-candidates`, because a backward branch is \
                 evidence of a loop rather than a loop.\n\n{USAGE}"
            );
            2
        }
        Some("draft") => match positional.get(1) {
            Some(path) => {
                let min = positional
                    .iter()
                    .position(|a| *a == "--min-size")
                    .and_then(|i| positional.get(i + 1))
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1);
                draft(path, min)
            }
            None => {
                eprintln!("draft needs an image to read\n\n{USAGE}");
                2
            }
        },
        Some("help" | "-h") | None => {
            print!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("no such command: {other}\n\n{USAGE}");
            2
        }
    };
    std::process::ExitCode::from(status)
}

fn loop_candidates(path: &str) -> u8 {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return 2;
        }
    };
    let (arch, edges) = match back_edges(&text) {
        Ok(v) => v,
        Err(NotAListing::NoFormatLine) => {
            eprintln!("{path}: no file-format line, so this is not a disassembly listing");
            return 2;
        }
        Err(NotAListing::UnknownArchitecture(a)) => {
            // Named rather than guessed at. A false loop is worse than a
            // missing one, because it is a declaration a reader will act on.
            eprintln!("{path}: no branch set for {a}; this adapter cannot read its branches");
            return 2;
        }
        Err(NotAListing::Empty) => {
            eprintln!("{path}: a listing with no instructions in it");
            return 2;
        }
    };

    println!("# Drafted from {path} by calx-telltale-census.");
    println!("#");
    println!("# Architecture: {arch}, read from the listing's own header.");
    println!("#");
    println!("# Instruction boundaries come from the listing's address column, so they were");
    println!("# found by the toolchain's decoder rather than by this adapter. That is the");
    println!("# whole reason this route works on encodings nothing here could decode. It is");
    println!("# also the soundness of the result: a mis-lengthed instruction loses a back");
    println!("# edge, and nothing downstream can tell. calx-telltale states that limit on");
    println!("# every run.");
    println!("#");
    println!("# A back edge is evidence of a loop rather than a loop, and a loop is not yet a");
    println!("# wait. What budget one carries and how its counter moves are decisions for");
    println!("# someone who can read the source, so each is left blank. The set below is a");
    println!("# lower bound: a loop with no backward branch, such as a zero-overhead loop,");
    println!("# is not a back edge and is not here.");
    println!("#");
    println!("# {} loop candidate(s) in {} function(s).", edges.len(), {
        let mut names: Vec<&str> = edges.iter().map(|e| e.symbol.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names.len()
    });
    println!();

    for (i, e) in edges.iter().enumerate() {
        println!("# branch at {:#x} back to {:#x}", e.at, e.target);
        println!(
            "wait id={i} budget=? cost=? unit=? counter=? measure=? on-exhaustion=? \
             from=extracted file={path} symbol={}",
            e.symbol
        );
    }
    0
}

fn draft(path: &str, min_size: u64) -> u8 {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: {e}");
            return 2;
        }
    };
    let found = match functions(&bytes) {
        Ok(f) => f,
        Err(NotAnImage::WrongMagic) => {
            eprintln!("{path}: not an ELF image");
            return 2;
        }
        Err(NotAnImage::Unsupported) => {
            eprintln!("{path}: the class byte names neither a 32-bit nor a 64-bit image");
            return 2;
        }
        Err(NotAnImage::Truncated) => {
            eprintln!("{path}: the image is truncated or malformed");
            return 2;
        }
    };

    let candidates: Vec<&elf::Function> = found.iter().filter(|f| f.size >= min_size).collect();

    let enc = encoding(&bytes).expect("the header parsed above");
    println!("# Drafted from {path} by calx-telltale-census.");
    println!("#");
    println!(
        "# Instruction encoding: {} ({}-endian), {}.",
        enc.name(),
        if enc.little_endian { "little" } else { "big" },
        enc.boundaries().says()
    );
    println!("#");
    println!("# Every `?` is a decision this adapter could not make. A budget, a counter");
    println!("# width and a measure all have to be read from the source by someone who");
    println!("# can see it, and inventing them would be worse than leaving them blank.");
    println!("#");
    println!("# Every line below is commented out, and that is the honest state of it.");
    println!("# A wait is a polling loop. What this adapter can see is a function symbol,");
    println!("# and a function is not a wait. Emitting these as declarations would claim a");
    println!("# kind that has not been established, once per symbol, and hand back a file");
    println!("# that has to be cut down before it can be read.");
    println!("#");
    println!("# So each is a candidate to uncomment once someone has looked and found a");
    println!("# loop worth declaring. `loop-candidates` narrows this to the functions that");
    println!("# hold a backward branch, where a listing is available to read it from.");
    println!("# Anything it misses is missing silently, which is why this line is here.");
    println!("#");
    println!(
        "# {} function symbol(s) read, {} candidate(s) at or above the size threshold.",
        found.len(),
        candidates.len()
    );
    println!();

    for (i, f) in candidates.iter().enumerate() {
        println!("# {} bytes of code", f.size);
        println!(
            "# wait id={i} budget=? cost=? unit=? counter=? measure=? on-exhaustion=? \
             from=extracted file={path} symbol={}",
            f.name
        );
    }
    0
}

#[cfg(test)]
mod tests {
    use super::USAGE;

    /// Spellings kept dispatched only so a caller that learned one is told its
    /// replacement by name. They must stay out of the help, which is where a
    /// reader learns the surface.
    const RETIRED: &[&str] = &["loops"];

    /// Every verb the dispatch answers to is named in the help, or is retired.
    ///
    /// Read out of this file rather than restated, for the reason the core's
    /// check gives: a restated list needs updating by whoever forgot the help.
    #[test]
    fn every_verb_is_named_in_the_help_or_retired() {
        let verbs: Vec<&str> = include_str!("main.rs")
            .split("Some(\"")
            .skip(1)
            .map(|arm| {
                arm.split_once('"')
                    .expect("a dispatch arm opens with a quoted name")
                    .0
            })
            .collect();

        assert!(
            verbs.len() >= 4,
            "the dispatch arms were not found; this check is reading the wrong thing"
        );
        for verb in verbs {
            if RETIRED.contains(&verb) {
                assert!(
                    !USAGE.contains(verb),
                    "`{verb}` is retired and back in the help"
                );
            } else {
                assert!(
                    USAGE.contains(&format!("calx-telltale-census {verb}")),
                    "`{verb}` is dispatched and absent from the help"
                );
            }
        }
    }
}
