# pdf_edit helper — runs under PyMuPDF (python3 with pymupdf, or `uv run --with pymupdf`).
# Reads a JSON args file (argv[1]) and prints one JSON object on stdout:
#   {"ok": true, "text": "...", "image": {"mime": "...", "b64": "..."}?}  or  {"ok": false, "error": "..."}
# Actions: info | find | replace | add_text | render
import base64
import io
import json
import os
import sys

try:
    import pymupdf as fitz  # PyMuPDF >= 1.24
except ImportError:  # pragma: no cover
    import fitz  # type: ignore

STYLE_FONTS = {
    # (mono, serif) -> [regular, bold, italic, bold-italic]
    (True, False): ["cour", "cobo", "coit", "cobi"],
    (True, True): ["cour", "cobo", "coit", "cobi"],
    (False, True): ["tiro", "tibo", "tiit", "tibi"],
    (False, False): ["helv", "hebo", "heit", "hebi"],
}


def fail(msg):
    print(json.dumps({"ok": False, "error": str(msg)}))
    sys.exit(0)


def parse_color(c, default=(0, 0, 0)):
    """'#rrggbb', 'rrggbb', [r,g,b] (0-1 or 0-255) or an int 0xRRGGBB -> (r,g,b) floats 0-1."""
    if c is None:
        return default
    if isinstance(c, int):
        return ((c >> 16 & 255) / 255, (c >> 8 & 255) / 255, (c & 255) / 255)
    if isinstance(c, str):
        s = c.strip().lstrip("#")
        if len(s) == 6:
            return tuple(int(s[i:i + 2], 16) / 255 for i in (0, 2, 4))
        raise ValueError(f"bad color {c!r} (use #rrggbb)")
    if isinstance(c, (list, tuple)) and len(c) == 3:
        vals = [float(v) for v in c]
        if max(vals) > 1:
            vals = [v / 255 for v in vals]
        return tuple(vals)
    raise ValueError(f"bad color {c!r}")


def font_for(flags=0, mono=None, serif=None, bold=None, italic=None):
    """Pick a base-14 font name from span flags / explicit style hints."""
    if mono is None:
        mono = bool(flags & 8)
    if serif is None:
        serif = bool(flags & 4)
    if bold is None:
        bold = bool(flags & 16)
    if italic is None:
        italic = bool(flags & 2)
    fam = STYLE_FONTS[(mono, serif)]
    return fam[(2 if italic else 0) + (1 if bold else 0)]


def font_with_glyphs(name, text):
    """Base-14 font `name`, or a wider-coverage fallback if it lacks glyphs for `text`."""
    notes = []
    f = fitz.Font(name)
    missing = [ch for ch in set(text) if not ch.isspace() and not f.has_glyph(ord(ch))]
    if not missing:
        return f, notes
    for alt in ("notos", "notosbo", "figo", "cjk"):
        try:
            g = fitz.Font(alt)
        except Exception:
            continue
        if all(g.has_glyph(ord(ch)) for ch in missing):
            notes.append(f"font {name} lacks {''.join(sorted(missing))!r}; used {alt}")
            return g, notes
    notes.append(f"font {name} lacks glyphs for {''.join(sorted(missing))!r} — they may render as boxes (pip install pymupdf-fonts for wider coverage)")
    return f, notes


def page_indices(doc, page_arg):
    """1-based page (int) or None = all -> list of 0-based indices."""
    if page_arg is None:
        return list(range(doc.page_count))
    p = int(page_arg)
    if p < 1 or p > doc.page_count:
        raise ValueError(f"page {p} out of range (document has {doc.page_count} page(s))")
    return [p - 1]


def spans_of(page):
    d = page.get_text("dict")
    for b in d["blocks"]:
        if b.get("type") != 0:
            continue
        for l in b["lines"]:
            for s in l["spans"]:
                yield s, l


def best_span(page, rect):
    """The text span overlapping `rect` the most (and its line)."""
    best, best_line, best_area = None, None, 0.0
    for s, l in spans_of(page):
        r = fitz.Rect(s["bbox"]) & rect
        a = 0.0 if r.is_empty else r.get_area()
        if a > best_area:
            best, best_line, best_area = s, l, a
    return best, best_line


def free_to_right(page, rect, needed):
    """Can text of width `needed` starting at rect.x0 extend past rect without hitting other text or the margin?"""
    ext = fitz.Rect(rect.x1, rect.y0, rect.x0 + needed, rect.y1)
    if ext.x1 > page.rect.x1 - 12:
        return False
    for s, _ in spans_of(page):
        if not (fitz.Rect(s["bbox"]) & ext).is_empty and s["text"].strip():
            return False
    return True


def line_text(line):
    return "".join(s["text"] for s in line["spans"])


def find_hits(doc, text, page_arg, ignore_case=False):
    hits = []
    flags = fitz.TEXT_DEHYPHENATE
    for pi in page_indices(doc, page_arg):
        page = doc[pi]
        rects = page.search_for(text, flags=flags) if not ignore_case else page.search_for(text)  # search_for is case-insensitive by default
        for r in rects:
            span, line = best_span(page, r)
            hits.append({"page": pi + 1, "rect": r, "span": span, "line": line})
    return hits


def hit_desc(h):
    s = h["span"]
    r = h["rect"]
    ctx = line_text(h["line"]).strip() if h["line"] else ""
    if s:
        return (f"page {h['page']}  x={r.x0:.1f} y={r.y0:.1f} w={r.width:.1f} h={r.height:.1f}  "
                f"font={s['font']} {s['size']:.1f}pt color=#{s['color']:06x}  line: {ctx!r}")
    return f"page {h['page']}  x={r.x0:.1f} y={r.y0:.1f} w={r.width:.1f} h={r.height:.1f}"


def save(doc, in_path, args, notes):
    out = args.get("output")
    if out:
        out = os.path.abspath(out)
        if os.path.abspath(in_path) == out:
            out = None
    if out:
        doc.save(out, garbage=4, deflate=True)
        return out
    # in place: keep a backup unless disabled, then rewrite (incremental keeps original bytes when possible)
    if args.get("backup", True):
        base, ext = os.path.splitext(in_path)
        bak = base + ".bak" + ext
        if not os.path.exists(bak):
            with open(in_path, "rb") as f, open(bak, "wb") as g:
                g.write(f.read())
            notes.append(f"backup: {bak}")
    try:
        doc.save(in_path, incremental=True, encryption=fitz.PDF_ENCRYPT_KEEP)
    except Exception:
        tmp = in_path + ".tmp"
        doc.save(tmp, garbage=4, deflate=True)
        os.replace(tmp, in_path)
    return in_path


def act_info(doc, args):
    lines = [f"pages: {doc.page_count}"]
    md = {k: v for k, v in (doc.metadata or {}).items() if v}
    if md:
        lines.append("metadata: " + ", ".join(f"{k}={v!r}" for k, v in md.items()))
    fonts = set()
    for pi in range(min(doc.page_count, 50)):
        page = doc[pi]
        for f in page.get_fonts(full=True):
            fonts.add(f"{f[3]} ({f[2]}{', embedded' if f[1] and 'n/a' not in f[1] else ''})")
    if fonts:
        lines.append("fonts: " + "; ".join(sorted(fonts)))
    n = int(args.get("max_pages") or 5)
    for pi in range(min(doc.page_count, n)):
        page = doc[pi]
        txt = page.get_text("text").strip()
        first = txt.splitlines()[0][:80] if txt else "(no text)"
        lines.append(f"page {pi + 1}: {page.rect.width:.0f}x{page.rect.height:.0f}pt, rot={page.rotation}, {len(txt)} chars, {len(page.get_images())} image(s) — {first!r}")
    if doc.is_encrypted:
        lines.append("encrypted: yes")
    return "\n".join(lines)


def act_find(doc, args):
    text = args.get("text") or args.get("old")
    if not text:
        raise ValueError("'text' is required for find")
    hits = find_hits(doc, text, args.get("page"))
    if not hits:
        return f"no occurrences of {text!r}"
    out = [f"{len(hits)} occurrence(s) of {text!r}:"]
    for i, h in enumerate(hits, 1):
        out.append(f"#{i}  " + hit_desc(h))
    return "\n".join(out)


def act_replace(doc, args, notes):
    old, new = args.get("old"), args.get("new")
    if not old or new is None:
        raise ValueError("'old' and 'new' are required for replace")
    hits = find_hits(doc, old, args.get("page"))
    if not hits:
        raise ValueError(f"{old!r} not found" + (f" on page {args['page']}" if args.get("page") else "") + " (text must lie on one line as extracted; use action=find to inspect)")
    occ = args.get("occurrence")
    if occ is not None:
        occ = int(occ)
        if occ < 1 or occ > len(hits):
            raise ValueError(f"occurrence {occ} out of range: {len(hits)} match(es)")
        hits = [hits[occ - 1]]
    fit = args.get("fit", True)
    align = (args.get("align") or "left").lower()
    fixed_size = args.get("font_size")
    fixed_color = args.get("color")
    style = {k: args.get(k) for k in ("bold", "italic", "mono", "serif")}

    # group by page: redact all rects on a page first, then write the new text
    by_page = {}
    for h in hits:
        by_page.setdefault(h["page"], []).append(h)
    fill = args.get("fill")
    fill_c = parse_color(fill) if fill else False
    done = []
    for pno, phits in by_page.items():
        page = doc[pno - 1]
        for h in phits:
            page.add_redact_annot(h["rect"], fill=fill_c)
        try:
            page.apply_redactions(images=fitz.PDF_REDACT_IMAGE_NONE, graphics=fitz.PDF_REDACT_LINE_ART_NONE)
        except TypeError:  # older PyMuPDF without graphics=
            page.apply_redactions(images=fitz.PDF_REDACT_IMAGE_NONE)
        if new == "":
            done.extend(f"page {pno}: removed {old!r} at x={h['rect'].x0:.1f} y={h['rect'].y0:.1f}" for h in phits)
            continue
        for h in phits:
            s = h["span"]
            r = h["rect"]
            size = float(fixed_size) if fixed_size else (s["size"] if s else max(r.height * 0.75, 4))
            color = parse_color(fixed_color) if fixed_color else (parse_color(s["color"]) if s else (0, 0, 0))
            fname = font_for(s["flags"] if s else 0, **style)
            font, fnotes = font_with_glyphs(fname, new)
            notes.extend(fnotes)
            baseline = s["origin"][1] if s else r.y1 - r.height * 0.22
            width = font.text_length(new, fontsize=size)
            if fit and width > r.width + 0.5 and free_to_right(page, r, width):
                notes.append(f"page {pno}: {new!r} is wider than {old!r} ({width:.1f} > {r.width:.1f}pt); nothing to the right, so the line was extended at {size:.1f}pt")
            elif fit and width > r.width + 0.5:
                new_size = max(4.0, size * r.width / width)
                notes.append(f"page {pno}: {new!r} is wider than {old!r} ({width:.1f} > {r.width:.1f}pt); font size {size:.1f} -> {new_size:.1f}")
                size = new_size
                width = font.text_length(new, fontsize=size)
            x = r.x0
            if align == "center":
                x = r.x0 + (r.width - width) / 2
            elif align == "right":
                x = r.x1 - width
            tw = fitz.TextWriter(page.rect, color=color)
            tw.append((x, baseline), new, font=font, fontsize=size)
            tw.write_text(page)
            done.append(f"page {pno}: {old!r} -> {new!r} at x={x:.1f} y={r.y0:.1f} ({font.name} {size:.1f}pt)")
    saved = save(doc, args["path"], args, notes)
    return f"replaced {len(hits)} occurrence(s); saved {saved}\n" + "\n".join(done)


def act_add_text(doc, args, notes):
    text = args.get("text")
    if not text:
        raise ValueError("'text' is required for add_text")
    pno = int(args.get("page") or 1)
    if pno < 1 or pno > doc.page_count:
        raise ValueError(f"page {pno} out of range")
    page = doc[pno - 1]
    x, y = float(args.get("x", 72)), float(args.get("y", 72))
    size = float(args.get("font_size") or 11)
    color = parse_color(args.get("color"))
    style = {k: args.get(k) for k in ("bold", "italic", "mono", "serif")}
    font, fnotes = font_with_glyphs(font_for(0, **style), text)
    notes.extend(fnotes)
    tw = fitz.TextWriter(page.rect, color=color)
    yy = y
    for line in text.split("\n"):
        tw.append((x, yy), line, font=font, fontsize=size)
        yy += size * 1.2
    tw.write_text(page)
    saved = save(doc, args["path"], args, notes)
    return f"added {len(text.splitlines())} line(s) on page {pno} at ({x:.1f},{y:.1f}) baseline, {font.name} {size:.1f}pt; saved {saved}"


def act_render(doc, args):
    pno = int(args.get("page") or 1)
    if pno < 1 or pno > doc.page_count:
        raise ValueError(f"page {pno} out of range")
    dpi = int(args.get("dpi") or 110)
    page = doc[pno - 1]
    clip = None
    if args.get("clip"):
        c = args["clip"]
        clip = fitz.Rect(*[float(v) for v in c])
    pix = page.get_pixmap(dpi=dpi, clip=clip)
    png = pix.tobytes("png")
    out = args.get("output")
    text = f"page {pno} rendered at {dpi} dpi ({pix.width}x{pix.height}px)"
    if out:
        with open(out, "wb") as f:
            f.write(png)
        text += f", saved {out}"
    return text, {"mime": "image/png", "b64": base64.b64encode(png).decode()}


def main():
    args = json.load(open(sys.argv[1]))
    action = args.get("action") or "find"
    path = args.get("path")
    if not path:
        fail("'path' is required")
    try:
        doc = fitz.open(path)
    except Exception as e:
        fail(f"cannot open {path}: {e}")
    if doc.is_encrypted:
        pw = args.get("password") or ""
        if not doc.authenticate(pw):
            fail("PDF is encrypted; pass 'password'")
    notes = []
    image = None
    try:
        if action == "info":
            text = act_info(doc, args)
        elif action == "find":
            text = act_find(doc, args)
        elif action == "replace":
            text = act_replace(doc, args, notes)
        elif action == "add_text":
            text = act_add_text(doc, args, notes)
        elif action == "render":
            text, image = act_render(doc, args)
        else:
            raise ValueError(f"unknown action {action!r} (info | find | replace | add_text | render)")
    except Exception as e:
        fail(f"{type(e).__name__}: {e}")
    if notes:
        text += "\nnotes:\n- " + "\n- ".join(notes)
    out = {"ok": True, "text": text}
    if image:
        out["image"] = image
    print(json.dumps(out))


if __name__ == "__main__":
    main()
