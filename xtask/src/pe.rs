// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Lettura delle mitigazioni di un eseguibile Windows a 64 bit (PE32+) direttamente dai
//! byte del file, senza `dumpbin`: la verifica di `cargo xtask dist` non dipende da uno
//! strumento che può mancare (e che, mancando, la faceva passare).
//!
//! Si leggono solo le strutture che servono, con ogni offset e lunghezza controllati:
//! - `DllCharacteristics` dell'intestazione opzionale (ASLR, DEP, `GUARD_CF`);
//! - la directory di configurazione del caricamento (`GuardFlags` di Control Flow Guard e
//!   `DependentLoadFlags`, che dice dove il sistema cerca le DLL importate);
//! - la directory di debug (voce `EX_DLLCHARACTERISTICS` con `CET_COMPAT`);
//! - i nomi delle DLL importate, anche in ritardo.
//!
//! [`Mitigations::problems`] dice che cosa manca rispetto a ciò che il progetto richiede.

use std::fmt;

/// `IMAGE_DLLCHARACTERISTICS_HIGH_ENTROPY_VA`.
const HIGH_ENTROPY_VA: u16 = 0x0020;
/// `IMAGE_DLLCHARACTERISTICS_DYNAMIC_BASE` (ASLR).
const DYNAMIC_BASE: u16 = 0x0040;
/// `IMAGE_DLLCHARACTERISTICS_NX_COMPAT` (DEP).
const NX_COMPAT: u16 = 0x0100;
/// `IMAGE_DLLCHARACTERISTICS_GUARD_CF`.
const GUARD_CF: u16 = 0x4000;
/// `IMAGE_GUARD_CF_INSTRUMENTED`.
const CF_INSTRUMENTED: u32 = 0x0000_0100;
/// `IMAGE_GUARD_CF_FUNCTION_TABLE_PRESENT`.
const CF_FUNCTION_TABLE_PRESENT: u32 = 0x0000_0400;
/// `IMAGE_DEBUG_TYPE_EX_DLLCHARACTERISTICS`.
const DEBUG_TYPE_EX_DLLCHARACTERISTICS: u32 = 20;
/// `IMAGE_DLLCHARACTERISTICS_EX_CET_COMPAT`.
const EX_CET_COMPAT: u32 = 0x0001;
/// `LOAD_LIBRARY_SEARCH_SYSTEM32`: le DLL importate si cercano solo nella cartella di
/// sistema, mai accanto all'eseguibile.
pub const SEARCH_SYSTEM32: u16 = 0x0800;

/// Magia dell'intestazione opzionale PE32+.
const PE32_PLUS_MAGIC: u16 = 0x020B;
/// Indici delle directory di dati.
const DIR_IMPORT: usize = 1;
const DIR_DEBUG: usize = 6;
const DIR_LOAD_CONFIG: usize = 10;
const DIR_DELAY_IMPORT: usize = 13;
/// Dimensioni delle strutture lette.
const SECTION_HEADER_BYTES: usize = 40;
const IMPORT_DESCRIPTOR_BYTES: usize = 20;
const DELAY_DESCRIPTOR_BYTES: usize = 32;
const DEBUG_DIRECTORY_BYTES: usize = 28;
/// Offset nella `IMAGE_LOAD_CONFIG_DIRECTORY64`.
const LOAD_CONFIG_DEPENDENT_LOAD_FLAGS: usize = 0x4E;
const LOAD_CONFIG_GUARD_FLAGS: usize = 0x90;
/// Tetti contro un file malformato: nessun ciclo senza fine.
const MAX_IMPORTS: usize = 4096;
const MAX_NAME_BYTES: usize = 256;

/// Le DLL del runtime C dinamico di Visual C++: l'eseguibile non deve importarle (CRT
/// statico, così parte anche dove il Visual C++ Redistributable non è installato).
const DYNAMIC_CRT_DLLS: &[&str] = &["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll"];

/// Perché un file non si legge come PE32+.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeError(String);

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "eseguibile non leggibile come PE32+: {}", self.0)
    }
}

impl std::error::Error for PeError {}

fn malformed(what: &str) -> PeError {
    PeError(what.to_owned())
}

/// Le mitigazioni lette da un eseguibile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mitigations {
    /// `DllCharacteristics` dell'intestazione opzionale.
    pub dll_characteristics: u16,
    /// `GuardFlags` della configurazione del caricamento (0 se manca).
    pub guard_flags: u32,
    /// `DependentLoadFlags` della configurazione del caricamento (0 se manca).
    pub dependent_load_flags: u16,
    /// Compatibile con CET (shadow stack).
    pub cet_compatible: bool,
    /// Le DLL importate (anche in ritardo), in minuscolo, nell'ordine del file.
    pub imports: Vec<String>,
}

impl Mitigations {
    /// Legge le mitigazioni dai byte di un eseguibile PE32+.
    pub fn read(image: &[u8]) -> Result<Mitigations, PeError> {
        let pe = PeImage::parse(image)?;
        let (guard_flags, dependent_load_flags) = pe.load_config()?;
        let mut imports = pe.import_names()?;
        imports.extend(pe.delay_import_names()?);
        Ok(Mitigations {
            dll_characteristics: pe.dll_characteristics,
            guard_flags,
            dependent_load_flags,
            cet_compatible: pe.cet_compatible()?,
            imports,
        })
    }

    /// Che cosa manca rispetto alle mitigazioni richieste dal progetto; vuoto = tutto
    /// presente.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let flag = |mask: u16| self.dll_characteristics & mask == mask;
        if !flag(DYNAMIC_BASE) || !flag(HIGH_ENTROPY_VA) {
            problems.push("ASLR (DYNAMIC_BASE e HIGH_ENTROPY_VA)".to_owned());
        }
        if !flag(NX_COMPAT) {
            problems.push("DEP (NX_COMPAT)".to_owned());
        }
        let cfg_tables = CF_INSTRUMENTED | CF_FUNCTION_TABLE_PRESENT;
        if !flag(GUARD_CF) || self.guard_flags & cfg_tables != cfg_tables {
            problems.push("Control Flow Guard (GUARD_CF e tabella delle funzioni)".to_owned());
        }
        if !self.cet_compatible {
            problems.push("CETCOMPAT".to_owned());
        }
        if self.dependent_load_flags & SEARCH_SYSTEM32 == 0 {
            problems.push(format!(
                "DependentLoadFlags {:#x} senza LOAD_LIBRARY_SEARCH_SYSTEM32 (0x800)",
                self.dependent_load_flags
            ));
        }
        for dll in &self.imports {
            if DYNAMIC_CRT_DLLS.contains(&dll.as_str()) {
                problems.push(format!("importa il runtime C dinamico {dll}"));
            }
        }
        problems
    }
}

/// Una sezione: dove stanno nel file i byte di un intervallo di indirizzi virtuali.
#[derive(Debug, Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

/// Le parti dell'immagine che servono.
struct PeImage<'a> {
    bytes: &'a [u8],
    dll_characteristics: u16,
    directories: Vec<(u32, u32)>,
    sections: Vec<Section>,
}

impl<'a> PeImage<'a> {
    fn parse(bytes: &'a [u8]) -> Result<PeImage<'a>, PeError> {
        if read_u16(bytes, 0)? != u16::from_le_bytes(*b"MZ") {
            return Err(malformed("manca la firma MZ"));
        }
        let pe_offset = usize::try_from(read_u32(bytes, 0x3C)?)
            .map_err(|_| malformed("offset dell'intestazione PE"))?;
        if read_u32(bytes, pe_offset)? != u32::from_le_bytes(*b"PE\0\0") {
            return Err(malformed("manca la firma PE"));
        }
        let coff = add(pe_offset, 4)?;
        let section_count = usize::from(read_u16(bytes, add(coff, 2)?)?);
        let optional_size = usize::from(read_u16(bytes, add(coff, 16)?)?);
        let optional = add(coff, 20)?;
        if read_u16(bytes, optional)? != PE32_PLUS_MAGIC {
            return Err(malformed("non è un eseguibile a 64 bit (PE32+)"));
        }
        let dll_characteristics = read_u16(bytes, add(optional, 70)?)?;
        let directory_count = usize::try_from(read_u32(bytes, add(optional, 108)?)?)
            .map_err(|_| malformed("numero delle directory"))?
            .min(16);
        let mut directories = Vec::with_capacity(directory_count);
        for index in 0..directory_count {
            let at = add(optional, 112 + index * 8)?;
            directories.push((read_u32(bytes, at)?, read_u32(bytes, add(at, 4)?)?));
        }
        let first_section = add(optional, optional_size)?;
        let mut sections = Vec::with_capacity(section_count);
        for index in 0..section_count {
            let at = add(first_section, index * SECTION_HEADER_BYTES)?;
            sections.push(Section {
                virtual_size: read_u32(bytes, add(at, 8)?)?,
                virtual_address: read_u32(bytes, add(at, 12)?)?,
                raw_size: read_u32(bytes, add(at, 16)?)?,
                raw_offset: read_u32(bytes, add(at, 20)?)?,
            });
        }
        Ok(PeImage {
            bytes,
            dll_characteristics,
            directories,
            sections,
        })
    }

    /// La directory `index` (indirizzo virtuale e dimensione), se presente.
    fn directory(&self, index: usize) -> Option<(u32, u32)> {
        self.directories
            .get(index)
            .copied()
            .filter(|&(rva, size)| rva != 0 && size != 0)
    }

    /// L'offset nel file dell'indirizzo virtuale `rva`, con almeno `len` byte nella stessa
    /// sezione.
    fn offset(&self, rva: u32, len: usize) -> Result<usize, PeError> {
        let len_u32 = u32::try_from(len).map_err(|_| malformed("lunghezza"))?;
        let end = rva
            .checked_add(len_u32)
            .ok_or_else(|| malformed("indirizzo oltre i 4 GiB"))?;
        let section = self
            .sections
            .iter()
            .find(|s| {
                let size = s.virtual_size.max(s.raw_size);
                rva >= s.virtual_address
                    && u64::from(end) <= u64::from(s.virtual_address) + u64::from(size)
            })
            .ok_or_else(|| malformed("indirizzo fuori dalle sezioni"))?;
        let inside = rva - section.virtual_address;
        if u64::from(inside) + u64::from(len_u32) > u64::from(section.raw_size) {
            return Err(malformed(
                "dati oltre la parte della sezione presente nel file",
            ));
        }
        let offset = u64::from(section.raw_offset) + u64::from(inside);
        let offset = usize::try_from(offset).map_err(|_| malformed("offset"))?;
        if offset
            .checked_add(len)
            .is_none_or(|end| end > self.bytes.len())
        {
            return Err(malformed("dati oltre la fine del file"));
        }
        Ok(offset)
    }

    /// `GuardFlags` e `DependentLoadFlags` (zero se la configurazione è più corta o manca).
    fn load_config(&self) -> Result<(u32, u16), PeError> {
        let Some((rva, _)) = self.directory(DIR_LOAD_CONFIG) else {
            return Ok((0, 0));
        };
        // Il primo campo è la dimensione effettiva della struttura.
        let at = self.offset(rva, 4)?;
        let size =
            usize::try_from(read_u32(self.bytes, at)?).map_err(|_| malformed("dimensione"))?;
        let at = self.offset(rva, size)?;
        let config = self
            .bytes
            .get(at..at + size)
            .ok_or_else(|| malformed("configurazione del caricamento"))?;
        let dependent = if size >= LOAD_CONFIG_DEPENDENT_LOAD_FLAGS + 2 {
            read_u16(config, LOAD_CONFIG_DEPENDENT_LOAD_FLAGS)?
        } else {
            0
        };
        let guard = if size >= LOAD_CONFIG_GUARD_FLAGS + 4 {
            read_u32(config, LOAD_CONFIG_GUARD_FLAGS)?
        } else {
            0
        };
        Ok((guard, dependent))
    }

    /// Vero se la directory di debug ha la voce `EX_DLLCHARACTERISTICS` con `CET_COMPAT`.
    fn cet_compatible(&self) -> Result<bool, PeError> {
        let Some((rva, size)) = self.directory(DIR_DEBUG) else {
            return Ok(false);
        };
        let size = usize::try_from(size).map_err(|_| malformed("dimensione"))?;
        let count = size / DEBUG_DIRECTORY_BYTES;
        let at = self.offset(rva, count * DEBUG_DIRECTORY_BYTES)?;
        for index in 0..count.min(MAX_IMPORTS) {
            let entry = add(at, index * DEBUG_DIRECTORY_BYTES)?;
            if read_u32(self.bytes, add(entry, 12)?)? != DEBUG_TYPE_EX_DLLCHARACTERISTICS {
                continue;
            }
            let data_size = read_u32(self.bytes, add(entry, 16)?)?;
            let data_rva = read_u32(self.bytes, add(entry, 20)?)?;
            if data_size < 4 {
                continue;
            }
            let data = self.offset(data_rva, 4)?;
            if read_u32(self.bytes, data)? & EX_CET_COMPAT != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// I nomi delle DLL della tabella delle importazioni.
    fn import_names(&self) -> Result<Vec<String>, PeError> {
        let Some((rva, _)) = self.directory(DIR_IMPORT) else {
            return Ok(Vec::new());
        };
        // Campo `Name` al byte 12 di ogni `IMAGE_IMPORT_DESCRIPTOR`.
        self.descriptor_names(rva, IMPORT_DESCRIPTOR_BYTES, 12)
    }

    /// I nomi delle DLL importate in ritardo.
    fn delay_import_names(&self) -> Result<Vec<String>, PeError> {
        let Some((rva, _)) = self.directory(DIR_DELAY_IMPORT) else {
            return Ok(Vec::new());
        };
        // Campo `DllNameRVA` al byte 4 di ogni `IMAGE_DELAYLOAD_DESCRIPTOR`.
        self.descriptor_names(rva, DELAY_DESCRIPTOR_BYTES, 4)
    }

    /// I nomi di una tabella di descrittori di `stride` byte terminata da un descrittore
    /// tutto a zero; il nome è un RVA al byte `name_field`.
    fn descriptor_names(
        &self,
        rva: u32,
        stride: usize,
        name_field: usize,
    ) -> Result<Vec<String>, PeError> {
        let mut names = Vec::new();
        for index in 0..MAX_IMPORTS {
            let step = u32::try_from(index * stride).map_err(|_| malformed("indice"))?;
            let entry_rva = rva
                .checked_add(step)
                .ok_or_else(|| malformed("tabella delle importazioni"))?;
            let at = self.offset(entry_rva, stride)?;
            let entry = self
                .bytes
                .get(at..at + stride)
                .ok_or_else(|| malformed("descrittore"))?;
            if entry.iter().all(|&b| b == 0) {
                return Ok(names);
            }
            let name_rva = read_u32(entry, name_field)?;
            names.push(self.c_string(name_rva)?.to_ascii_lowercase());
        }
        Err(malformed("tabella delle importazioni senza fine"))
    }

    /// La stringa ASCII terminata da zero all'indirizzo virtuale `rva`.
    fn c_string(&self, rva: u32) -> Result<String, PeError> {
        let at = self.offset(rva, 1)?;
        let rest = self.bytes.get(at..).unwrap_or_default();
        let window = rest
            .get(..MAX_NAME_BYTES.min(rest.len()))
            .unwrap_or_default();
        let end = window
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| malformed("nome di una DLL senza terminatore"))?;
        let name = window.get(..end).unwrap_or_default();
        if !name.iter().all(u8::is_ascii_graphic) {
            return Err(malformed("nome di una DLL non ASCII"));
        }
        Ok(String::from_utf8_lossy(name).into_owned())
    }
}

fn add(base: usize, delta: usize) -> Result<usize, PeError> {
    base.checked_add(delta)
        .ok_or_else(|| malformed("offset oltre i limiti"))
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, PeError> {
    let end = add(at, 2)?;
    bytes
        .get(at..end)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| malformed("file troncato"))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, PeError> {
    let end = add(at, 4)?;
    bytes
        .get(at..end)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| malformed("file troncato"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un'immagine PE32+ minima costruita a mano: una sola sezione con RVA 0x1000 al byte
    /// 0x400 del file, che contiene configurazione del caricamento, directory di debug e
    /// tabella delle importazioni.
    struct Fake {
        dll_characteristics: u16,
        guard_flags: u32,
        dependent_load_flags: u16,
        cet: bool,
        imports: Vec<&'static str>,
    }

    impl Fake {
        /// Tutto ciò che il progetto richiede.
        fn complete() -> Fake {
            Fake {
                dll_characteristics: HIGH_ENTROPY_VA | DYNAMIC_BASE | NX_COMPAT | GUARD_CF,
                guard_flags: CF_INSTRUMENTED | CF_FUNCTION_TABLE_PRESENT,
                dependent_load_flags: SEARCH_SYSTEM32,
                cet: true,
                imports: vec!["KERNEL32.dll", "dwmapi.dll"],
            }
        }

        fn build(&self) -> Vec<u8> {
            const SECTION_RVA: u32 = 0x1000;
            const SECTION_RAW: usize = 0x400;
            const SECTION_SIZE: usize = 0x1000;
            let mut image = vec![0u8; SECTION_RAW + SECTION_SIZE];
            let put16 = |image: &mut Vec<u8>, at: usize, v: u16| {
                image[at..at + 2].copy_from_slice(&v.to_le_bytes());
            };
            let put32 = |image: &mut Vec<u8>, at: usize, v: u32| {
                image[at..at + 4].copy_from_slice(&v.to_le_bytes());
            };
            image[0..2].copy_from_slice(b"MZ");
            put32(&mut image, 0x3C, 0x80);
            image[0x80..0x84].copy_from_slice(b"PE\0\0");
            let coff = 0x84;
            put16(&mut image, coff + 2, 1);
            put16(&mut image, coff + 16, 240);
            let optional = coff + 20;
            put16(&mut image, optional, PE32_PLUS_MAGIC);
            put16(&mut image, optional + 70, self.dll_characteristics);
            put32(&mut image, optional + 108, 16);
            let section = optional + 240;
            image[section..section + 5].copy_from_slice(b".data");
            put32(&mut image, section + 8, 0x1000);
            put32(&mut image, section + 12, SECTION_RVA);
            put32(&mut image, section + 16, 0x1000);
            put32(&mut image, section + 20, 0x400);
            let rva = |offset: usize| SECTION_RVA + u32::try_from(offset).expect("offset");
            let file = |offset: usize| SECTION_RAW + offset;
            let directory = |image: &mut Vec<u8>, index: usize, offset: usize, size: u32| {
                put32(image, optional + 112 + index * 8, rva(offset));
                put32(image, optional + 112 + index * 8 + 4, size);
            };
            // Configurazione del caricamento a 0x000.
            directory(&mut image, DIR_LOAD_CONFIG, 0, 0x140);
            put32(&mut image, file(0), 0x140);
            put16(
                &mut image,
                file(LOAD_CONFIG_DEPENDENT_LOAD_FLAGS),
                self.dependent_load_flags,
            );
            put32(&mut image, file(LOAD_CONFIG_GUARD_FLAGS), self.guard_flags);
            // Directory di debug a 0x200 (una voce), dati a 0x240.
            if self.cet {
                directory(&mut image, DIR_DEBUG, 0x200, 28);
                put32(
                    &mut image,
                    file(0x200 + 12),
                    DEBUG_TYPE_EX_DLLCHARACTERISTICS,
                );
                put32(&mut image, file(0x200 + 16), 4);
                put32(&mut image, file(0x200 + 20), rva(0x240));
                put32(&mut image, file(0x240), EX_CET_COMPAT);
            }
            // Importazioni a 0x300, nomi da 0x500.
            directory(&mut image, DIR_IMPORT, 0x300, 20);
            for (index, name) in self.imports.iter().enumerate() {
                let name_at = 0x500 + index * 0x40;
                image[file(name_at)..file(name_at) + name.len()].copy_from_slice(name.as_bytes());
                put32(&mut image, file(0x300 + index * 20 + 12), rva(name_at));
                put32(&mut image, file(0x300 + index * 20 + 16), 0x9999);
            }
            image
        }
    }

    #[test]
    fn a_complete_image_has_no_problems() {
        let read = Mitigations::read(&Fake::complete().build()).expect("letto");
        assert_eq!(read.imports, ["kernel32.dll", "dwmapi.dll"]);
        assert_eq!(read.dependent_load_flags, SEARCH_SYSTEM32);
        assert!(read.cet_compatible);
        assert_eq!(read.problems(), Vec::<String>::new());
    }

    /// Una modifica dell'immagine completa.
    type Change = fn(&mut Fake);

    /// Ogni mitigazione mancante è un problema con il suo nome.
    #[test]
    fn each_missing_mitigation_is_reported() {
        let cases: [(&str, Change); 6] = [
            ("Control Flow Guard", |f| f.dll_characteristics &= !GUARD_CF),
            ("Control Flow Guard", |f| f.guard_flags = 0),
            ("CETCOMPAT", |f| f.cet = false),
            ("DependentLoadFlags", |f| f.dependent_load_flags = 0),
            ("ASLR", |f| f.dll_characteristics &= !HIGH_ENTROPY_VA),
            ("vcruntime140.dll", |f| f.imports.push("VCRUNTIME140.dll")),
        ];
        for (expected, change) in cases {
            let mut fake = Fake::complete();
            change(&mut fake);
            let problems = Mitigations::read(&fake.build()).expect("letto").problems();
            assert_eq!(problems.len(), 1, "{expected}: {problems:?}");
            assert!(problems[0].contains(expected), "{expected}: {problems:?}");
        }
    }

    /// File troncati o con offset impossibili: errore, mai un panic né un ciclo.
    #[test]
    fn malformed_images_are_errors() {
        let good = Fake::complete().build();
        for len in [0, 1, 0x3C, 0x84, 0x200, 0x400, 0x600] {
            assert!(Mitigations::read(&good[..len]).is_err(), "{len}");
        }
        let mut wild = good.clone();
        wild[0x3C..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Mitigations::read(&wild).is_err());
        let mut not_pe = good;
        not_pe[0] = b'X';
        assert!(Mitigations::read(&not_pe).is_err());
    }

    /// Byte qualsiasi dopo un'intestazione valida: nessun panic.
    #[test]
    fn corrupted_bytes_never_panic() {
        let good = Fake::complete().build();
        for at in (0..good.len()).step_by(7) {
            for value in [0x00, 0xFF, 0x7F] {
                let mut bad = good.clone();
                bad[at] = value;
                let _ = Mitigations::read(&bad).map(|m| m.problems());
            }
        }
    }
}
