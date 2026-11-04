// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 MR

//! PDF sintetici ostili per il prototipo: si scrivono in una cartella temporanea, mai nel repository.

use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// Scrive tutti i casi in `dir` (che deve esistere).
pub fn write_all(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join("vuoto.pdf"), b"")?;
    let mut garbage = b"%PDF-1.7\n".to_vec();
    garbage.extend(
        (0u32..4096).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap_or(0)),
    );
    std::fs::write(dir.join("intestazione-spazzatura.pdf"), garbage)?;
    std::fs::write(
        dir.join("valido-minimo.pdf"),
        minimal(TEXT_CONTENT, None, "1"),
    )?;
    std::fs::write(dir.join("ciclo-pagine.pdf"), cycle())?;
    std::fs::write(
        dir.join("conteggio-falso.pdf"),
        minimal(TEXT_CONTENT, None, "2147483647"),
    )?;
    std::fs::write(
        dir.join("bomba-flate.pdf"),
        minimal(TEXT_CONTENT, Some(bomb_stream()?), "1"),
    )?;
    let depth = 200_000;
    let mut nested = String::from("BT /F1 12 Tf 72 720 Td (aragosta) Tj ET\n");
    nested.push_str(&"[".repeat(depth));
    nested.push_str(&"]".repeat(depth));
    std::fs::write(
        dir.join("annidamento-profondo.pdf"),
        minimal(nested.as_bytes(), None, "1"),
    )?;
    Ok(())
}

const TEXT_CONTENT: &[u8] = b"BT /F1 12 Tf 72 720 Td (Citt\\340 e aragosta) Tj ET";

/// Flusso compresso di 1 GiB di spazi (circa 1 MB su disco), con il testo in testa.
fn bomb_stream() -> std::io::Result<Vec<u8>> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(TEXT_CONTENT)?;
    let block = vec![b' '; 1 << 20];
    for _ in 0..1024 {
        enc.write_all(&block)?;
    }
    enc.finish()
}

/// PDF a una pagina con xref corretto. `flate` sostituisce il contenuto con un flusso già compresso.
fn minimal(content: &[u8], flate: Option<Vec<u8>>, count: &str) -> Vec<u8> {
    let (stream, filter) = match flate {
        Some(data) => (data, " /Filter /FlateDecode"),
        None => (content.to_vec(), ""),
    };
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [3 0 R] /Count {count} >>").into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
        {
            let mut s = format!("<< /Length {}{filter} >>\nstream\n", stream.len()).into_bytes();
            s.extend_from_slice(&stream);
            s.extend_from_slice(b"\nendstream");
            s
        },
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    assemble(&objects)
}

/// Il nodo `Pages` ha come figlio se stesso.
fn cycle() -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [2 0 R 3 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
        {
            let mut s = format!("<< /Length {} >>\nstream\n", TEXT_CONTENT.len()).into_bytes();
            s.extend_from_slice(TEXT_CONTENT);
            s.extend_from_slice(b"\nendstream");
            s
        },
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    assemble(&objects)
}

/// Numera gli oggetti da 1, scrive xref e trailer con gli offset corretti.
fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
