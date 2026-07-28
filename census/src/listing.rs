//! Reading loops out of a disassembly listing.
//!
//! The listing has already solved the hard problem. Its address column is the
//! instruction boundaries, found by a decoder that knows the encoding, which is
//! why [call/0012] delegates rather than decodes.
//!
//! What is left is small: group the instructions by the function they sit in,
//! recognise the branches, and take the ones whose target sits at or before
//! them. That last part is what makes a loop rather than a jump onward.

/// Why a listing could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NotAListing {
    /// No `file format` line, so this is not a disassembly listing.
    NoFormatLine,
    /// The listing names an architecture whose branches this adapter does not
    /// know. Declined by name rather than guessed at: a false loop is worse
    /// than a missing one, because it is a declaration a reader will act on.
    UnknownArchitecture(String),
    /// A listing with a header and no instructions in it.
    Empty,
}

/// A loop found in the listing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackEdge {
    /// The function containing it.
    pub symbol: String,
    /// Where the branch sits.
    pub at: u64,
    /// Where it goes, which is at or before `at`.
    pub target: u64,
}

/// The branch mnemonics of one architecture.
struct Arch {
    /// How the listing's own format line names it.
    format: &'static str,
    /// A human-facing name.
    name: &'static str,
    /// Mnemonic prefixes that transfer control. Prefixes rather than whole
    /// mnemonics because condition codes and width suffixes attach to them.
    branches: &'static [&'static str],
}

/// The architectures this adapter knows the branches of.
///
/// Deliberately a list rather than a heuristic. Growing it is a small, checkable
/// change; guessing is not.
const ARCHES: &[Arch] = &[
    Arch {
        format: "littlearc",
        name: "ARC",
        // ARC also has a zero-overhead loop instruction, which is a loop with
        // no backward branch at all. It is listed here so that it is found, and
        // it is a reminder that a back edge is not the only shape a loop takes.
        branches: &["b", "j", "dbnz", "lp"],
    },
    Arch {
        format: "littlearm",
        name: "ARM",
        branches: &["b", "cbz", "cbnz"],
    },
    Arch {
        format: "littleaarch64",
        name: "AArch64",
        branches: &["b", "cbz", "cbnz", "tbz", "tbnz"],
    },
    Arch {
        format: "littleriscv",
        name: "RISC-V",
        branches: &["b", "j", "c.b", "c.j"],
    },
    Arch {
        format: "x86-64",
        name: "x86-64",
        branches: &["j", "loop"],
    },
];

fn arch_for(format_line: &str) -> Option<&'static Arch> {
    ARCHES.iter().find(|a| format_line.contains(a.format))
}

/// Whether a mnemonic transfers control on this architecture.
fn is_branch(arch: &Arch, mnemonic: &str) -> bool {
    arch.branches.iter().any(|b| mnemonic.starts_with(b))
}

/// The address a listing line sits at, where it has one.
///
/// An instruction line begins with an indented hexadecimal address and a colon.
fn address_of(line: &str) -> Option<u64> {
    let t = line.trim_start();
    if t.len() == line.len() {
        return None; // a function header is not indented
    }
    let (addr, _) = t.split_once(':')?;
    u64::from_str_radix(addr.trim(), 16).ok()
}

/// The symbol a function header names, with the address it starts at.
fn header_of(line: &str) -> Option<(u64, String)> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let (addr, rest) = line.split_once(' ')?;
    let start = u64::from_str_radix(addr.trim(), 16).ok()?;
    let name = rest.trim().strip_prefix('<')?.strip_suffix(">:")?;
    Some((start, name.to_string()))
}

/// The mnemonic and the branch target of an instruction line.
///
/// A listing puts the encoded bytes between the address and the mnemonic, and
/// resolves a branch target to an address followed by a symbol in angle
/// brackets. That resolved target is what makes the target column trustworthy:
/// the decoder computed it, not this adapter.
fn instruction_of(line: &str) -> Option<(&str, Option<u64>)> {
    let (_, rest) = line.trim_start().split_once(':')?;
    // The address, the encoded bytes and the instruction are tab-separated, and
    // the instruction is last. Counting from the front would land on the bytes,
    // because the text after the colon opens with the separator.
    let text = rest.rsplit('\t').next()?.trim();
    let mut words = text.split_whitespace();
    let mnemonic = words.next()?;
    let target = words.find_map(|w| u64::from_str_radix(w.trim_start_matches("0x"), 16).ok());
    Some((mnemonic, target))
}

/// Every loop the listing shows, in the order they appear.
pub fn back_edges(listing: &str) -> Result<(&'static str, Vec<BackEdge>), NotAListing> {
    let format_line = listing
        .lines()
        .find(|l| l.contains("file format"))
        .ok_or(NotAListing::NoFormatLine)?;
    let arch = arch_for(format_line).ok_or_else(|| {
        NotAListing::UnknownArchitecture(
            format_line
                .split("file format")
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string(),
        )
    })?;

    let mut out = Vec::new();
    let mut symbol: Option<String> = None;
    let mut start: u64 = 0;
    let mut saw_instruction = false;

    for line in listing.lines() {
        if let Some((addr, name)) = header_of(line) {
            symbol = Some(name);
            start = addr;
            continue;
        }
        let Some(at) = address_of(line) else { continue };
        saw_instruction = true;
        let Some((mnemonic, target)) = instruction_of(line) else {
            continue;
        };
        if !is_branch(arch, mnemonic) {
            continue;
        }
        let Some(target) = target else { continue };
        // Backward, and inside the function it was found in. A branch out of
        // the function is a tail call or a jump onward, not a loop.
        if target <= at && target >= start {
            if let Some(sym) = &symbol {
                out.push(BackEdge {
                    symbol: sym.clone(),
                    at,
                    target,
                });
            }
        }
    }

    if !saw_instruction {
        return Err(NotAListing::Empty);
    }
    Ok((arch.name, out))
}

#[cfg(test)]
mod tests {
    use super::{back_edges, NotAListing};

    const ARC_LISTING: &str = "
firmware.elf:     file format elf32-littlearc

Disassembly of section .text:

00001000 <bus_wait_idle>:
    1000:\t7f 20 00 00 \tld r0,[r1]
    1004:\t20 80 00 00 \ttst r0,1
    1008:\tf9 07       \tbne 1000 <bus_wait_idle>
    100a:\te0 7e       \tj_s [blink]

00001010 <chan_teardown>:
    1010:\t20 80 00 00 \tsub r2,r2,1
    1014:\tf9 07       \tbne 1010 <chan_teardown>
    1016:\te0 7e       \tj_s [blink]
";

    #[test]
    fn a_listing_yields_the_loops_it_shows() {
        let (arch, edges) = back_edges(ARC_LISTING).unwrap();
        assert_eq!(arch, "ARC");
        assert_eq!(edges.len(), 2);
        assert_eq!(edges[0].symbol, "bus_wait_idle");
        assert_eq!((edges[0].at, edges[0].target), (0x1008, 0x1000));
        assert_eq!(edges[1].symbol, "chan_teardown");
    }

    #[test]
    fn a_forward_branch_is_not_a_loop() {
        let forward =
            ARC_LISTING.replace("bne 1000 <bus_wait_idle>", "bne 100a <bus_wait_idle+0xa>");
        let (_, edges) = back_edges(&forward).unwrap();
        assert_eq!(edges.len(), 1, "only the teardown loop is left");
    }

    #[test]
    fn a_branch_out_of_the_function_is_not_a_loop() {
        // Backward, but into an earlier function: a tail call rather than a
        // loop, and counting it would invent a loop that is not there.
        let out = ARC_LISTING.replace("bne 1010 <chan_teardown>", "bne 1000 <bus_wait_idle>");
        let (_, edges) = back_edges(&out).unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].symbol, "bus_wait_idle");
    }

    #[test]
    fn an_architecture_with_no_branch_set_is_declined_by_name() {
        let odd = ARC_LISTING.replace("elf32-littlearc", "elf32-somethingelse");
        assert_eq!(
            back_edges(&odd).unwrap_err(),
            NotAListing::UnknownArchitecture("elf32-somethingelse".to_string())
        );
    }

    #[test]
    fn something_that_is_not_a_listing_is_declined() {
        assert_eq!(back_edges("hello").unwrap_err(), NotAListing::NoFormatLine);
        let header_only = "a.elf:     file format elf32-littlearc\n";
        assert_eq!(back_edges(header_only).unwrap_err(), NotAListing::Empty);
    }

    #[test]
    fn a_non_branch_referencing_an_address_is_not_a_loop() {
        // A load whose operand happens to name an earlier address would be a
        // loop under a shape heuristic. The mnemonic set is what stops it.
        let load = ARC_LISTING.replace("bne 1000 <bus_wait_idle>", "ld r3,[1000]");
        let (_, edges) = back_edges(&load).unwrap();
        assert_eq!(edges.len(), 1, "the load is not counted");
    }
}
