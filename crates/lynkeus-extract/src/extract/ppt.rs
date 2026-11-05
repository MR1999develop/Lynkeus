// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! PowerPoint 97-2003 (.ppt): scansione propria dei record del flusso
//! `PowerPoint Document` ([MS-PPT]) sopra il file composto.
//!
//! - Ogni record ha un'intestazione di 8 byte (versione e istanza, tipo, lunghezza); i
//!   contenitori (versione 0xF) contengono altri record. Una lunghezza che esce dal genitore
//!   tronca la scansione di quel contenitore.
//! - Ordine delle diapositive: `Current User` → `UserEditAtom` → catena delle
//!   `PersistDirectoryAtom` (la modifica più recente vince) → `DocumentContainer` →
//!   `SlideListWithText` delle diapositive (istanza 0): ogni `SlidePersistAtom` è una
//!   diapositiva, nell'ordine della presentazione. Se la catena non si ricava, ripiego: i
//!   `SlideContainer` al primo livello del flusso, nell'ordine in cui compaiono.
//! - Testo di una diapositiva ([`Location::Slide`], una riga per paragrafo): gli atomi
//!   `TextCharsAtom` (UTF-16LE) e `TextBytesAtom` (un byte per carattere, Latin-1) che
//!   seguono il suo `SlidePersistAtom` (segnaposto scritti da PowerPoint), poi quelli dentro
//!   il suo `SlideContainer` (caselle di testo dei disegni, come le scrive LibreOffice).
//! - Cifrato: `headerToken` di `Current User`, `encryptSessionPersistIdRef` nella
//!   `UserEditAtom` o un `CryptSession10Container`: avviso "protetto da password".
//!
//! Non si estraggono note del relatore, schemi (master), commenti, testo negli oggetti
//! incorporati (grafici, tabelle OLE). Ogni `SlideContainer` si legge al più una volta, così
//! il testo emesso non supera il flusso.

use std::collections::{HashMap, HashSet};

use lynkeus_core::Location;

use crate::error::ExtractError;
use crate::extract::cfbfile::{Compound, to_usize, u16_at, u32_at};
use crate::extract::{Ctx, Fail, Input};

const PPT_STREAM: &str = "PowerPoint Document";
const CURRENT_USER_STREAM: &str = "Current User";

/// `headerToken` di un documento cifrato.
const ENCRYPTED_TOKEN: u32 = 0xF3D1_C4DF;

const RT_DOCUMENT: u16 = 0x03E8;
const RT_SLIDE: u16 = 0x03EE;
const RT_SLIDE_PERSIST_ATOM: u16 = 0x03F3;
const RT_TEXT_CHARS: u16 = 0x0FA0;
const RT_TEXT_BYTES: u16 = 0x0FA8;
const RT_SLIDE_LIST_WITH_TEXT: u16 = 0x0FF0;
const RT_USER_EDIT: u16 = 0x0FF5;
const RT_PERSIST_DIRECTORY: u16 = 0x1772;
const RT_CRYPT_SESSION: u16 = 0x2F14;

/// Versione dei contenitori.
const CONTAINER_VERSION: u8 = 0x0F;
const HEADER_BYTES: usize = 8;
/// Annidamento massimo dei contenitori letto: oltre si ignora.
const MAX_RECORD_DEPTH: usize = 32;
/// Modifiche incrementali al più seguite nella catena delle `UserEditAtom`.
const MAX_EDITS: usize = 4096;
/// Byte di una `UserEditAtom` che ha `encryptSessionPersistIdRef`.
const USER_EDIT_WITH_CRYPT: usize = 32;

/// Un record con il suo contenuto.
#[derive(Debug, Clone, Copy)]
struct Record<'a> {
    version: u8,
    instance: u16,
    kind: u16,
    body: &'a [u8],
}

impl Record<'_> {
    fn is_container(&self) -> bool {
        self.version == CONTAINER_VERSION
    }
}

/// Il record all'offset `at` di `data`, o `None` se l'intestazione o il contenuto escono.
fn record_at(data: &[u8], at: usize) -> Option<(Record<'_>, usize)> {
    let head = u16_at(data, at)?;
    let kind = u16_at(data, at.checked_add(2)?)?;
    let len = to_usize(u32_at(data, at.checked_add(4)?)?);
    let start = at.checked_add(HEADER_BYTES)?;
    let end = start.checked_add(len)?;
    let body = data.get(start..end)?;
    let version = u8::try_from(head & 0x000F).unwrap_or(0);
    Some((
        Record {
            version,
            instance: head >> 4,
            kind,
            body,
        },
        end,
    ))
}

/// I record uno dopo l'altro in `data` (il contenuto di un contenitore, o il flusso), fino al
/// primo che non sta nei byte.
fn records(data: &[u8]) -> impl Iterator<Item = Record<'_>> {
    let mut at = 0_usize;
    std::iter::from_fn(move || {
        let (record, end) = record_at(data, at)?;
        at = end;
        Some(record)
    })
}

/// Una diapositiva trovata: il suo contenitore e il testo dei segnaposto.
#[derive(Debug, Default)]
struct SlideRef {
    container: Option<usize>,
    outline: Vec<String>,
}

/// Estrae le diapositive di un .ppt.
pub(crate) fn extract(ctx: &mut Ctx<'_>, input: Input<'_>) -> Result<(), Fail> {
    let mut compound = Compound::open(ctx, input)?;
    let Some(stream) = compound.stream(ctx, PPT_STREAM)? else {
        return Err(ExtractError::ContentMismatch.into());
    };
    let current_user = compound.stream(ctx, CURRENT_USER_STREAM)?;
    for (number, text) in slide_texts(&stream, current_user.as_deref())? {
        ctx.unit(Location::Slide(number), &text)?;
    }
    Ok(())
}

/// Il testo di ogni diapositiva con testo, con il suo numero (da 1).
pub(crate) fn slide_texts(
    stream: &[u8],
    current_user: Option<&[u8]>,
) -> Result<Vec<(u32, String)>, ExtractError> {
    let edit = match current_user {
        Some(user) => current_edit(user)?,
        None => None,
    };
    let slides = match edit {
        Some(offset) => match slides_by_persist(stream, offset)? {
            Some(slides) => slides,
            None => slides_in_stream_order(stream)?,
        },
        None => slides_in_stream_order(stream)?,
    };
    let mut visited = HashSet::new();
    let mut out = Vec::new();
    for (index, slide) in slides.into_iter().enumerate() {
        let mut lines = slide.outline;
        if let Some(offset) = slide.container
            && visited.insert(offset)
            && let Some((record, _)) = record_at(stream, offset)
            && record.kind == RT_SLIDE
        {
            collect_text(record.body, 0, &mut lines);
        }
        let text = lines.join("\n");
        if !text.trim().is_empty() {
            let number = u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX);
            out.push((number, text));
        }
    }
    Ok(out)
}

/// `offsetToCurrentEdit` del flusso `Current User`; [`ExtractError::Encrypted`] se il
/// documento è cifrato, `None` se il flusso non è valido.
fn current_edit(user: &[u8]) -> Result<Option<usize>, ExtractError> {
    let Some((record, _)) = record_at(user, 0) else {
        return Ok(None);
    };
    // size (4 byte), headerToken, offsetToCurrentEdit.
    if u32_at(record.body, 4) == Some(ENCRYPTED_TOKEN) {
        return Err(ExtractError::Encrypted);
    }
    Ok(u32_at(record.body, 8).map(to_usize))
}

/// Ciò che si ricava dalla catena delle modifiche incrementali.
#[derive(Debug, Default, PartialEq, Eq)]
struct EditChain {
    /// Offset delle `PersistDirectoryAtom`, ognuno una volta, dalla modifica più recente.
    directories: Vec<usize>,
    /// `docPersistIdRef` della modifica più recente che lo ha.
    document_ref: Option<u32>,
}

/// Segue la catena delle `UserEditAtom` da `edit` (al più [`MAX_EDITS`] anelli, ognuno una
/// volta); `None` se un anello non è una `UserEditAtom`. Una directory a cui puntano più
/// modifiche compare una volta sola: rileggerla non aggiungerebbe nulla, e migliaia di
/// modifiche di pochi byte moltiplicherebbero il lavoro.
fn edit_chain(stream: &[u8], edit: usize) -> Result<Option<EditChain>, ExtractError> {
    let mut chain = EditChain::default();
    let mut visited = HashSet::new();
    let mut directories = HashSet::new();
    let mut next = Some(edit);
    while let Some(offset) = next {
        if visited.len() >= MAX_EDITS || !visited.insert(offset) {
            break;
        }
        let Some((record, _)) = record_at(stream, offset) else {
            return Ok(None);
        };
        if record.kind != RT_USER_EDIT {
            return Ok(None);
        }
        let body = record.body;
        if body.len() >= USER_EDIT_WITH_CRYPT {
            return Err(ExtractError::Encrypted);
        }
        if chain.document_ref.is_none() {
            chain.document_ref = u32_at(body, 16);
        }
        if let Some(directory) = u32_at(body, 12).map(to_usize)
            && directories.insert(directory)
        {
            chain.directories.push(directory);
        }
        next = u32_at(body, 8).map(to_usize).filter(|&last| last != 0);
    }
    Ok(Some(chain))
}

/// Le diapositive nell'ordine della `SlideListWithText`, seguendo la catena delle modifiche
/// da `edit`; `None` se la catena non porta a un `DocumentContainer`.
fn slides_by_persist(stream: &[u8], edit: usize) -> Result<Option<Vec<SlideRef>>, ExtractError> {
    let Some(EditChain {
        directories,
        document_ref,
    }) = edit_chain(stream, edit)?
    else {
        return Ok(None);
    };
    let mut persist: HashMap<u32, usize> = HashMap::new();
    for directory in directories {
        read_persist_directory(stream, directory, &mut persist);
    }
    let Some(document) = document_ref
        .and_then(|id| persist.get(&id).copied())
        .and_then(|offset| record_at(stream, offset))
        .map(|(record, _)| record)
        .filter(|record| record.kind == RT_DOCUMENT)
    else {
        return Ok(None);
    };
    let Some(list) = records(document.body)
        .find(|r| r.kind == RT_SLIDE_LIST_WITH_TEXT && r.instance == 0 && r.is_container())
    else {
        // Una presentazione senza diapositive.
        return Ok(Some(Vec::new()));
    };
    let mut slides: Vec<SlideRef> = Vec::new();
    for record in records(list.body) {
        match record.kind {
            RT_SLIDE_PERSIST_ATOM => slides.push(SlideRef {
                container: u32_at(record.body, 0).and_then(|id| persist.get(&id).copied()),
                outline: Vec::new(),
            }),
            RT_TEXT_CHARS | RT_TEXT_BYTES => {
                if let Some(slide) = slides.last_mut() {
                    push_text_atom(&record, &mut slide.outline);
                }
            }
            _ => {}
        }
    }
    Ok(Some(slides))
}

/// Aggiunge a `persist` le voci (id → offset) della `PersistDirectoryAtom` all'offset `at`,
/// senza sostituire quelle già presenti (di modifiche più recenti).
fn read_persist_directory(stream: &[u8], at: usize, persist: &mut HashMap<u32, usize>) {
    let Some((record, _)) = record_at(stream, at) else {
        return;
    };
    if record.kind != RT_PERSIST_DIRECTORY {
        return;
    }
    let body = record.body;
    let mut pos = 0_usize;
    while let Some(entry) = u32_at(body, pos) {
        let first = entry & 0x000F_FFFF;
        let count = entry >> 20;
        pos += 4;
        for k in 0..count {
            let Some(offset) = u32_at(body, pos) else {
                return;
            };
            pos += 4;
            persist
                .entry(first.saturating_add(k))
                .or_insert_with(|| to_usize(offset));
        }
    }
}

/// Ripiego: i `SlideContainer` al primo livello del flusso, nell'ordine.
fn slides_in_stream_order(stream: &[u8]) -> Result<Vec<SlideRef>, ExtractError> {
    let mut slides = Vec::new();
    let mut at = 0_usize;
    while let Some((record, end)) = record_at(stream, at) {
        match record.kind {
            RT_CRYPT_SESSION => return Err(ExtractError::Encrypted),
            RT_SLIDE => slides.push(SlideRef {
                container: Some(at),
                outline: Vec::new(),
            }),
            _ => {}
        }
        at = end;
    }
    Ok(slides)
}

/// Il testo degli atomi di testo dentro `body`, in ordine, a qualsiasi profondità (fino a
/// [`MAX_RECORD_DEPTH`]).
fn collect_text(body: &[u8], depth: usize, lines: &mut Vec<String>) {
    if depth >= MAX_RECORD_DEPTH {
        return;
    }
    for record in records(body) {
        if record.is_container() {
            collect_text(record.body, depth + 1, lines);
        } else if matches!(record.kind, RT_TEXT_CHARS | RT_TEXT_BYTES) {
            push_text_atom(&record, lines);
        }
    }
}

/// Le righe di un atomo di testo: paragrafi (`\r`) e a capo (0x0B) diventano righe, gli altri
/// caratteri di controllo si tolgono.
fn push_text_atom(record: &Record<'_>, lines: &mut Vec<String>) {
    let text: String = if record.kind == RT_TEXT_CHARS {
        let units = record
            .body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair));
        char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    } else {
        record.body.iter().map(|&b| char::from(b)).collect()
    };
    for line in text.split(['\r', '\u{b}', '\n']) {
        let clean: String = line
            .chars()
            .filter(|c| *c == '\t' || !c.is_control())
            .collect();
        if !clean.trim().is_empty() {
            lines.push(clean);
        }
    }
}

/// Punto d'ingresso per il fuzzing: `data` è il flusso `PowerPoint Document`; se i primi
/// byte sono un `Current User` valido (record di tipo 0x0FF6 in testa) li si usa come tale e
/// il resto come flusso. Non deve mai andare in panic.
#[cfg(any(test, feature = "fuzzing"))]
pub(crate) fn fuzz(data: &[u8]) {
    let (user, stream) = match record_at(data, 0) {
        Some((record, end)) if record.kind == 0x0FF6 => (data.get(..end), data.get(end..)),
        _ => (None, Some(data)),
    };
    let _ignored = slide_texts(stream.unwrap_or_default(), user);
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn record(version: u8, instance: u16, kind: u16, body: &[u8]) -> Vec<u8> {
        let head = (instance << 4) | u16::from(version);
        let mut out = Vec::new();
        out.extend_from_slice(&head.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    fn container(kind: u16, instance: u16, children: &[Vec<u8>]) -> Vec<u8> {
        record(CONTAINER_VERSION, instance, kind, &children.concat())
    }

    fn chars(text: &str) -> Vec<u8> {
        let body: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        record(0, 0, RT_TEXT_CHARS, &body)
    }

    fn slide(text: &[u8]) -> Vec<u8> {
        // Il testo di LibreOffice: dentro un disegno, a due livelli.
        let textbox = container(0xF00D, 0, &[text.to_vec()]);
        let drawing = container(0x040C, 0, &[container(0xF002, 0, &[textbox])]);
        container(RT_SLIDE, 0, &[drawing])
    }

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Un flusso con le diapositive `slides` (in ordine di flusso) e una presentazione che le
    /// elenca nell'ordine `order` (indici in `slides`).
    fn presentation(slides: &[Vec<u8>], order: &[usize]) -> (Vec<u8>, Vec<u8>) {
        let mut stream = Vec::new();
        let mut offsets = Vec::new();
        for s in slides {
            offsets.push(u32::try_from(stream.len()).unwrap_or(0));
            stream.extend_from_slice(s);
        }
        // persistId: 1 = documento, 2.. = diapositive.
        // Un testo prima della prima diapositiva non appartiene a nessuna.
        let mut list = vec![chars("non una diapositiva")];
        for &index in order {
            let id = u32::try_from(index + 2).unwrap_or(0);
            list.push(record(
                0,
                0,
                RT_SLIDE_PERSIST_ATOM,
                &u32s(&[id, 0, 0, 0, 0]),
            ));
        }
        let master_list = container(RT_SLIDE_LIST_WITH_TEXT, 1, &[chars("schema")]);
        let document_at = u32::try_from(stream.len()).unwrap_or(0);
        stream.extend_from_slice(&container(
            RT_DOCUMENT,
            0,
            &[master_list, container(RT_SLIDE_LIST_WITH_TEXT, 0, &list)],
        ));
        let directory_at = u32::try_from(stream.len()).unwrap_or(0);
        let count = u32::try_from(1 + slides.len()).unwrap_or(0);
        let mut directory = vec![1 | (count << 20), document_at];
        directory.extend_from_slice(&offsets);
        stream.extend_from_slice(&record(0, 0, RT_PERSIST_DIRECTORY, &u32s(&directory)));
        let edit_at = u32::try_from(stream.len()).unwrap_or(0);
        let mut edit = u32s(&[0, 0, 0, directory_at, 1, 0]);
        edit.extend_from_slice(&[0, 0, 0, 0]);
        stream.extend_from_slice(&record(0, 0, RT_USER_EDIT, &edit));
        let user = record(0, 0, 0x0FF6, &u32s(&[0x14, 0xE391_C05F, edit_at]));
        (stream, user)
    }

    #[test]
    fn slides_follow_the_presentation_order() {
        let slides = [slide(&chars("prima")), slide(&chars("Stasera\raragosta"))];
        let (stream, user) = presentation(&slides, &[1, 0]);
        let texts = slide_texts(&stream, Some(&user)).ok();
        assert_eq!(
            texts,
            Some(vec![
                (1, "Stasera\naragosta".to_owned()),
                (2, "prima".to_owned())
            ])
        );
    }

    #[test]
    fn without_current_user_slides_are_in_stream_order() {
        let slides = [
            slide(&chars("prima")),
            slide(&record(0, 0, RT_TEXT_BYTES, b"citt\xE0")),
        ];
        let (stream, _) = presentation(&slides, &[1, 0]);
        let texts = slide_texts(&stream, None).ok();
        assert_eq!(
            texts,
            Some(vec![(1, "prima".to_owned()), (2, "città".to_owned())])
        );
    }

    #[test]
    fn repeated_slide_reference_is_read_once() {
        let slides = [slide(&chars("una volta"))];
        let (stream, user) = presentation(&slides, &[0, 0, 0]);
        let texts = slide_texts(&stream, Some(&user)).ok();
        assert_eq!(texts, Some(vec![(1, "una volta".to_owned())]));
    }

    /// Molte modifiche che puntano alla stessa `PersistDirectoryAtom`: la directory si
    /// legge una volta sola, quelle diverse una volta ciascuna, dalla più recente.
    #[test]
    fn repeated_persist_directory_is_read_once() {
        let directory = record(0, 0, RT_PERSIST_DIRECTORY, &u32s(&[1 | (1 << 20), 0]));
        let mut stream = directory.clone();
        let other = u32::try_from(stream.len()).unwrap_or(0);
        stream.extend_from_slice(&directory);
        // Dieci modifiche in catena: la prima (la più vecchia) punta all'altra directory.
        let mut previous = 0_u32;
        for index in 0..10 {
            let at = u32::try_from(stream.len()).unwrap_or(0);
            let target = if index == 0 { other } else { 0 };
            let mut body = u32s(&[0, 0, previous, target, 1, 0]);
            body.extend_from_slice(&[0; 4]);
            stream.extend_from_slice(&record(0, 0, RT_USER_EDIT, &body));
            previous = at;
        }
        let chain = edit_chain(&stream, to_usize(previous)).ok().flatten();
        assert_eq!(
            chain,
            Some(EditChain {
                directories: vec![0, to_usize(other)],
                document_ref: Some(1),
            })
        );
    }

    #[test]
    fn encryption_partitions() {
        let user = record(0, 0, 0x0FF6, &u32s(&[0x14, ENCRYPTED_TOKEN, 0]));
        assert!(matches!(
            slide_texts(&[], Some(&user)),
            Err(ExtractError::Encrypted)
        ));
        let crypt = container(RT_CRYPT_SESSION, 0, &[]);
        assert!(matches!(
            slide_texts(&crypt, None),
            Err(ExtractError::Encrypted)
        ));
        // UserEditAtom con encryptSessionPersistIdRef.
        let edit = record(0, 0, RT_USER_EDIT, &[0; USER_EDIT_WITH_CRYPT]);
        let user = record(0, 0, 0x0FF6, &u32s(&[0x14, 0xE391_C05F, 0]));
        assert!(matches!(
            slide_texts(&edit, Some(&user)),
            Err(ExtractError::Encrypted)
        ));
    }

    #[test]
    fn record_lengths_are_bounded_by_the_parent() {
        // Un figlio che dichiara più byte del genitore viene ignorato.
        let mut liar = record(0, 0, RT_TEXT_CHARS, b"a\0");
        liar[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let stream = container(RT_SLIDE, 0, &[liar]);
        assert_eq!(slide_texts(&stream, None).ok(), Some(Vec::new()));
        assert!(record_at(&[1, 2, 3], 0).is_none());
    }

    #[test]
    fn deep_nesting_is_cut() {
        let mut inner = chars("profondo");
        for _ in 0..(MAX_RECORD_DEPTH + 5) {
            inner = container(0xF003, 0, &[inner]);
        }
        let stream = container(RT_SLIDE, 0, &[inner]);
        assert_eq!(slide_texts(&stream, None).ok(), Some(Vec::new()));
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_bytes(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
            fuzz(&data);
        }

        #[test]
        fn never_panics_on_damaged_presentations(
            cut in 0_usize..1024,
            flip in 0_usize..1024,
            value in any::<u8>(),
        ) {
            let slides = [slide(&chars("prima")), slide(&chars("seconda"))];
            let (mut stream, mut user) = presentation(&slides, &[0, 1]);
            if let Some(byte) = stream.get_mut(flip) {
                *byte = value;
            }
            if let Some(byte) = user.get_mut(flip % 20) {
                *byte ^= value;
            }
            stream.truncate(cut);
            if let Ok(texts) = slide_texts(&stream, Some(&user)) {
                let total: usize = texts.iter().map(|(_, t)| t.len()).sum();
                prop_assert!(total <= stream.len() * 2);
            }
        }
    }
}
