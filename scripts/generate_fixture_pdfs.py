#!/usr/bin/env python3
"""Generate original, deterministic PDFs without external packages."""
from pathlib import Path


def pdf(pages):
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    kids = []
    for lines in pages:
        page_id = len(objects) + 1
        stream_id = page_id + 1
        kids.append(f"{page_id} 0 R")
        objects.append(
            (
                f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                f"/Resources << /Font << /F1 3 0 R >> >> "
                f"/Contents {stream_id} 0 R >>"
            ).encode("ascii")
        )
        stream = b"BT /F1 11 Tf 48 740 Td 16 TL\n"
        for line in lines:
            escaped = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
            stream += f"({escaped}) Tj T*\n".encode("ascii")
        stream += b"ET\n"
        objects.append(
            f"<< /Length {len(stream)} >>\nstream\n".encode("ascii")
            + stream
            + b"endstream"
        )
    objects[1] = (
        f"<< /Type /Pages /Kids [{' '.join(kids)}] /Count {len(kids)} >>"
    ).encode("ascii")
    output = b"%PDF-1.4\n"
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(output))
        output += f"{number} 0 obj\n".encode("ascii") + obj + b"\nendobj\n"
    xref = len(output)
    output += f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode("ascii")
    for offset in offsets[1:]:
        output += f"{offset:010d} 00000 n \n".encode("ascii")
    output += (
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\n"
        f"startxref\n{xref}\n%%EOF\n"
    ).encode("ascii")
    return output


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    for version in (1, 2):
        target = root / "corpus" / "pdf" / f"v{version}" / "guidelines.pdf"
        target.parent.mkdir(parents=True, exist_ok=True)
        lines = [
            f"SYNTHETIC V{version} IMPLEMENTATION GUIDELINES",
            "Not an official Berlin Group specification. Test evidence only.",
            "1. TRANSACTIONS",
            "GET /accounts/{accountId}/transactions",
            "The synthetic transaction endpoint returns account transactions.",
        ]
        if version == 2:
            lines += [
                "Each synthetic transaction MUST include transactionId and status.",
                "The synthetic bookingStatus accepts booked, pending, or both.",
            ]
        else:
            lines += ["Each synthetic transaction MUST include transactionId."]
        target.write_bytes(pdf([lines, ["2. TEST SCOPE", "These fixtures define no real Open Banking obligations."]]))
    target = root / "crates" / "bg-spec-indexer" / "tests" / "fixtures" / "blank.pdf"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(pdf([[]]))
