//! Just enough ELF to find the functions in an image.
//!
//! Written here rather than taken as a dependency because
//! [call/0011] draws the line at exactly this point: parsing an object file is
//! bounded work against a published layout, while decoding instructions is
//! architecture-specific and unbounded. This half is the bounded half.
//!
//! Every field is read with a bounds check and every offset is validated before
//! use, because the input is a binary somebody else built and a census that
//! panicked on a malformed image would be worse than one that declined.

/// Why an image could not be read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotAnImage {
    /// The magic number says this is something else.
    WrongMagic,
    /// The class byte says neither 32-bit nor 64-bit.
    Unsupported,
    /// A header, table or name ran past the end of the file.
    Truncated,
}

/// The instruction encoding an image holds, so far as its header declares it.
///
/// This is the unit a decoder is written against, and it is finer than the
/// instruction set. One set can carry several encodings, and they are separate
/// decoders: fixed-width instructions can be strided over, while a mixed-width
/// encoding has to be decoded far enough to learn each instruction's length
/// before the next one can be found at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Encoding {
    /// The instruction set, as the image's own header names it.
    pub machine: u16,
    /// Whether instruction words are little-endian.
    pub little_endian: bool,
}

impl Encoding {
    /// The instruction set's usual name, or its number where this adapter has
    /// none. Naming the number beats reporting nothing: an operator can look it
    /// up, and a wrong guess would be worse than an honest gap.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self.machine {
            3 => "x86",
            40 => "ARM (A32 or T32)",
            45 => "ARC",
            62 => "x86-64",
            93 => "ARCompact",
            183 => "AArch64 (A64)",
            195 => "ARCv2",
            243 => "RISC-V",
            _ => "unrecognised",
        }
    }

    /// How much work it takes to find where one instruction ends and the next
    /// begins.
    ///
    /// This is the cost that decides whether writing a decoder is affordable,
    /// and it has three tiers rather than two.
    #[must_use]
    pub const fn boundaries(self) -> Boundaries {
        match self.machine {
            183 => Boundaries::Strided,
            // Thumb-2 and the compressed RISC-V forms both read their length
            // from a fixed prefix of the first halfword, so a reader advances
            // without understanding the instruction.
            40 | 243 => Boundaries::PrefixLength,
            // ARC carries two independent width-changing forms: the compact
            // encodings, and a long-immediate word appended to an instruction.
            // Whether that word follows is a property of the operand form, so
            // no fixed prefix gives the length.
            45 | 93 | 195 => Boundaries::OperandLength,
            3 | 62 => Boundaries::OperandLength,
            _ => Boundaries::Unknown,
        }
    }
}

/// What it takes to find instruction boundaries in an encoding.
///
/// Boundaries come before branches: a back edge cannot be found without knowing
/// where instructions start, and on a mixed-width encoding that is the larger
/// half of the work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boundaries {
    /// Every instruction is the same size, so a reader strides over the extent.
    Strided,
    /// Sizes vary and a fixed prefix of the first unit gives the length, so a
    /// reader advances without understanding the instruction.
    PrefixLength,
    /// Sizes vary and the length depends on the operand form, so a reader has
    /// to decode most of the instruction before it can find the next one. This
    /// is the tier where writing a decoder stops being cheap.
    OperandLength,
    /// This adapter has no entry for the encoding.
    Unknown,
}

impl Boundaries {
    /// How a report describes the tier.
    #[must_use]
    pub const fn says(self) -> &'static str {
        match self {
            Boundaries::Strided => "fixed-width, so boundaries are strided",
            Boundaries::PrefixLength => {
                "mixed-width, with the length in a fixed prefix of each instruction"
            }
            Boundaries::OperandLength => {
                "mixed-width, with the length depending on the operand form"
            }
            Boundaries::Unknown => "boundaries unknown to this adapter",
        }
    }
}

/// A function the image declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    pub name: String,
    pub size: u64,
}

const MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const CLASS_64: u8 = 2;
const CLASS_32: u8 = 1;
const LITTLE_ENDIAN: u8 = 1;
const SYMTAB: u32 = 2;
const STRTAB: u32 = 3;
const FUNC: u8 = 2;

fn u16_at(b: &[u8], at: usize) -> Result<u16, NotAnImage> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(NotAnImage::Truncated)
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, NotAnImage> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(NotAnImage::Truncated)
}

fn u64_at(b: &[u8], at: usize) -> Result<u64, NotAnImage> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or(NotAnImage::Truncated)
}

fn name_at(b: &[u8], table: usize, offset: usize) -> Result<String, NotAnImage> {
    let start = table.checked_add(offset).ok_or(NotAnImage::Truncated)?;
    let rest = b.get(start..).ok_or(NotAnImage::Truncated)?;
    let end = rest.iter().position(|c| *c == 0).unwrap_or(rest.len());
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// Every function symbol the image declares, in the order the table holds them.
///
/// This is a list of candidates rather than a census of waits. Finding the
/// polling loops inside these functions needs a disassembler, which is the part
/// [call/0011] keeps out.
/// The encoding this image declares in its header.
pub fn encoding(image: &[u8]) -> Result<Encoding, NotAnImage> {
    if image.get(..4) != Some(&MAGIC[..]) {
        return Err(NotAnImage::WrongMagic);
    }
    let little_endian = image.get(5) == Some(&LITTLE_ENDIAN);
    Ok(Encoding {
        machine: u16_at(image, 0x12)?,
        little_endian,
    })
}

pub fn functions(image: &[u8]) -> Result<Vec<Function>, NotAnImage> {
    if image.get(..4) != Some(&MAGIC[..]) {
        return Err(NotAnImage::WrongMagic);
    }
    // Thirty-two bit images are the common case on the parts this tool is
    // aimed at, so both classes are read. The layouts differ in more than field
    // width: a symbol's info byte moves, which is the kind of difference that
    // reads plausible values from the wrong offsets rather than failing.
    let wide = match image.get(4) {
        Some(&CLASS_64) => true,
        Some(&CLASS_32) => false,
        _ => return Err(NotAnImage::Unsupported),
    };

    let (shoff, shentsize_at, shnum_at) = if wide {
        (u64_at(image, 0x28)? as usize, 0x3a, 0x3c)
    } else {
        (u32_at(image, 0x20)? as usize, 0x2e, 0x30)
    };
    let shentsize = u16_at(image, shentsize_at)? as usize;
    let shnum = u16_at(image, shnum_at)? as usize;

    let section = |i: usize| -> Result<(u32, usize, usize, usize, usize), NotAnImage> {
        let at = shoff
            .checked_add(i.checked_mul(shentsize).ok_or(NotAnImage::Truncated)?)
            .ok_or(NotAnImage::Truncated)?;
        if wide {
            Ok((
                u32_at(image, at + 4)?,
                u64_at(image, at + 0x18)? as usize,
                u64_at(image, at + 0x20)? as usize,
                u32_at(image, at + 0x28)? as usize,
                u64_at(image, at + 0x38)? as usize,
            ))
        } else {
            Ok((
                u32_at(image, at + 4)?,
                u32_at(image, at + 0x10)? as usize,
                u32_at(image, at + 0x14)? as usize,
                u32_at(image, at + 0x18)? as usize,
                u32_at(image, at + 0x24)? as usize,
            ))
        }
    };

    let mut out = Vec::new();
    for i in 0..shnum {
        let (kind, offset, size, link, entsize) = section(i)?;
        if kind != SYMTAB || entsize == 0 {
            continue;
        }
        let (strkind, stroff, _, _, _) = section(link)?;
        if strkind != STRTAB {
            return Err(NotAnImage::Truncated);
        }
        for s in 0..(size / entsize) {
            let at = offset
                .checked_add(s.checked_mul(entsize).ok_or(NotAnImage::Truncated)?)
                .ok_or(NotAnImage::Truncated)?;
            // The info byte sits at a different offset in each class, and the
            // size with it.
            let (info_at, size_at) = if wide { (4, 0x10) } else { (12, 8) };
            let info = *image.get(at + info_at).ok_or(NotAnImage::Truncated)?;
            if info & 0xf != FUNC {
                continue;
            }
            let name = name_at(image, stroff, u32_at(image, at)? as usize)?;
            if name.is_empty() {
                continue;
            }
            let size = if wide {
                u64_at(image, at + size_at)?
            } else {
                u32_at(image, at + size_at)? as u64
            };
            out.push(Function { name, size });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{functions, NotAnImage};

    #[test]
    fn something_that_is_not_an_image_is_declined_rather_than_guessed_at() {
        assert_eq!(
            functions(b"not an elf").unwrap_err(),
            NotAnImage::WrongMagic
        );
        assert_eq!(functions(&[]).unwrap_err(), NotAnImage::WrongMagic);
    }

    #[test]
    fn a_class_byte_naming_neither_width_is_declined() {
        let mut header = vec![0x7f, b'E', b'L', b'F', 7, 1];
        header.resize(64, 0);
        assert_eq!(functions(&header).unwrap_err(), NotAnImage::Unsupported);
    }

    #[test]
    fn a_thirty_two_bit_image_is_read_rather_than_declined() {
        // The parts this tool is aimed at are largely 32-bit, so declining
        // them would have declined the domain. A header with no sections
        // yields nothing and reads cleanly.
        let mut header = vec![0x7f, b'E', b'L', b'F', 1, 1];
        header.resize(64, 0);
        assert_eq!(functions(&header), Ok(Vec::new()));
    }

    #[test]
    fn a_truncated_image_is_declined_rather_than_read_past() {
        // The magic and class are right and the section table is not there.
        let mut header = vec![0x7f, b'E', b'L', b'F', 2, 1];
        header.resize(20, 0);
        assert_eq!(functions(&header).unwrap_err(), NotAnImage::Truncated);
    }

    #[test]
    fn an_image_names_the_encoding_it_holds() {
        // The image answers part of the question itself, which is what lets the
        // adapter decline by name rather than guess at a decoder.
        let me = std::env::current_exe().expect("a test binary exists");
        let bytes = std::fs::read(me).expect("and can be read");
        let e = super::encoding(&bytes).expect("and declares an encoding");
        assert!(e.little_endian);
        assert_eq!(e.name(), "x86-64");
        assert_eq!(e.boundaries(), super::Boundaries::OperandLength);
    }

    #[test]
    fn the_boundary_tiers_are_told_apart() {
        let mk = |machine| super::Encoding {
            machine,
            little_endian: true,
        };
        // A64 strides. Thumb and compressed RISC-V read a length prefix. ARC
        // needs the operand form, which is the tier where a decoder stops
        // being cheap.
        assert_eq!(mk(183).boundaries(), super::Boundaries::Strided);
        assert_eq!(mk(40).boundaries(), super::Boundaries::PrefixLength);
        assert_eq!(mk(243).boundaries(), super::Boundaries::PrefixLength);
        assert_eq!(mk(195).boundaries(), super::Boundaries::OperandLength);
    }

    #[test]
    fn an_encoding_this_adapter_cannot_name_is_still_reported() {
        let unknown = super::Encoding {
            machine: 0xbeef,
            little_endian: true,
        };
        assert_eq!(unknown.name(), "unrecognised");
        assert_eq!(unknown.boundaries(), super::Boundaries::Unknown);
    }

    #[test]
    fn a_real_image_yields_its_functions() {
        // The test binary is itself an ELF, so it is the honest fixture.
        let me = std::env::current_exe().expect("a test binary exists");
        let bytes = std::fs::read(me).expect("and can be read");
        let fns = functions(&bytes).expect("and parses");
        assert!(!fns.is_empty(), "a Rust test binary declares functions");
        assert!(fns.iter().all(|f| !f.name.is_empty()));
    }
}
