# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 MR
"""Genera i file malevoli di fixtures/malevoli/ che non richiedono LibreOffice (rieseguibile,
byte per byte deterministico): solo la libreria standard di Python.

    python -I fixtures/src/genera_malevoli.py

- sparso.xlsx: un foglio con due sole celle, A1 e XFD1048576 (l'ultima cella possibile);
  un lettore che riserva righe x colonne celle chiede centinaia di GB.
- sparso.zip: sparso.xlsx seguito da nota.txt, per provare che le voci dopo il foglio si
  leggono ancora.
- slide-ripetute.pptx: una presentazione che elenca 10.000 volte la stessa diapositiva.
- testo-ripetuto.ods: una cella di testo di 300 caratteri ripetuta su 16.384 colonne e 255
  righe (meno di 1 KB): un lettore che clona il testo per ogni ripetizione supera 1 GB.
- dizionario-enorme.7z: un 7z (LZMA2, intestazione non compressa) con una riga di testo e il
  dizionario dichiarato portato a 2 GiB: un decodificatore che lo allocasse subito
  supererebbe il limite di memoria dell'estrattore. Richiede 7-Zip
  (C:\\Program Files\\7-Zip\\7z.exe); senza, il file non si rigenera.
- nomi-lunghi.tar.gz: un tar con un nome lungo GNU (`L`) di 128 KiB e un'estensione PAX
  (`x`) di 128 KiB, ognuno davanti a una voce con `aragosta`, poi un nome lungo GNU e un
  percorso PAX di 300 caratteri (entro il tetto) e `dopo.txt`: le due intestazioni enormi
  superano il tetto di 64 KiB del lettore (avviso per la loro voce), le voci seguenti si
  leggono.
"""

import gzip
import io
import struct
import subprocess
import tarfile
import tempfile
import zipfile
import zlib
from pathlib import Path

MAL = Path(__file__).resolve().parent.parent / "malevoli"
SEVENZ = Path(r"C:\Program Files\7-Zip\7z.exe")

# Data fissa delle voci zip (2026-01-01 00:00).
DATE = (2026, 1, 1, 0, 0, 0)


def zip_bytes(entries):
    """Uno zip deflate con le voci (nome, byte) nell'ordine dato."""
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in entries:
            info = zipfile.ZipInfo(name, DATE)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, data)
    return buf.getvalue()


def write(name, data):
    MAL.mkdir(parents=True, exist_ok=True)
    (MAL / name).write_bytes(data)
    print(f"{name}: {len(data)} byte")


def sparse_xlsx():
    content_types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
        '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
        '<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>'
        "</Types>"
    )
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
        "</Relationships>"
    )
    workbook = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        '<sheets><sheet name="Sparso" sheetId="1" r:id="rId1"/></sheets></workbook>'
    )
    workbook_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>'
        "</Relationships>"
    )
    shared = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2">'
        "<si><t>aragosta</t></si><si><t>città lontana</t></si></sst>"
    )
    sheet = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
        '<dimension ref="A1:XFD1048576"/><sheetData>'
        '<row r="1"><c r="A1" t="s"><v>0</v></c></row>'
        '<row r="1048576"><c r="XFD1048576" t="s"><v>1</v></c></row>'
        "</sheetData></worksheet>"
    )
    return zip_bytes(
        [
            ("[Content_Types].xml", content_types.encode()),
            ("_rels/.rels", rels.encode()),
            ("xl/workbook.xml", workbook.encode()),
            ("xl/_rels/workbook.xml.rels", workbook_rels.encode()),
            ("xl/sharedStrings.xml", shared.encode("utf-8")),
            ("xl/worksheets/sheet1.xml", sheet.encode()),
        ]
    )


def repeated_slides_pptx():
    ids = "".join('<p:sldId id="256" r:id="rId2"/>' for _ in range(10_000))
    presentation = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
        f"<p:sldIdLst>{ids}</p:sldIdLst></p:presentation>"
    )
    presentation_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>'
        "</Relationships>"
    )
    slide = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" '
        'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
        "<p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Stasera aragosta</a:t></a:r></a:p>"
        "</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"
    )
    content_types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="xml" ContentType="application/xml"/></Types>'
    )
    return zip_bytes(
        [
            ("[Content_Types].xml", content_types.encode()),
            ("ppt/presentation.xml", presentation.encode()),
            ("ppt/_rels/presentation.xml.rels", presentation_rels.encode()),
            ("ppt/slides/slide1.xml", slide.encode()),
        ]
    )


def huge_dictionary_7z():
    """Un 7z di 7-Zip con la proprietà del dizionario LZMA2 riscritta (38 = 2 GiB) e i CRC
    dell'intestazione ricalcolati."""
    with tempfile.TemporaryDirectory() as tmp:
        (Path(tmp) / "nota.txt").write_bytes(b"aragosta nel dizionario enorme\n")
        out = Path(tmp) / "a.7z"
        subprocess.run(
            [str(SEVENZ), "a", "-t7z", "-m0=lzma2", "-mhc=off", "-mtm=off", "-mtc=off",
             "-mta=off", str(out), "nota.txt"],
            cwd=tmp, check=True, capture_output=True,
        )
        data = bytearray(out.read_bytes())
    next_offset, next_size = struct.unpack_from("<QQ", data, 12)
    start = 32 + next_offset
    header = bytearray(data[start:start + next_size])
    # Coder LZMA2: flag (id di 1 byte, con proprietà), id 0x21, 1 byte di proprietà.
    at = header.find(b"\x21\x21\x01")
    assert at >= 0, "coder LZMA2 non trovato"
    header[at + 3] = 38
    data[start:start + next_size] = header
    struct.pack_into("<I", data, 28, zlib.crc32(header) & 0xFFFFFFFF)
    struct.pack_into("<I", data, 8, zlib.crc32(bytes(data[12:32])) & 0xFFFFFFFF)
    return bytes(data)


def repeated_text_ods():
    """Un ods di meno di 1 KB: una cella di testo di 300 caratteri ripetuta su 16.384 colonne
    e 255 righe. Le celle (circa 4,2 milioni) stanno sotto un tetto contato in celle, ma un
    lettore che clona il testo per ogni ripetizione ne fa oltre 1 GB."""
    mime = "application/vnd.oasis.opendocument.spreadsheet"
    manifest = (
        '<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0">'
        f'<manifest:file-entry manifest:full-path="/" manifest:media-type="{mime}"/>'
        '</manifest:manifest>'
    )
    content = (
        '<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
        'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
        'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">'
        '<office:body><office:spreadsheet><table:table table:name="Ripetuto">'
        '<table:table-row table:number-rows-repeated="255">'
        '<table:table-cell office:value-type="string" table:number-columns-repeated="16384">'
        f'<text:p>{"aragosta " * 33}fin</text:p></table:table-cell></table:table-row>'
        '</table:table></office:spreadsheet></office:body></office:document-content>'
    )
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        for name, data, method in (("mimetype", mime, zipfile.ZIP_STORED),
                                   ("META-INF/manifest.xml", manifest, zipfile.ZIP_DEFLATED),
                                   ("content.xml", content, zipfile.ZIP_DEFLATED)):
            info = zipfile.ZipInfo(name, DATE)
            info.compress_type = method
            info.external_attr = 0o644 << 16
            z.writestr(info, data)
    return buf.getvalue()


def tar_member(name, data, fmt, pax_headers=None):
    """Un membro tar (intestazioni, dati e riempimento a 512 byte) nel formato `fmt`, con
    data, proprietario e permessi fissi."""
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mtime = 0
    info.mode = 0o644
    info.uname = info.gname = ""
    if pax_headers:
        info.pax_headers = pax_headers
    header = info.tobuf(fmt, "utf-8", "surrogateescape")
    padding = b"\0" * (-len(data) % tarfile.BLOCKSIZE)
    return header + data + padding


def long_names_tar_gz():
    """nomi-lunghi.tar.gz: i membri nell'ordine della descrizione in testa, compressi con
    gzip (mtime 0, nessun nome interno)."""
    huge = 128 * 1024
    members = [
        tar_member("g/" + "n" * huge + ".txt", b"aragosta nel nome enorme\n", tarfile.GNU_FORMAT),
        tar_member("pax-enorme.txt", b"aragosta nella voce pax enorme\n", tarfile.PAX_FORMAT,
                   {"comment": "c" * huge}),
        tar_member("lungo/" + "l" * 300 + ".txt", b"aragosta nel nome lungo\n", tarfile.GNU_FORMAT),
        tar_member("pax/" + "p" * 300 + ".txt", b"aragosta nel percorso pax\n", tarfile.PAX_FORMAT),
        tar_member("dopo.txt", b"aragosta dopo\n", tarfile.GNU_FORMAT),
    ]
    tar = b"".join(members) + b"\0" * (2 * tarfile.BLOCKSIZE)
    buf = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=buf, mtime=0) as gz:
        gz.write(tar)
    return buf.getvalue()


def main():
    write("nomi-lunghi.tar.gz", long_names_tar_gz())
    write("testo-ripetuto.ods", repeated_text_ods())
    xlsx = sparse_xlsx()
    write("sparso.xlsx", xlsx)
    write("sparso.zip", zip_bytes([("sparso.xlsx", xlsx), ("nota.txt", "aragosta in nota\n".encode())]))
    write("slide-ripetute.pptx", repeated_slides_pptx())
    if SEVENZ.is_file():
        write("dizionario-enorme.7z", huge_dictionary_7z())
    else:
        print("dizionario-enorme.7z non generato: 7-Zip non trovato")


if __name__ == "__main__":
    main()
