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
    /// Only 64-bit little-endian images are read. A big-endian or 32-bit part
    /// is a real case and simply is not handled yet.
    Unsupported,
    /// A header, table or name ran past the end of the file.
    Truncated,
}

/// A function the image declares.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    pub name: String,
    pub size: u64,
}

const MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const CLASS_64: u8 = 2;
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
pub fn functions(image: &[u8]) -> Result<Vec<Function>, NotAnImage> {
    if image.get(..4) != Some(&MAGIC[..]) {
        return Err(NotAnImage::WrongMagic);
    }
    if image.get(4) != Some(&CLASS_64) || image.get(5) != Some(&LITTLE_ENDIAN) {
        return Err(NotAnImage::Unsupported);
    }

    let shoff = u64_at(image, 0x28)? as usize;
    let shentsize = u16_at(image, 0x3a)? as usize;
    let shnum = u16_at(image, 0x3c)? as usize;

    let section = |i: usize| -> Result<(u32, usize, usize, usize, usize), NotAnImage> {
        let at = shoff
            .checked_add(i.checked_mul(shentsize).ok_or(NotAnImage::Truncated)?)
            .ok_or(NotAnImage::Truncated)?;
        Ok((
            u32_at(image, at + 4)?,             // kind
            u64_at(image, at + 0x18)? as usize, // offset
            u64_at(image, at + 0x20)? as usize, // size
            u32_at(image, at + 0x28)? as usize, // link
            u64_at(image, at + 0x38)? as usize, // entry size
        ))
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
            let info = *image.get(at + 4).ok_or(NotAnImage::Truncated)?;
            if info & 0xf != FUNC {
                continue;
            }
            let name = name_at(image, stroff, u32_at(image, at)? as usize)?;
            if name.is_empty() {
                continue;
            }
            out.push(Function {
                name,
                size: u64_at(image, at + 0x10)?,
            });
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
    fn a_thirty_two_bit_image_is_declined_by_name() {
        // Saying which support is missing beats a wrong answer or a panic.
        let mut header = vec![0x7f, b'E', b'L', b'F', 1, 1];
        header.resize(64, 0);
        assert_eq!(functions(&header).unwrap_err(), NotAnImage::Unsupported);
    }

    #[test]
    fn a_truncated_image_is_declined_rather_than_read_past() {
        // The magic and class are right and the section table is not there.
        let mut header = vec![0x7f, b'E', b'L', b'F', 2, 1];
        header.resize(20, 0);
        assert_eq!(functions(&header).unwrap_err(), NotAnImage::Truncated);
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
