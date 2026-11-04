"""Genera fixtures/demo/ (rieseguibile): python -I fixtures/src/genera_demo.py"""
import os, re, shutil, struct, subprocess, tempfile, unicodedata, zipfile, zlib
from pathlib import Path

SRC = Path(__file__).resolve().parent
FIX = SRC.parent
DEMO = FIX / "demo"
SOFFICE = r"C:\Program Files\LibreOffice\program\soffice.exe"
DATE = (2026, 1, 1, 0, 0, 0)

MARE = """Ricetta del pesce al forno
Ingredienti per quattro persone:
un chilo di pesce fresco, due limoni e olio.
Servono anche sale, pepe e prezzemolo.
Pulire i calamari con cura.
Mettere il pesce in una ciotola.
Accendere il forno a 200 gradi.
Aggiungere olio, aglio e un po' di vino bianco.
Il vino deve essere secco e freddo.
Mescolando tutto con un cucchiaio di legno.
Il profumo riempie subito la cucina.
Cuocere l'aragosta per 10 minuti.
Togliere dal fuoco e attendere un poco.
Servire con pane caldo e insalata.
Buon appetito a tutti!
"""
NFD_CITTA = unicodedata.normalize("NFD", "città")
assert len(NFD_CITTA) == 6

def w(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)

def png():
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    rows = b""
    for y in range(8):
        rows += b"\x00" + b"".join(bytes((x * 32, y * 32, 128)) for x in range(8))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))

def convert(src, fmt, outdir):
    subprocess.run([SOFFICE, "--headless", "--convert-to", fmt, "--outdir", str(outdir), str(src)],
                   check=True, capture_output=True)

CORE = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
        '<dcterms:created xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:created>'
        '<dc:creator></dc:creator><cp:lastModifiedBy></cp:lastModifiedBy>'
        '<dcterms:modified xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:modified>'
        '</cp:coreProperties>')

def clean_docx(path):
    with zipfile.ZipFile(path) as z:
        items = [(i.filename, z.read(i.filename)) for i in z.infolist()]
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in items:
            if name == "docProps/core.xml":
                data = CORE.encode()
            elif name == "docProps/app.xml":
                data = re.sub(rb"<(Application|AppVersion|Company|Manager)>[^<]*</\1>", b"", data)
            zi = zipfile.ZipInfo(name, DATE)
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, data)

def main():
    w(DEMO / "ricette" / "mare.txt", MARE.encode("utf-8"))
    w(DEMO / "viaggi" / "citta.txt",
      ("Firenze è una città d'arte.\nPerché no?\nARMADIO grande\nLa " + NFD_CITTA + " di notte è bella.\n").encode("utf-8"))
    w(DEMO / "legacy" / "vecchio-1252.txt",
      "Fatelo perché è giusto.\nUn atto di carità.\n".encode("cp1252"))
    w(DEMO / "legacy" / "utf16.txt", b"\xff\xfe" + "Una carta d'identità\r\n".encode("utf-16-le"))
    w(DEMO / "binario" / "immagine.png", png())
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)
        convert(SRC / "demo" / "note.fodt", "docx", t)
        convert(SRC / "demo" / "elenco.fodt", "pdf", t)
        (DEMO / "ufficio").mkdir(parents=True, exist_ok=True)
        shutil.copy(t / "note.docx", DEMO / "ufficio" / "note.docx")
        clean_docx(DEMO / "ufficio" / "note.docx")
        (DEMO / "musica").mkdir(parents=True, exist_ok=True)
        zi = zipfile.ZipInfo("elenco.pdf", DATE)
        zi.compress_type = zipfile.ZIP_DEFLATED
        with zipfile.ZipFile(DEMO / "musica" / "archivio.zip", "w") as z:
            z.writestr(zi, (t / "elenco.pdf").read_bytes())

if __name__ == "__main__":
    main()
