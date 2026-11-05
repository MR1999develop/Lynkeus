// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! Mark-of-the-Web: la zona d'origine che Windows scrive nello stream alternativo
//! `Zone.Identifier` dei file scaricati, e che Office, SmartScreen e la shell usano per
//! aprirli in modo protetto (vista protetta, macro bloccate).
//!
//! Una voce estratta da un archivio per aprirla riceve la zona dell'archivio sul disco
//! ([`copy_mark_of_the_web`]): un documento con macro dentro uno zip scaricato resta "da
//! Internet" anche aperto dalla copia privata dell'app.
//!
//! Solo Windows: su Linux i file non hanno una zona d'origine.
//!
//! Il contenuto dello stream dell'archivio è input non fidato: se ne legge al più
//! [`MAX_STREAM_BYTES`] byte e se ne riconosce solo `ZoneId` ([`parse_zone_identifier`]);
//! sulla voce si scrive un testo generato qui ([`zone_identifier_text`]), senza gli URL
//! d'origine. Uno stream presente ma illeggibile o senza zona valida conta come Internet
//! (fallire chiusi).

use std::io;
use std::path::Path;

/// Byte massimi letti dallo stream `Zone.Identifier` di un archivio.
pub const MAX_STREAM_BYTES: u64 = 64 * 1024;

/// Una zona di sicurezza di Windows (`URLZONE`): 0 computer locale, 1 Intranet, 2 siti
/// attendibili, 3 Internet, 4 siti con restrizioni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zone(u8);

impl Zone {
    /// Internet: la zona dei file scaricati, e quella usata quando lo stream non si capisce.
    pub const INTERNET: Zone = Zone(3);

    /// La zona `id`, se è una delle cinque note.
    pub fn new(id: u32) -> Option<Zone> {
        u8::try_from(id).ok().filter(|id| *id <= 4).map(Zone)
    }

    /// Il numero della zona.
    pub fn id(self) -> u8 {
        self.0
    }
}

/// La zona scritta in un `Zone.Identifier` (formato INI: sezione `[ZoneTransfer]`, chiave
/// `ZoneId`). `None` se il testo non ha una zona valida.
pub fn parse_zone_identifier(bytes: &[u8]) -> Option<Zone> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[ZoneTransfer]");
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if in_section && key.trim().eq_ignore_ascii_case("ZoneId") {
            let value = value.trim();
            if value.is_empty() || value.len() > 3 || !value.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            return value.parse().ok().and_then(Zone::new);
        }
    }
    None
}

/// Il contenuto di `Zone.Identifier` per `zone`, come lo scrive Windows (senza URL).
pub fn zone_identifier_text(zone: Zone) -> String {
    format!("[ZoneTransfer]\r\nZoneId={}\r\n", zone.id())
}

/// La zona di un file letta dal contenuto (al più [`MAX_STREAM_BYTES`] byte) del suo
/// stream: quella scritta, o Internet se lo stream c'è ma non si capisce.
fn zone_of_stream(bytes: &[u8]) -> Zone {
    parse_zone_identifier(bytes).unwrap_or(Zone::INTERNET)
}

/// Copia la zona d'origine di `source` (l'archivio sul disco) su `target` (il file appena
/// estratto dall'app). Senza zona sull'archivio non scrive nulla.
///
/// Windows: stream `Zone.Identifier`. Uno stream che c'è ma non si legge conta come
/// Internet. Un volume senza stream alternativi (FAT, exFAT) non ha zone: nulla da copiare.
/// Errore se la zona non si scrive su `target`: chi chiama non deve aprire il file.
pub fn copy_mark_of_the_web(source: &Path, target: &Path) -> io::Result<()> {
    use std::io::Write;

    let Some(zone) = read_zone(source) else {
        return Ok(());
    };
    // Nuovo: uno stream già presente non si sovrascrive.
    let mut stream = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stream_path(target))?;
    stream.write_all(zone_identifier_text(zone).as_bytes())?;
    stream.flush()
}

/// La zona di `source`; `None` se non ha lo stream (o il volume non ha stream).
fn read_zone(source: &Path) -> Option<Zone> {
    use std::io::Read;

    let opened = std::fs::File::open(stream_path(source));
    let mut stream = match opened {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename
            ) =>
        {
            return None;
        }
        Err(error) => {
            tracing::warn!(%error, "zona d'origine dell'archivio non letta: Internet");
            return Some(Zone::INTERNET);
        }
    };
    let mut bytes = Vec::new();
    match (&mut stream).take(MAX_STREAM_BYTES).read_to_end(&mut bytes) {
        Ok(_) => Some(zone_of_stream(&bytes)),
        Err(error) => {
            tracing::warn!(%error, "zona d'origine dell'archivio non letta: Internet");
            Some(Zone::INTERNET)
        }
    }
}

/// Il percorso dello stream `Zone.Identifier` di `path`.
fn stream_path(path: &Path) -> std::ffi::OsString {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    stream
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_identifier_partitions() {
        let windows = b"[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://example.invalid/\r\nHostUrl=https://example.invalid/a.zip\r\n";
        assert_eq!(parse_zone_identifier(windows), Some(Zone::INTERNET));
        assert_eq!(
            parse_zone_identifier(b"[zonetransfer]\nzoneid = 4\n"),
            Zone::new(4)
        );
        assert_eq!(
            parse_zone_identifier(b"[ZoneTransfer]\nZoneId=0"),
            Zone::new(0)
        );
        for bad in [
            &b""[..],
            b"ZoneId=3",
            b"[Altro]\nZoneId=3",
            b"[ZoneTransfer]\nZoneId=5",
            b"[ZoneTransfer]\nZoneId=-1",
            b"[ZoneTransfer]\nZoneId=",
            b"[ZoneTransfer]\nZoneId=99999999999",
            b"[ZoneTransfer]\nZoneId=3x",
            b"\xFF\xFE[\x00Z\x00",
        ] {
            assert_eq!(parse_zone_identifier(bad), None, "{bad:?}");
        }
        // Uno stream che c'è ma non si capisce: Internet.
        assert_eq!(zone_of_stream(b"\xFF garbage"), Zone::INTERNET);
    }

    #[test]
    fn the_written_text_reads_back() {
        for id in 0..=4 {
            let zone = Zone::new(id).expect("zona");
            let text = zone_identifier_text(zone);
            assert_eq!(parse_zone_identifier(text.as_bytes()), Some(zone));
            assert!(!text.contains("Url"));
        }
        assert_eq!(Zone::new(5), None);
    }

    /// Lo stream dell'archivio scaricato passa al file estratto (solo la zona); un file
    /// senza zona non ne riceve una; un file che ha già lo stream non si sovrascrive.
    #[test]
    fn the_zone_is_copied_from_the_archive() {
        let dir = tempfile::tempdir().expect("cartella");
        let archive = dir.path().join("scaricato.zip");
        std::fs::write(&archive, b"PK").expect("archivio");
        std::fs::write(
            stream_path(&archive),
            b"[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.invalid/x.zip\r\n",
        )
        .expect("zona dell'archivio");
        let extracted = dir.path().join("voce.docm");
        std::fs::write(&extracted, b"x").expect("voce");
        copy_mark_of_the_web(&archive, &extracted).expect("zona copiata");
        let copied = std::fs::read(stream_path(&extracted)).expect("zona della voce");
        assert_eq!(copied, zone_identifier_text(Zone::INTERNET).as_bytes());
        // Già presente: errore, nessuna sovrascrittura silenziosa.
        assert!(copy_mark_of_the_web(&archive, &extracted).is_err());

        let local = dir.path().join("locale.zip");
        std::fs::write(&local, b"PK").expect("archivio locale");
        let plain = dir.path().join("voce.txt");
        std::fs::write(&plain, b"x").expect("voce");
        copy_mark_of_the_web(&local, &plain).expect("nulla da copiare");
        assert_eq!(
            std::fs::read(stream_path(&plain)).map_err(|e| e.kind()),
            Err(io::ErrorKind::NotFound)
        );
    }
}
