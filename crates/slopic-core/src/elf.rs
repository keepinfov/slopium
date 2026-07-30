//! Writing a relocatable ELF64 object.
//!
//! This is the last step that used to belong to `as`. It takes a laid-out
//! [`Object`] — bytes, symbols, and the references the linker still has to
//! fill in — and produces the file a linker accepts. It knows nothing about
//! either instruction set beyond two numbers per architecture: the machine id
//! in the header, and which relocation type spells each [`FixupKind`].
//!
//! Executables are still the system linker's to produce. It is what knows
//! where the C runtime lives, which dynamic loader to name, and how this
//! platform starts a process; none of that is a code generation question, and
//! `D-001` keeps that dependency deliberately.

use crate::asm::{Against, Definition, FixupKind, Object, Section};

const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_RELA: u32 = 4;

const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;
const SHF_INFO_LINK: u64 = 0x40;

const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;
const STT_NOTYPE: u8 = 0;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;

const HEADER_SIZE: u64 = 64;
const SECTION_HEADER_SIZE: u64 = 64;
const SYMBOL_SIZE: u64 = 24;
const RELOCATION_SIZE: u64 = 24;

/// The architecture-dependent half of an object file.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    id: u16,
    /// The relocation types this architecture spells the shared fixup kinds
    /// with. A kind the architecture has no relocation for cannot appear in
    /// its objects, and saying so here is how that is enforced.
    relocation: fn(FixupKind) -> Option<u32>,
}

pub const X86_64: Machine = Machine {
    id: EM_X86_64,
    relocation: |kind| match kind {
        // Values from the System V AMD64 psABI.
        FixupKind::Pc32 => Some(2),
        FixupKind::Plt32 => Some(4),
        _ => None,
    },
};

pub const AARCH64: Machine = Machine {
    id: EM_AARCH64,
    relocation: |kind| match kind {
        // Values from the AArch64 ELF ABI.
        FixupKind::AdrPage21 => Some(275),
        FixupKind::AddLo12 => Some(277),
        FixupKind::CondBr19 => Some(280),
        FixupKind::Jump26 => Some(282),
        FixupKind::Call26 => Some(283),
        _ => None,
    },
};

/// The machine for a target triple, or `None` when no backend claims it.
pub fn machine_for(triple: &str) -> Option<Machine> {
    match triple {
        "x86_64-unknown-linux-gnu" => Some(X86_64),
        "aarch64-unknown-linux-gnu" => Some(AARCH64),
        _ => None,
    }
}

/// A section as it will appear in the file.
struct Part {
    name: &'static str,
    kind: u32,
    flags: u64,
    align: u64,
    link: u32,
    info: u32,
    entry_size: u64,
    body: Vec<u8>,
}

/// Serializes `object` as a relocatable ELF64 file for `machine`.
pub fn write(object: &Object, machine: Machine) -> Result<Vec<u8>, String> {
    let mut strings = StringTable::new();
    let mut section_names = StringTable::new();

    // Section indices are fixed here and referred to below, because a
    // relocation section names the one it applies to by index and the symbol
    // table names the section every defined symbol sits in.
    let text_index = 1u16;
    let rodata_index = 2u16;
    let note_index = 3u16;
    let symtab_index = 4u16;
    let strtab_index = 5u16;

    let index_of = |section: Section| match section {
        Section::Text => text_index,
        Section::RoData => rodata_index,
        Section::GnuStack => note_index,
    };

    // The symbol table opens with the null entry and the section symbols,
    // which are what a local label is reached through, and which have to come
    // before every global because `sh_info` splits the table in exactly one
    // place.
    let mut symbols = vec![SymbolEntry::null()];
    let text_symbol = symbols.len() as u32;
    symbols.push(SymbolEntry::section(text_index));
    let rodata_symbol = symbols.len() as u32;
    symbols.push(SymbolEntry::section(rodata_index));
    let first_global = symbols.len() as u32;

    let mut symbol_index = Vec::with_capacity(object.symbols.len());
    for symbol in &object.symbols {
        symbol_index.push(symbols.len() as u32);
        let kind = if object.functions.contains(&symbol.name) {
            STT_FUNC
        } else {
            STT_NOTYPE
        };
        symbols.push(match symbol.definition {
            Some(Definition {
                section,
                offset,
                size,
            }) => SymbolEntry {
                name: strings.add(&symbol.name),
                info: (STB_GLOBAL << 4) | kind,
                other: 0,
                section: index_of(section),
                value: offset,
                size,
            },
            None => SymbolEntry {
                name: strings.add(&symbol.name),
                info: (STB_GLOBAL << 4) | STT_NOTYPE,
                other: 0,
                section: 0,
                value: 0,
                size: 0,
            },
        });
    }

    let mut relocations = Vec::with_capacity(object.relocations.len() * RELOCATION_SIZE as usize);
    for relocation in &object.relocations {
        let kind = (machine.relocation)(relocation.kind).ok_or_else(|| {
            format!(
                "{:?} has no relocation on this architecture",
                relocation.kind
            )
        })?;
        let symbol = match relocation.against {
            Against::Symbol(index) => *symbol_index
                .get(index)
                .ok_or_else(|| format!("relocation names symbol {index}, which does not exist"))?,
            Against::Section(Section::Text) => text_symbol,
            Against::Section(Section::RoData) => rodata_symbol,
            Against::Section(Section::GnuStack) => {
                return Err("nothing can refer to .note.GNU-stack".into())
            }
        };
        relocations.extend_from_slice(&relocation.offset.to_le_bytes());
        relocations.extend_from_slice(&(((symbol as u64) << 32) | kind as u64).to_le_bytes());
        relocations.extend_from_slice(&relocation.addend.to_le_bytes());
    }

    let mut symbol_bytes = Vec::with_capacity(symbols.len() * SYMBOL_SIZE as usize);
    for symbol in &symbols {
        symbol.write(&mut symbol_bytes);
    }

    let parts = vec![
        Part {
            name: Section::Text.name(),
            kind: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_EXECINSTR,
            align: 16,
            link: 0,
            info: 0,
            entry_size: 0,
            body: object.text.clone(),
        },
        Part {
            name: Section::RoData.name(),
            kind: SHT_PROGBITS,
            flags: SHF_ALLOC,
            align: 8,
            link: 0,
            info: 0,
            entry_size: 0,
            body: object.rodata.clone(),
        },
        // Present and empty is the whole point: its absence is what makes a
        // linker mark the stack executable.
        Part {
            name: Section::GnuStack.name(),
            kind: SHT_PROGBITS,
            flags: 0,
            align: 1,
            link: 0,
            info: 0,
            entry_size: 0,
            body: Vec::new(),
        },
        Part {
            name: ".symtab",
            kind: SHT_SYMTAB,
            flags: 0,
            align: 8,
            link: strtab_index as u32,
            info: first_global,
            entry_size: SYMBOL_SIZE,
            body: symbol_bytes,
        },
        Part {
            name: ".strtab",
            kind: SHT_STRTAB,
            flags: 0,
            align: 1,
            link: 0,
            info: 0,
            entry_size: 0,
            body: strings.finish(),
        },
        Part {
            name: ".rela.text",
            kind: SHT_RELA,
            flags: SHF_INFO_LINK,
            align: 8,
            link: symtab_index as u32,
            info: text_index as u32,
            entry_size: RELOCATION_SIZE,
            body: relocations,
        },
    ];
    let shstrtab_index = parts.len() as u16 + 1;

    let mut file = vec![0u8; HEADER_SIZE as usize];
    let mut headers = vec![SectionHeader::null()];
    for part in &parts {
        pad_to(&mut file, part.align.max(1));
        let offset = file.len() as u64;
        file.extend_from_slice(&part.body);
        headers.push(SectionHeader {
            name: section_names.add(part.name),
            kind: part.kind,
            flags: part.flags,
            offset,
            size: part.body.len() as u64,
            link: part.link,
            info: part.info,
            align: part.align,
            entry_size: part.entry_size,
        });
    }

    let shstrtab_name = section_names.add(".shstrtab");
    let shstrtab = section_names.finish();
    pad_to(&mut file, 1);
    let shstrtab_offset = file.len() as u64;
    file.extend_from_slice(&shstrtab);
    headers.push(SectionHeader {
        name: shstrtab_name,
        kind: SHT_STRTAB,
        flags: 0,
        offset: shstrtab_offset,
        size: shstrtab.len() as u64,
        link: 0,
        info: 0,
        align: 1,
        entry_size: 0,
    });

    pad_to(&mut file, 8);
    let section_header_offset = file.len() as u64;
    for header in &headers {
        header.write(&mut file);
    }

    write_file_header(
        &mut file,
        machine.id,
        section_header_offset,
        headers.len() as u16,
        shstrtab_index,
    );
    Ok(file)
}

fn write_file_header(
    file: &mut [u8],
    machine: u16,
    section_header_offset: u64,
    section_count: u16,
    shstrtab_index: u16,
) {
    let header = &mut file[..HEADER_SIZE as usize];
    header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2; // ELFCLASS64
    header[5] = 1; // ELFDATA2LSB
    header[6] = 1; // EV_CURRENT
    header[16..18].copy_from_slice(&1u16.to_le_bytes()); // ET_REL
    header[18..20].copy_from_slice(&machine.to_le_bytes());
    header[20..24].copy_from_slice(&1u32.to_le_bytes()); // EV_CURRENT
    header[40..48].copy_from_slice(&section_header_offset.to_le_bytes());
    header[52..54].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    header[58..60].copy_from_slice(&(SECTION_HEADER_SIZE as u16).to_le_bytes());
    header[60..62].copy_from_slice(&section_count.to_le_bytes());
    header[62..64].copy_from_slice(&shstrtab_index.to_le_bytes());
}

fn pad_to(file: &mut Vec<u8>, alignment: u64) {
    let alignment = alignment.max(1) as usize;
    while !file.len().is_multiple_of(alignment) {
        file.push(0);
    }
}

struct SectionHeader {
    name: u32,
    kind: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entry_size: u64,
}

impl SectionHeader {
    fn null() -> Self {
        Self {
            name: 0,
            kind: 0,
            flags: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            align: 0,
            entry_size: 0,
        }
    }

    fn write(&self, file: &mut Vec<u8>) {
        file.extend_from_slice(&self.name.to_le_bytes());
        file.extend_from_slice(&self.kind.to_le_bytes());
        file.extend_from_slice(&self.flags.to_le_bytes());
        file.extend_from_slice(&0u64.to_le_bytes()); // sh_addr
        file.extend_from_slice(&self.offset.to_le_bytes());
        file.extend_from_slice(&self.size.to_le_bytes());
        file.extend_from_slice(&self.link.to_le_bytes());
        file.extend_from_slice(&self.info.to_le_bytes());
        file.extend_from_slice(&self.align.to_le_bytes());
        file.extend_from_slice(&self.entry_size.to_le_bytes());
    }
}

struct SymbolEntry {
    name: u32,
    info: u8,
    other: u8,
    section: u16,
    value: u64,
    size: u64,
}

impl SymbolEntry {
    fn null() -> Self {
        Self {
            name: 0,
            info: 0,
            other: 0,
            section: 0,
            value: 0,
            size: 0,
        }
    }

    fn section(index: u16) -> Self {
        Self {
            name: 0,
            info: (STB_LOCAL << 4) | STT_SECTION,
            other: 0,
            section: index,
            value: 0,
            size: 0,
        }
    }

    fn write(&self, file: &mut Vec<u8>) {
        file.extend_from_slice(&self.name.to_le_bytes());
        file.push(self.info);
        file.push(self.other);
        file.extend_from_slice(&self.section.to_le_bytes());
        file.extend_from_slice(&self.value.to_le_bytes());
        file.extend_from_slice(&self.size.to_le_bytes());
    }
}

/// An ELF string table: a leading NUL, then every name, each NUL-terminated.
struct StringTable {
    bytes: Vec<u8>,
}

impl StringTable {
    fn new() -> Self {
        Self { bytes: vec![0] }
    }

    fn add(&mut self, name: &str) -> u32 {
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.push(0);
        offset
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{Assembly, Code, Instruction, Item, Section};
    use std::fmt;

    #[derive(Clone, Debug, PartialEq)]
    struct Nop;

    impl fmt::Display for Nop {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("nop")
        }
    }

    impl Instruction for Nop {
        fn encode(&self, code: &mut Code) -> Result<(), String> {
            code.byte(0x90);
            Ok(())
        }

        fn undo(&self) -> Option<Self> {
            None
        }
    }

    fn sample() -> Object {
        let mut assembly: Assembly<Nop> = Assembly::new();
        assembly.push(Item::Section(Section::RoData));
        assembly.push(Item::Label(".Lstr".into()));
        assembly.push(Item::Bytes(b"hi\0".to_vec()));
        assembly.push(Item::Section(Section::Text));
        assembly.push(Item::Global("main".into()));
        assembly.push(Item::Function("main".into()));
        assembly.push(Item::Label("main".into()));
        assembly.push(Item::Instruction(Nop));
        assembly.push(Item::Size("main".into()));
        assembly.push(Item::Section(Section::GnuStack));
        assembly.finish().unwrap()
    }

    fn half(file: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(file[at..at + 2].try_into().unwrap())
    }

    fn quad(file: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(file[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn the_header_says_relocatable_and_names_its_machine() {
        let file = write(&sample(), X86_64).unwrap();
        assert_eq!(&file[0..4], b"\x7fELF");
        assert_eq!(file[4], 2, "ELFCLASS64");
        assert_eq!(file[5], 1, "little endian");
        assert_eq!(half(&file, 16), 1, "ET_REL");
        assert_eq!(half(&file, 18), EM_X86_64);
        assert_eq!(half(&file, 52), 64, "e_ehsize");
        assert_eq!(half(&file, 58), 64, "e_shentsize");
        let aarch64 = write(&sample(), AARCH64).unwrap();
        assert_eq!(half(&aarch64, 18), EM_AARCH64);
    }

    #[test]
    fn every_section_header_lies_within_the_file() {
        let file = write(&sample(), X86_64).unwrap();
        let count = half(&file, 60) as usize;
        let start = quad(&file, 40) as usize;
        assert_eq!(start + count * 64, file.len());
        for index in 1..count {
            let header = start + index * 64;
            let kind = u32::from_le_bytes(file[header + 4..header + 8].try_into().unwrap());
            let offset = quad(&file, header + 24) as usize;
            let size = quad(&file, header + 32) as usize;
            // A NOBITS section would occupy no file space; this writer emits
            // none, so every section is really there.
            assert_ne!(kind, 8);
            assert!(
                offset + size <= file.len(),
                "section {index} runs past the file"
            );
            let align = quad(&file, header + 48).max(1) as usize;
            assert_eq!(offset % align, 0, "section {index} is misaligned");
        }
    }

    #[test]
    fn a_relocation_the_architecture_cannot_spell_is_refused() {
        // An AArch64 page reference in an x86-64 object is a compiler bug, and
        // it has to fail here rather than produce a file a linker misreads.
        let mut object = sample();
        object.relocations.push(crate::asm::Relocation {
            offset: 0,
            kind: FixupKind::AdrPage21,
            against: Against::Section(Section::RoData),
            addend: 0,
        });
        let error = write(&object, X86_64).unwrap_err();
        assert!(error.contains("no relocation"), "{error}");
    }

    #[test]
    fn the_symbol_table_puts_every_local_before_every_global() {
        let file = write(&sample(), X86_64).unwrap();
        let count = half(&file, 60) as usize;
        let start = quad(&file, 40) as usize;
        let mut checked = false;
        for index in 1..count {
            let header = start + index * 64;
            let kind = u32::from_le_bytes(file[header + 4..header + 8].try_into().unwrap());
            if kind != SHT_SYMTAB {
                continue;
            }
            let offset = quad(&file, header + 24) as usize;
            let size = quad(&file, header + 32) as usize;
            let info = u32::from_le_bytes(file[header + 44..header + 48].try_into().unwrap());
            let entries = size / SYMBOL_SIZE as usize;
            for entry in 0..entries {
                let at = offset + entry * SYMBOL_SIZE as usize;
                let binding = file[at + 4] >> 4;
                if entry < info as usize {
                    assert_eq!(
                        binding, STB_LOCAL,
                        "symbol {entry} before sh_info is global"
                    );
                } else {
                    assert_eq!(binding, STB_GLOBAL, "symbol {entry} after sh_info is local");
                }
            }
            checked = true;
        }
        assert!(checked, "the object has no symbol table");
    }
}
