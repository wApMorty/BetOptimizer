#!/usr/bin/env python3
"""Fabrique un tableur de test minuscule au format de tennis-data.co.uk (joueurs fictifs).

Aucune donnée de tennis-data n'est versionnée : leurs fichiers sont protégés par le droit d'auteur. Ce script écrit
`mini_atp_2024.xlsx` (sans bibliothèque externe : un .xlsx est un zip de petits fichiers XML).
Usage : python3 make_fixture.py
"""
import zipfile
from xml.sax.saxutils import escape

HEADER = ["ATP", "Location", "Tournament", "Date", "Series", "Court", "Surface", "Round", "Best of", "Winner", "Loser",
          "WRank", "LRank", "WPts", "LPts", "W1", "L1", "W2", "L2", "W3", "L3", "Wsets", "Lsets", "Comment",
          "B365W", "B365L", "PSW", "PSL", "MaxW", "MaxL", "AvgW", "AvgL"]

# Les dates sont des numéros de série Excel (jours depuis le 30/12/1899) : 45292 = 2024-01-01, donc 45298 = 2024-01-07.
D = 45298
ROWS = [
    # 1. Match normal sur dur, avec toutes les cotes.
    [1, "Faketown", "Fake Open", ("date", D), "ATP250", "Outdoor", "Hard", "1st Round", 3, "Alpha A.", "Bravo B.",
     10, 50, 3000, 900, 6, 4, 6, 3, None, None, 2, 0, "Completed", 1.50, 2.62, 1.55, 2.60, 1.58, 2.70, 1.54, 2.55],
    # 2. Terre battue, pas de cote Pinnacle (cellules vides), cote Bet365 seule.
    [1, "Faketown", "Fake Open", ("date", D + 1), "ATP250", "Outdoor", "Clay", "2nd Round", 3, "Charlie C.", "Delta D.",
     20, 30, 2000, 1500, 7, 5, 6, 4, None, None, 2, 0, "Completed", 1.80, 2.00, None, None, None, None, None, None],
    # 3. Forfait : aucune manche jouée.
    [1, "Faketown", "Fake Open", ("date", D + 2), "ATP250", "Outdoor", "Hard", "Quarter-finals", 3, "Alpha A.", "Echo E.",
     10, 70, 3000, 700, None, None, None, None, None, None, None, None, "Walkover", 1.30, 3.40, 1.31, 3.45, None, None, None, None],
    # 4. Abandon pendant le match.
    [1, "Faketown", "Fake Open", ("date", D + 3), "ATP250", "Outdoor", "Hard", "Semi-finals", 3, "Bravo B.", "Charlie C.",
     50, 20, 900, 2000, 6, 3, 2, 1, None, None, 1, 0, "Retired", 2.10, 1.75, 2.15, 1.80, None, None, None, None],
    # 5. Surface inconnue : rejetée.
    [1, "Faketown", "Fake Open", ("date", D + 4), "ATP250", "Outdoor", "Wood", "Final", 3, "Alpha A.", "Charlie C.",
     10, 20, 3000, 2000, 6, 1, 6, 1, None, None, 2, 0, "Completed", 1.4, 3.0, 1.4, 3.0, None, None, None, None],
    # 6. Cote Pinnacle invalide (1,00) : la cote est écartée, le match est gardé.
    [1, "Faketown", "Fake Open", ("date", D + 5), "ATP250", "Outdoor", "Hard", "Final", 3, "Delta D.", "Echo E.",
     30, 70, 1500, 700, 6, 4, 6, 4, None, None, 2, 0, "Completed", 1.60, 2.40, 1.00, 2.40, None, None, None, None],
    # 7. Vainqueur manquant : rejeté.
    [1, "Faketown", "Fake Open", ("date", D + 6), "ATP250", "Outdoor", "Hard", "Final", 3, None, "Echo E.",
     30, 70, 1500, 700, 6, 4, 6, 4, None, None, 2, 0, "Completed", None, None, None, None, None, None, None, None],
    # 8. Date écrite en texte au lieu d'une date Excel.
    [2, "Faketown", "Grand Fake", "2024-01-14", "Grand Slam", "Outdoor", "Grass", "1st Round", 5, "Alpha A.", "Delta D.",
     10, 30, 3000, 1500, 6, 4, 6, 4, 6, 4, 3, 0, "Completed", 1.20, 4.50, 1.22, 4.40, 1.25, 4.60, 1.21, 4.30],
]


def cell(ref, value, style=None):
    s = f' s="{style}"' if style else ""
    if value is None:
        return ""
    if isinstance(value, tuple):  # ("date", numéro de série)
        return f'<c r="{ref}" s="1"><v>{value[1]}</v></c>'
    if isinstance(value, (int, float)):
        return f'<c r="{ref}"{s}><v>{value}</v></c>'
    return f'<c r="{ref}" t="inlineStr"><is><t>{escape(value)}</t></is></c>'


def column(index):
    name = ""
    index += 1
    while index:
        index, rest = divmod(index - 1, 26)
        name = chr(65 + rest) + name
    return name


def sheet():
    lines = []
    for r, row in enumerate([HEADER] + ROWS, start=1):
        cells = "".join(cell(f"{column(c)}{r}", v) for c, v in enumerate(row))
        lines.append(f'<row r="{r}">{cells}</row>')
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'
            + "".join(lines) + "</sheetData></worksheet>")


FILES = {
    "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>',
    "_rels/.rels": '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    "xl/workbook.xml": '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="2024" sheetId="1" r:id="rId1"/></sheets></workbook>',
    "xl/_rels/workbook.xml.rels": '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>',
    # Style 1 = format de date 14 (jj/mm/aaaa) : sans lui, une date Excel se lit comme un nombre.
    "xl/styles.xml": '<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>',
    "xl/worksheets/sheet1.xml": sheet(),
}

if __name__ == "__main__":
    with zipfile.ZipFile("mini_atp_2024.xlsx", "w", zipfile.ZIP_DEFLATED) as z:
        for name, content in FILES.items():
            z.writestr(name, content)
    print("mini_atp_2024.xlsx écrit")
