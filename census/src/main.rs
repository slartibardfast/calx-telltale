//! Draft a register from an image.
//!
//! The adapter supplies what it can read and leaves a blank wherever a human
//! has to decide. A blank is written `?`, which the core reports as a blank
//! rather than as a missing field, because one wants a decision and the other
//! wants a correction.
//!
//! What it emits is a list of candidates rather than a census of waits. Finding
//! the polling loops inside a function needs a disassembler, which call/0011
//! deliberately keeps out of this project for now. Saying so on every run
//! matters more than the omission does.

mod elf;
mod listing;

use elf::{encoding, functions, NotAnImage};
use listing::{back_edges, NotAListing};

const USAGE: &str = "\
calx-telltale-census — draft a register from an image

usage:
  calx-telltale-census loops <listing>          draft a register from a disassembly listing
  calx-telltale-census draft <image> [--min-size <n>]
                                                list candidate functions from an image
  calx-telltale-census help

`loops` is the one that finds waits. It reads a listing produced by a toolchain
disassembler, takes instruction boundaries from its address column, and reports
the backward branches inside each function. Produce one with, for example:

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
        Some("loops") => match positional.get(1) {
            Some(path) => loops(path),
            None => {
                eprintln!("loops needs a listing to read\n\n{USAGE}");
                2
            }
        },
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

fn loops(path: &str) -> u8 {
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
            eprintln!("{path}: no branch set for {a}; this adapter cannot read its loops");
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
    println!("# A back edge is a loop. Whether a loop is a wait, what budget it carries and");
    println!("# how its counter moves are decisions for someone who can read the source, so");
    println!("# each is left blank. The freeze set below is a lower bound: a loop with no");
    println!("# backward branch, such as a zero-overhead loop, is not a back edge.");
    println!("#");
    println!("# {} loop(s) in {} function(s).", edges.len(), {
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
            eprintln!("{path}: only 64-bit little-endian images are read");
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
    println!("# loop worth declaring. Finding those loops is what would make this a census,");
    println!("# and it needs instruction boundaries this adapter does not yet compute.");
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
