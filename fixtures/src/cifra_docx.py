"""Cifra un documento con password usando LibreOffice via UNO (python di LibreOffice).
Uso: python.exe cifra_docx.py <sorgente> <uscita.docx> <password>"""
import subprocess, sys, time, os
import uno
from com.sun.star.beans import PropertyValue, NamedValue

def pv(n, v):
    p = PropertyValue(); p.Name = n; p.Value = v; return p

src, dst, pwd = sys.argv[1:4]
soffice = r"C:\Program Files\LibreOffice\program\soffice.exe"
proc = subprocess.Popen([soffice, "--headless", "--norestore", "--accept=socket,host=127.0.0.1,port=2002;urp;"])
ctx = None
local = uno.getComponentContext()
res = local.ServiceManager.createInstanceWithContext("com.sun.star.bridge.UnoUrlResolver", local)
for _ in range(60):
    try:
        ctx = res.resolve("uno:socket,host=127.0.0.1,port=2002;urp;StarOffice.ComponentContext"); break
    except Exception:
        time.sleep(1)
desktop = ctx.ServiceManager.createInstanceWithContext("com.sun.star.frame.Desktop", ctx)
doc = desktop.loadComponentFromURL(uno.systemPathToFileUrl(src), "_blank", 0, (pv("Hidden", True),))
try:
    doc.storeToURL(uno.systemPathToFileUrl(dst), (pv("FilterName", "MS Word 2007 XML"), pv("Password", pwd)))
finally:
    doc.close(True)
try: desktop.terminate()
except Exception: pass
proc.wait(timeout=30)
