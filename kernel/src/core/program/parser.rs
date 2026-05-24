#![allow(non_camel_case_types)]
use super::LoaderError;
use core::mem::size_of;

pub type Elf64_Addr = u64;              // u prog addr
pub type Elf64_Off = u64;               // u file offset
pub type Elf64_Half = u16;              // u medium int
pub type Elf64_Word = u32;              // u int
pub type Elf64_Sword = i32;             // s int 
pub type Elf64_Xword = u64;             // u long int
pub type Elf64_Sxword = i64;            // s long int

pub const EI_NIDENT: usize = 16;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64_Ehdr {
    pub e_ident: [u8; EI_NIDENT],       // elf ident
    pub e_type: Elf64_Half,             // obj file type
    pub e_machine: Elf64_Half,          // machine type
    pub e_version: Elf64_Word,          // obj file version
    pub e_entry: Elf64_Addr,            // entry point addr 
    pub e_phoff: Elf64_Off,             // prog header offset
    pub e_shoff: Elf64_Off,             // section header offset
    pub e_flags: Elf64_Word,            // cpu specific flags
    pub e_ehsize: Elf64_Half,           // elf header size
    pub e_phentsize: Elf64_Half,        // size of prog header entry 
    pub e_phnum: Elf64_Half,            // number of prog header entreis
    pub e_shentsize: Elf64_Half,        // size of section header entry
    pub e_shnum: Elf64_Half,            // number of section header entries
    pub e_shstrndx: Elf64_Half,         // section name string table idx
}

pub const ELF_MAGIC_NUMBERS: [u8; 4] = [0x7F, b'E', b'L', b'F'];

#[repr(u16)]
pub enum E_Type {
    ET_NONE = 0,                        // no file type
    ET_REL = 1,                         // relocatable obj file
    ET_EXEC = 2,                        // executable file
    ET_DYN = 3,                         // shared obj file
    ET_CORE = 4,                        // core file
    ET_LOOS = 0xFE00,                   // env specific use
    ET_HIOS = 0xFEFF,                   // 
    ET_LOPROC = 0xFF00,                 // cpu specific use
    ET_HIPROC = 0xFFFF,                 // 
}

// ignore big endian because why would i ever use that i'm not running this on a washing machine
impl Elf64_Ehdr {
    pub fn from_bytes(bytes: &[u8]) -> Result<&Self, LoaderError> {
        if bytes.len() < size_of::<Self>() { return Err(LoaderError::InvalidBuffer); }
        if bytes[0..4] != ELF_MAGIC_NUMBERS { return Err(LoaderError::InvalidMagicNumbers); }

        unsafe {
            Ok(&*(bytes.as_ptr() as *const Self))
        }
    }

    pub fn get_type(&self) -> Result<E_Type, LoaderError> {
        match self.e_type {
            2 => Ok(E_Type::ET_EXEC),
            3 => Ok(E_Type::ET_DYN),
            _ => Err(LoaderError::UnsupportedElfType(self.e_type)),
        }
    }

    pub fn validate(&self) -> Result<(), LoaderError> {
        match self.e_ident[4] {
            2 => {},
            _ => return Err(LoaderError::Not64BitElf),
        }

        match self.e_ident[5] {
            1 => {},
            _ => return Err(LoaderError::NotAWashingMachine),
        }

        match self.e_ident[7] {
            0 => {},
            _ => return Err(LoaderError::UnsupportedABI(self.e_ident[7])),
        }

        // explicitly matching arm and risc-v because i might support them later
        match self.e_machine {
            0x3E => {},
            0xB7 => return Err(LoaderError::UnsupportedArch(self.e_machine)),
            0xF3 => return Err(LoaderError::UnsupportedArch(self.e_machine)),
            _ => return Err(LoaderError::UnsupportedArch(self.e_machine)),
        }

        match self.e_type {
            2 | 3 => (),
            _ => return Err(LoaderError::UnsupportedElfType(self.e_type)),
        }   

        Ok(())
    }

    pub fn prog_headers<'a>(&self, file_bytes: &'a [u8]) -> Option<ProgHeaderIter<'a>> {
        let ph_off= self.e_phoff as usize;
        let ph_num = self.e_phnum as usize;
        let ph_size = self.e_phentsize as usize;

        if ph_size != size_of::<Elf64_Phdr>() { return None };

        let tot_size = ph_num.checked_mul(ph_size)?;
        let end_off = ph_off.checked_add(tot_size)?;

        if end_off > file_bytes.len() { return None };

        let table_bytes = &file_bytes[ph_off..end_off];

        Some(ProgHeaderIter { table_bytes, ph_size, current: 0, total: ph_num })
    }
}

pub struct ProgHeaderIter<'a> {
    table_bytes: &'a [u8],
    ph_size: usize,
    current: usize,
    total: usize,
}

impl<'a> Iterator for ProgHeaderIter<'a> {
    type Item = &'a Elf64_Phdr;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current >= self.total {
            return None;
        }

        let start = self.current * self.ph_size;
        let end = start + self.ph_size;
        let ph_slice = &self.table_bytes[start..end];

        self.current += 1;

        let phdr = Elf64_Phdr::from_bytes(ph_slice).ok()?;

        Some(phdr)
    }
}

#[repr(u32)]
pub enum P_Type {
    PT_NULL     = 0,
    PT_LOAD     = 1,
    PT_DYNAMIC  = 2,
    PT_INTERP   = 3,
    PT_NOTE     = 4,
    PT_SHLIB    = 5,
    PT_PHDR     = 6,
    PT_LOOS     = 0x6000_0000,
    PT_HIOS     = 0x6FFF_FFFF,
    PT_LOPROC   = 0x7000_0000,
    PT_HIPROC   = 0x7FFF_FFFF,
}

pub const PF_X: u32 = 0x1;
pub const PF_W: u32 = 0x2;
pub const PF_R: u32 = 0x4;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf64_Phdr {
    pub p_type: Elf64_Word,                 // type of segment
    pub p_flags: Elf64_Word,                // segment flags
    pub p_offset: Elf64_Off,                // offset in file
    pub p_vaddr: Elf64_Addr,                // virt addr
    pub p_paddr: Elf64_Addr,                // reserved
    pub p_filesz: Elf64_Xword,              // size of segment in file
    pub p_memsz: Elf64_Xword,               // size of segment in memory
    pub p_align: Elf64_Xword,               // alignment of segment
}   

impl Elf64_Phdr {
    pub fn from_bytes(bytes: &[u8]) -> Result<&Self, LoaderError> {
        if bytes.len() < size_of::<Self>() {
            return Err(LoaderError::InvalidBuffer);
        }
        unsafe {
            Ok(&*(bytes.as_ptr() as *const Self))
        }
    }
}
