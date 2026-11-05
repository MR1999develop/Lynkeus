"""Genera fixtures/formati/ e fixtures/malevoli/ (rieseguibile):
    python -I fixtures/src/genera_formati.py [--vuote N]
I sorgenti dei documenti (.fodt/.fods/.fodp) sono scritti in fixtures/src/formati/ e poi
convertiti con LibreOffice headless. Con --vuote N si genera anche malevoli/molte-voci.zip
con N voci vuote (default: non generato, perche' con 100.001 voci supera i 5 MB)."""
import gzip, io, re, shutil, subprocess, sys, tarfile, tempfile, zipfile
from email.message import EmailMessage
from email.utils import format_datetime
from datetime import datetime, timezone
from pathlib import Path
from xml.sax.saxutils import escape

SRC = Path(__file__).resolve().parent
FIX = SRC.parent
FORM = FIX / "formati"
MAL = FIX / "malevoli"
SRCF = SRC / "formati"
SOFFICE = r"C:\Program Files\LibreOffice\program\soffice.exe"
SEVENZ = r"C:\Program Files\7-Zip\7z.exe"
DATE = (2026, 1, 1, 0, 0, 0)
NS = ('xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
      'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
      'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" '
      'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
      'xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" '
      'xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" '
      'xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" '
      'xmlns:presentation="urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" '
      'office:version="1.3"')
MIME = "application/vnd.oasis.opendocument."
CORE = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
        '<dcterms:created xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:created>'
        '<dc:creator></dc:creator><cp:lastModifiedBy></cp:lastModifiedBy>'
        '<dcterms:modified xsi:type="dcterms:W3CDTF">2026-01-01T00:00:00Z</dcterms:modified>'
        '</cp:coreProperties>')


def w(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))


def zi(name, comp=zipfile.ZIP_DEFLATED):
    z = zipfile.ZipInfo(name, DATE)
    z.compress_type = comp
    z.external_attr = 0o644 << 16
    return z


def convert(src, fmt, outdir, ext=None):
    r = subprocess.run([SOFFICE, "--headless", "--convert-to", fmt, "--outdir", str(outdir), str(src)],
                       capture_output=True, text=True)
    out = Path(outdir) / (Path(src).stem + "." + (ext or fmt.split(":")[0]))
    if r.returncode != 0 or not out.exists():
        raise RuntimeError(f"conversione fallita {src.name} -> {fmt}: {r.stdout} {r.stderr}")
    return out


def clean_ooxml(path):
    with zipfile.ZipFile(path) as z:
        items = [(i.filename, z.read(i.filename)) for i in z.infolist()]
    with zipfile.ZipFile(path, "w") as z:
        for name, data in items:
            if name == "docProps/core.xml":
                data = CORE.encode()
            elif name == "docProps/app.xml":
                data = re.sub(rb"<(Application|AppVersion|Company|Manager)>[^<]*</\1>", b"", data)
            z.writestr(zi(name), data)


def clean_odf(path):
    with zipfile.ZipFile(path) as z:
        items = [(i.filename, z.read(i.filename), i.compress_type) for i in z.infolist()]
    with zipfile.ZipFile(path, "w") as z:
        for name, data, ct in items:
            if name == "meta.xml":
                data = re.sub(rb"<meta:(initial-creator|creation-date|generator|editing-duration|editing-cycles)>[^<]*</meta:\1>", b"", data)
                data = re.sub(rb"<dc:(creator|date)>[^<]*</dc:\1>", b"", data)
            z.writestr(zi(name, ct), data)


# ---- sorgenti -------------------------------------------------------------
def fodt(paragraphs, breaks=()):
    """paragraphs: lista di testi; breaks: indici (da 0) dei paragrafi che iniziano una nuova pagina."""
    body = ""
    for i, p in enumerate(paragraphs):
        st = ' text:style-name="PB"' if i in breaks else ""
        body += f"   <text:p{st}>{escape(p)}</text:p>\n"
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n<office:document {NS} office:mimetype="{MIME}text">\n'
            ' <office:meta/>\n <office:automatic-styles>\n'
            '  <style:style style:name="PB" style:family="paragraph"><style:paragraph-properties fo:break-before="page"/></style:style>\n'
            ' </office:automatic-styles>\n <office:body>\n  <office:text>\n' + body +
            '  </office:text>\n </office:body>\n</office:document>\n')


def fods(sheet, rows):
    out = ""
    for r in rows:
        out += "    <table:table-row>"
        for c in r:
            if isinstance(c, (int, float)):
                out += f'<table:table-cell office:value-type="float" office:value="{c}"><text:p>{c}</text:p></table:table-cell>'
            else:
                out += f'<table:table-cell office:value-type="string"><text:p>{escape(c)}</text:p></table:table-cell>'
        out += "</table:table-row>\n"
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n<office:document {NS} office:mimetype="{MIME}spreadsheet">\n'
            f' <office:meta/>\n <office:body>\n  <office:spreadsheet>\n   <table:table table:name="{sheet}">\n'
            '    <table:table-column table:number-columns-repeated="3"/>\n' + out +
            '   </table:table>\n  </office:spreadsheet>\n </office:body>\n</office:document>\n')


def fodp(slides):
    pages = ""
    for i, (titolo, testo) in enumerate(slides, 1):
        pages += (f'   <draw:page draw:name="Diapositiva{i}" draw:master-page-name="Default">\n'
                  '    <draw:frame svg:x="2cm" svg:y="1cm" svg:width="20cm" svg:height="2cm"><draw:text-box>'
                  f'<text:p>{escape(titolo)}</text:p></draw:text-box></draw:frame>\n'
                  '    <draw:frame svg:x="2cm" svg:y="4cm" svg:width="20cm" svg:height="6cm"><draw:text-box>'
                  f'<text:p>{escape(testo)}</text:p></draw:text-box></draw:frame>\n   </draw:page>\n')
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n<office:document {NS} office:mimetype="{MIME}presentation">\n'
            ' <office:meta/>\n <office:automatic-styles>\n'
            '  <style:page-layout style:name="PL"><style:page-layout-properties fo:page-width="28cm" fo:page-height="15.75cm"/></style:page-layout>\n'
            ' </office:automatic-styles>\n <office:master-styles>\n'
            '  <style:master-page style:name="Default" style:page-layout-name="PL"/>\n </office:master-styles>\n'
            ' <office:body>\n  <office:presentation>\n' + pages + '  </office:presentation>\n </office:body>\n</office:document>\n')


SPESA = [["Voce", "Importo", "Nota"], ["Pane", 2, "città"], ["Latte", 1], ["Uova", 3],
         ["Mele", 4], ["Riso", 2], ["Totale", "aragosta", "fine"]]

# Registro lungo: 501 righe per 3 colonne, tutte piene (flusso BIFF ben oltre 4 KB, oltre la
# soglia dei settori piccoli del file composto). `città` in C250, `aragosta` in C501.
REGISTRO = [["Voce", "Importo", "Nota"]] + [
    [f"Voce {n}", n, "città" if n == 250 else ("aragosta" if n == 501 else f"nota {n}")]
    for n in range(2, 502)]

TESTI = {
    # docx: paragrafo 4
    "relazione": fodt(["Relazione della settimana", "Siamo andati in città per il mercato.",
                       "Abbiamo comprato frutta e verdura.", "Per cena c'è l'aragosta con il limone.",
                       "Poi siamo tornati a casa."]),
    # odt: paragrafo 2
    "appunti": fodt(["Appunti di cucina", "Prima di tutto l'aragosta va lavata bene.",
                     "Perché la città è lontana, compriamo tutto qui.", "Fine degli appunti."]),
    # doc: paragrafo 3
    "vecchia-lettera": fodt(["Gentile cliente,", "la ringraziamo per la visita in città.",
                             "Le consigliamo l'aragosta del giorno.", "Cordiali saluti."]),
    # pdf: pagina 2, riga 3 (ogni paragrafo e' una riga)
    "catalogo": fodt(["Catalogo dei prodotti", "Pagina uno: pane e latte.", "Fine della prima pagina.",
                      "Pagina due: elenco del mare.", "Il pesce arriva dalla città vicina.",
                      "Oggi abbiamo l'aragosta fresca.", "Fine del catalogo."], breaks={3}),
    # allegato dell'e-mail (docx): paragrafo 2
    "allegato": fodt(["Ordine del ristorante", "Due porzioni di aragosta per la città di sera."]),
    # docx dentro lo zip annidato: paragrafo 2
    "interno": fodt(["Documento interno", "L'aragosta più grande della città."]),
    # docx protetto: paragrafo 1
    "protetto": fodt(["Documento riservato con aragosta e città."]),
    # xlsx/xls/ods: foglio, cella
    "spesa": fods("Spesa", SPESA),
    "vecchia-spesa": fods("Spesa", SPESA),
    "registro-lungo": fods("Registro", REGISTRO),
    "bilancio": fods("Conti", [["Mese", "Entrate", "Note"], ["Gennaio", 100, "ok"], ["Febbraio", 120, "città"],
                               ["Marzo", 90, "aragosta"]]),
    # pptx: diapositiva 2; ppt: diapositiva 3; odp: diapositiva 2
    "presentazione": fodp([("Benvenuti", "Una presentazione di prova."), ("Il menu", "Stasera aragosta e vino."),
                           ("La città", "Grazie per l'attenzione.")]),
    "vecchia-presentazione": fodp([("Titolo", "Prima diapositiva."), ("La città", "Seconda diapositiva."),
                                   ("Il menu", "Terza diapositiva con aragosta.")]),
    "lezione": fodp([("Lezione uno", "Introduzione alla città."), ("Cucina", "Cuocere l'aragosta in acqua salata."),
                     ("Fine", "Domande?")]),
}


def zbytes(entries):
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w") as z:
        for n, d in entries:
            z.writestr(zi(n), d)
    return b.getvalue()


def main():
    vuote = 0
    if "--vuote" in sys.argv:
        vuote = int(sys.argv[sys.argv.index("--vuote") + 1])
    SRCF.mkdir(parents=True, exist_ok=True)
    for n, t in TESTI.items():
        kind = re.search(r'mimetype="[^"]*\.(\w+)"', t).group(1)
        w(SRCF / (n + {"text": ".fodt", "spreadsheet": ".fods", "presentation": ".fodp"}[kind]), t)

    def S(n):
        for e in (".fodt", ".fods", ".fodp"):
            if (SRCF / (n + e)).exists():
                return SRCF / (n + e)

    FORM.mkdir(parents=True, exist_ok=True)
    MAL.mkdir(parents=True, exist_ok=True)
    avvisi = []
    with tempfile.TemporaryDirectory() as t:
        t = Path(t)

        def conv(n, fmt, ext, clean=None):
            o = convert(S(n), fmt, t, ext)
            if clean:
                clean(o)
            shutil.copy(o, FORM / o.name)
            return FORM / o.name

        docx = conv("relazione", "docx", "docx", clean_ooxml)
        conv("appunti", "odt", "odt", clean_odf)
        conv("vecchia-lettera", "doc", "doc")
        conv("catalogo", "pdf", "pdf")
        conv("spesa", "xlsx", "xlsx", clean_ooxml)
        conv("bilancio", "ods", "ods", clean_odf)
        conv("vecchia-spesa", "xls", "xls")
        conv("registro-lungo", "xls", "xls")
        conv("presentazione", "pptx", "pptx", clean_ooxml)
        conv("lezione", "odp", "odp", clean_odf)
        conv("vecchia-presentazione", "ppt", "ppt")
        allegato = convert(S("allegato"), "docx", t)
        clean_ooxml(allegato)
        interno = convert(S("interno"), "docx", t)
        clean_ooxml(interno)
        pdf = (FORM / "catalogo.pdf").read_bytes()
        allegato = allegato.read_bytes()
        interno = interno.read_bytes()

        # docx protetto (password "segreto"): cifrato via UNO con il python di LibreOffice
        try:
            lo_py = str(Path(SOFFICE).with_name("python.exe"))
            out = FORM / "protetto-password.docx"
            subprocess.run([lo_py, str(SRC / "cifra_docx.py"), str(S("protetto")), str(out), "segreto"],
                           check=True, capture_output=True, timeout=180)
            if zipfile.is_zipfile(out):
                avvisi.append("docx protetto non cifrato")
        except Exception as e:
            avvisi.append(f"docx protetto non generato: {e}")

    # RTF
    rtf = (b"{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0 Arial;}}\\pard Prima riga di prova.\\par\r\n"
           b"La citt\\'e0 \\u232? bella, perch\\u233? c'\\u232? il mare.\\par\r\n"
           b"Stasera si mangia l'aragosta.\\par\r\n}")
    w(FORM / "lettera.rtf", rtf)
    w(FORM / "in-realta-rtf.doc", rtf.replace(b"Prima riga", b"Un .doc che e' RTF: prima riga"))

    # EML
    m = EmailMessage()
    m["From"] = "mittente@example.com"
    m["To"] = "destinatario@example.com"
    m["Subject"] = "Ordine di aragosta"
    m["Date"] = format_datetime(datetime(2026, 1, 1, tzinfo=timezone.utc))
    m["Message-ID"] = "<fixture-1@example.com>"
    m.set_content("Buongiorno,\nvi scrivo dalla città: nel corpo c'è l'aragosta.\n"
                  "Il menu è nell'allegato.\nPerché non passate a trovarci?\n", charset="utf-8")
    m.add_attachment(allegato, maintype="application",
                     subtype="vnd.openxmlformats-officedocument.wordprocessingml.document", filename="allegato.docx")
    m.add_attachment("Nota allegata: l'aragosta arriva domani.\nLa città aspetta.\n".encode("utf-8"),
                     maintype="text", subtype="plain", filename="nota.txt")
    w(FORM / "ordine.eml", m.as_bytes())
    m2 = EmailMessage()
    m2["From"] = "mittente@example.com"
    m2["To"] = "destinatario@example.com"
    m2["Subject"] = "Saluti"
    m2["Date"] = m["Date"]
    m2["Message-ID"] = "<fixture-2@example.com>"
    m2.set_content("Ciao,\nnel corpo del messaggio c'è l'aragosta, nella città di sera.\n", charset="utf-8")
    w(FORM / "saluti.eml", m2.as_bytes())

    # EPUB
    cont = ('<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
            '<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
    opf = ('<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
           '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">fixture-epub-1</dc:identifier>'
           '<dc:title>Libro di prova</dc:title><dc:language>it</dc:language>'
           '<meta property="dcterms:modified">2026-01-01T00:00:00Z</meta></metadata>'
           '<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
           '<item id="c1" href="cap1.xhtml" media-type="application/xhtml+xml"/>'
           '<item id="c2" href="cap2.xhtml" media-type="application/xhtml+xml"/></manifest>'
           '<spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>')

    def xh(title, body):
        return ('<?xml version="1.0" encoding="UTF-8"?><html xmlns="http://www.w3.org/1999/xhtml" '
                'xmlns:epub="http://www.idpf.org/2007/ops" lang="it">'
                f'<head><title>{title}</title></head><body>{body}</body></html>')

    nav = xh("Indice", '<nav epub:type="toc"><ol><li><a href="cap1.xhtml">Capitolo uno</a></li>'
                       '<li><a href="cap2.xhtml">Capitolo due</a></li></ol></nav>')
    with zipfile.ZipFile(FORM / "libro.epub", "w") as z:
        z.writestr(zi("mimetype", zipfile.ZIP_STORED), "application/epub+zip")
        z.writestr(zi("META-INF/container.xml"), cont)
        z.writestr(zi("OEBPS/content.opf"), opf)
        z.writestr(zi("OEBPS/nav.xhtml"), nav)
        z.writestr(zi("OEBPS/cap1.xhtml"), xh("Capitolo uno", "<h1>Capitolo uno</h1><p>Il viaggio comincia in città.</p><p>Perché partire presto?</p>"))
        z.writestr(zi("OEBPS/cap2.xhtml"), xh("Capitolo due", "<h1>Capitolo due</h1><p>Al porto si vende l'aragosta.</p>"))

    # TXT e archivi semplici
    w(FORM / "spesa.txt", "Elenco della spesa\nPane\nLatte\nUova\nMele\nOggi compriamo l'aragosta in città.\nRiso\n")
    inner = {"cartella/ricetta.txt": "Ricetta\nBollire l'aragosta in città.\n", "cartella/nota.txt": "Niente di speciale.\n"}

    def add_tar(tf):
        for n, s in inner.items():
            b = s.encode("utf-8")
            ti = tarfile.TarInfo(n)
            ti.size = len(b)
            ti.mtime = 1767225600
            ti.mode = 0o644
            tf.addfile(ti, io.BytesIO(b))

    with tarfile.open(FORM / "archivio.tar", "w", format=tarfile.PAX_FORMAT) as tf:
        add_tar(tf)
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.PAX_FORMAT) as tf:
        add_tar(tf)
    with open(FORM / "archivio.tar.gz", "wb") as f, gzip.GzipFile("", "wb", 9, f, mtime=0) as g:
        g.write(buf.getvalue())
    with open(FORM / "appunti.txt.gz", "wb") as f, gzip.GzipFile("", "wb", 9, f, mtime=0) as g:
        g.write("Prima riga\nSeconda riga con aragosta e città.\n".encode("utf-8"))

    # 7z (Python non ha un modulo standard: si usa 7-Zip)
    if Path(SEVENZ).exists():
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            (t / "cartella").mkdir()
            for n, s in inner.items():
                (t / n).write_bytes(s.encode("utf-8"))
            out = FORM / "archivio.7z"
            out.unlink(missing_ok=True)
            subprocess.run([SEVENZ, "a", "-t7z", "-mtm=off", "-mtc=off", "-mta=off", str(out), "cartella"],
                           cwd=t, check=True, capture_output=True)
    else:
        avvisi.append("7z non generato: 7-Zip non trovato")

    # zip annidato a 3 livelli: annidato.zip > livello2.zip > livello3.zip > interno.docx
    l3 = zbytes([("interno.docx", interno)])
    l2 = zbytes([("livello3.zip", l3)])
    w(FORM / "annidato.zip", zbytes([("livello2.zip", l2)]))

    # ---- malevoli ----
    w(MAL / "bomba.zip", zbytes([("zeri.txt", b"\0" * (5 * 1024 * 1024))]))
    b = io.BytesIO()
    with zipfile.ZipFile(b, "w") as z:
        for n in ("../../evil.txt", "/assoluto/evil.txt", "C:/Windows/evil.txt", "ok/normale.txt"):
            z.writestr(zi(n), "File con aragosta nella voce " + n + "\n")
    w(MAL / "percorsi.zip", b.getvalue())
    cur = zbytes([("fondo.txt", "Testo in fondo con aragosta.\n")])
    for i in range(6, 0, -1):
        cur = zbytes([(f"livello{i + 1}.zip", cur)])
    w(MAL / "annidato-profondo.zip", cur)
    if vuote:
        with zipfile.ZipFile(MAL / "molte-voci.zip", "w") as z:
            for i in range(vuote):
                z.writestr(zi(f"e{i}"), b"")
    w(MAL / "troncato.pdf", pdf[: len(pdf) // 2])
    with zipfile.ZipFile(docx) as z:
        items = [(i.filename, z.read(i.filename)) for i in z.infolist()]
    dtd = ('<!DOCTYPE lolz [<!ENTITY lol "aragosta "><!ENTITY lol1 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">'
           '<!ENTITY lol2 "&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;">'
           '<!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">]>').encode()
    with zipfile.ZipFile(MAL / "entita.docx", "w") as z:
        for n, d in items:
            if n == "word/document.xml":
                d = re.sub(rb"(<w:t(?: [^>]*)?>)[^<]*(</w:t>)", rb"\1&lol3;\2", d, count=1)
                d = d.replace(b"?>", b"?>\n" + dtd, 1)
            z.writestr(zi(n), d)
    w(MAL / "immagine-finta.pdf", (FIX / "demo" / "binario" / "immagine.png").read_bytes())
    w(MAL / "riga-lunga.txt", ("abcdefghij" * 200000) + "\naragosta\n")
    for a in avvisi:
        print("AVVISO:", a)


if __name__ == "__main__":
    main()
