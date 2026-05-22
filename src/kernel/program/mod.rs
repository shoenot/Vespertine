pub mod parser;
use core::fmt;
use parser::*;

use crate::klogln;

pub enum LoaderError {
    InvalidBuffer,
    InvalidMagicNumbers,
    NotAWashingMachine,
    Not64BitElf,
    UnsupportedElfType(u16),
    UnsupportedArch(u16),
    UnsupportedABI(u8),
}

impl fmt::Display for LoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoaderError::InvalidBuffer => write!(f, "InvalidBuffer"),
            LoaderError::InvalidMagicNumbers => write!(f, "Invalid ELF Magic numbers"),
            LoaderError::NotAWashingMachine => write!(f, "Big endian not supported"),
            LoaderError::Not64BitElf => write!(f, "32 bit programs not supported"),
            LoaderError::UnsupportedElfType(t) => write!(f, "Unsupported ELF type: 0x{:X}", t),
            LoaderError::UnsupportedArch(t) => write!(f, "Unsupported architechture: 0x{:X}", t),
            LoaderError::UnsupportedABI(t) => write!(f, "Unsupported ABI: 0x{:X}", t),
        }
    }
}

pub fn load_elf(file_bytes: &[u8]) -> Result<(), LoaderError> {
    let header = Elf64_Ehdr::from_bytes(file_bytes)?;
    let ph_iter = header.prog_headers(file_bytes).unwrap();
    
    for ph in ph_iter {
        if ph.p_type == P_Type::PT_LOAD as u32 {
            klogln!(
                "Mapping Segment: file offset 0x{:X} -> virt addr 0x{:X} file size: {}, mem_size: {}",
                ph.p_offset, ph.p_vaddr, ph.p_filesz, ph.p_memsz
            );
        }

        // TODO: Map VMO and memcpy program contents to there
    }

    klogln!("Ready to jump to entry 0x{:X}",header.e_entry);
    Ok(())
}
