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

use elf::{functions, NotAnImage};

const USAGE: &str = "\
calx-telltale-census — draft a register from an image

usage:
  calx-telltale-census draft <image> [--min-size <n>]
  calx-telltale-census help

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

    println!("# Drafted from {path} by calx-telltale-census.");
    println!("#");
    println!("# Every `?` is a decision this adapter could not make. A budget, a counter");
    println!("# width and a measure all have to be read from the source by someone who");
    println!("# can see it, and inventing them would be worse than leaving them blank.");
    println!("#");
    println!("# These are candidates rather than waits. Finding the polling loops inside");
    println!("# a function needs a disassembler, which this adapter does not have, so the");
    println!("# set below is a starting point and not a census. Anything it misses is");
    println!("# missing silently, which is why this line is here.");
    println!("#");
    println!(
        "# {} function symbol(s) read, {} at or above the size threshold.",
        found.len(),
        candidates.len()
    );
    println!();

    for (i, f) in candidates.iter().enumerate() {
        println!("# {} bytes of code", f.size);
        println!(
            "wait id={i} budget=? cost=? unit=? counter=? measure=? on-exhaustion=? \
             from=extracted file={path} symbol={}",
            f.name
        );
    }
    0
}
