"""Genera fixtures/formati/protetto-password.pdf (rieseguibile):
    python -I fixtures/src/genera_pdf_protetto.py
Esporta fixtures/src/formati/catalogo.fodt con LibreOffice headless, con password d'apertura
`segreto` (filtro writer_pdf_Export, opzioni EncryptFile e DocumentOpenPassword). Le date interne
e l'identificativo del PDF cambiano a ogni generazione; il contenuto cifrato e' quello di
catalogo.pdf. Atteso dall'estrattore: avviso "protetto da password", nessun testo."""
import json, shutil, subprocess, tempfile
from pathlib import Path

SRC = Path(__file__).resolve().parent
OUT = SRC.parent / "formati" / "protetto-password.pdf"
SOFFICE = r"C:\Program Files\LibreOffice\program\soffice.exe"
OPTIONS = {
    "EncryptFile": {"type": "boolean", "value": "true"},
    "DocumentOpenPassword": {"type": "string", "value": "segreto"},
}


def main():
    fmt = "pdf:writer_pdf_Export:" + json.dumps(OPTIONS, separators=(",", ":"))
    with tempfile.TemporaryDirectory() as t:
        r = subprocess.run([SOFFICE, "--headless", "--convert-to", fmt, "--outdir", t,
                            str(SRC / "formati" / "catalogo.fodt")], capture_output=True, text=True)
        out = Path(t) / "catalogo.pdf"
        if r.returncode != 0 or not out.exists():
            raise RuntimeError(f"conversione fallita: {r.stdout} {r.stderr}")
        if b"/Encrypt" not in out.read_bytes():
            raise RuntimeError("il PDF prodotto non e' cifrato")
        shutil.copyfile(out, OUT)
    print(OUT)


if __name__ == "__main__":
    main()
