# pdf_edit helper — runs under PyMuPDF (python3 with pymupdf, or `uv run --with pymupdf`).
# Reads a JSON args file (argv[1]) and prints one JSON object on stdout:
#   {"ok": true, "text": "...", "image": {"mime": "...", "b64": "..."}?}  or  {"ok": false, "error": "..."}
# Actions: info | find | replace | add_text | render
import base64
import json
import os
import re
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


BASE14 = {
    "helvetica": "helv", "helvetica-bold": "hebo", "helvetica-oblique": "heit", "helvetica-boldoblique": "hebi",
    "arial": "helv", "arial-bold": "hebo", "arial-italic": "heit", "arial-bolditalic": "hebi", "arialmt": "helv",
    "arial-boldmt": "hebo", "arial-italicmt": "heit", "arial-bolditalicmt": "hebi",
    "times-roman": "tiro", "times-bold": "tibo", "times-italic": "tiit", "times-bolditalic": "tibi",
    "timesnewroman": "tiro", "timesnewroman-bold": "tibo", "timesnewroman-italic": "tiit", "timesnewroman-bolditalic": "tibi",
    "timesnewromanpsmt": "tiro", "timesnewromanps-boldmt": "tibo", "timesnewromanps-italicmt": "tiit", "timesnewromanps-bolditalicmt": "tibi",
    "courier": "cour", "courier-bold": "cobo", "courier-oblique": "coit", "courier-boldoblique": "cobi",
    "couriernew": "cour", "couriernew-bold": "cobo", "couriernew-italic": "coit", "couriernew-bolditalic": "cobi",
    "couriernewpsmt": "cour", "couriernewps-boldmt": "cobo", "couriernewps-italicmt": "coit", "couriernewps-bolditalicmt": "cobi",
    "symbol": "symb", "zapfdingbats": "zadb",
    "nimbussans": "helv", "nimbussansregular": "helv", "nimbussansbold": "hebo", "nimbussansitalic": "heit", "nimbussansbolditalic": "hebi",
    "nimbusroman": "tiro", "nimbusromanregular": "tiro", "nimbusromanbold": "tibo", "nimbusromanitalic": "tiit", "nimbusromanbolditalic": "tibi",
    "nimbusmonops": "cour", "nimbusmonopsregular": "cour", "nimbusmonopsbold": "cobo", "nimbusmonopsitalic": "coit", "nimbusmonopsbolditalic": "cobi",
}
BASE14 = {re.sub(r"[^a-z0-9]", "", k): v for k, v in BASE14.items()}

# characters PDFs often store as a lookalike code point (soft hyphen for '-', nbsp for ' ', …)
EQUIV = {45: (173, 8208, 8209, 8211), 173: (45,), 8208: (45,), 8209: (45,), 8211: (45,),
         32: (160,), 160: (32,), 39: (8217,), 8217: (39,), 34: (8220, 8221), 8220: (34,), 8221: (34,)}


def fold(text):
    return re.sub(r"\s+", " ", text.replace("\xa0", " ").replace("\xad", "-").replace("\u2010", "-").replace("\u2011", "-").replace("\u2019", "'")).strip()


def variants(text):
    """Search variants of `text` for the common lookalike storage forms."""
    seen, out = set(), []
    for t in (text, text.replace("-", "\xad"), text.replace("-", "\u2010"), text.replace(" ", "\xa0"),
              text.replace("-", "\xad").replace(" ", "\xa0"), text.replace("'", "\u2019")):
        if t not in seen:
            seen.add(t)
            out.append(t)
    return out


FONT_DIRS = ["/System/Library/Fonts", "/System/Library/Fonts/Supplemental", "/Library/Fonts", "~/Library/Fonts",
             "/usr/share/fonts", "/usr/local/share/fonts", "~/.fonts", "~/.local/share/fonts", "C:/Windows/Fonts"]
_SYS_FONTS = None


def system_font(name):
    """A fitz.Font loaded from an installed font file whose name matches `name` (e.g. 'Georgia Bold'), or None."""
    global _SYS_FONTS
    if _SYS_FONTS is None:
        _SYS_FONTS = []
        for d in FONT_DIRS:
            d = os.path.expanduser(d)
            if not os.path.isdir(d):
                continue
            for root, _, files in os.walk(d):
                for fn in files:
                    stem, ext = os.path.splitext(fn)
                    if ext.lower() in (".ttf", ".otf", ".ttc"):
                        _SYS_FONTS.append((norm_name(stem), os.path.join(root, fn)))
    target = norm_name(name)
    if not target:
        return None
    wanted = {target}
    if target.endswith("regular"):
        wanted.add(target[:-7])
    for style, suf in (("bolditalic", "bi"), ("bold", "b"), ("italic", "i"), ("boldoblique", "bi"), ("oblique", "i")):
        if target.endswith(style):
            wanted.add(target[: -len(style)] + suf)
            wanted.add(target[: -len(style)] + "-" + suf)
    for stem, path in _SYS_FONTS:
        if stem in wanted:
            try:
                return fitz.Font(fontfile=path)
            except Exception:
                continue
    # second pass: files that start with the family name; compare the font's own full name
    fam = re.split(r"(bold|italic|regular|oblique|light|medium)", target)[0]
    if len(fam) >= 4:
        for stem, path in _SYS_FONTS:
            if stem.startswith(fam):
                try:
                    f = fitz.Font(fontfile=path)
                except Exception:
                    continue
                if norm_name(f.name) in wanted:
                    return f
    return None


def norm_name(n):
    return re.sub(r"[^a-z0-9]", "", (n or "").split("+")[-1].lower())


_FONT_INDEX = {}


def page_fonts(doc, page):
    """[{xref, type, basefont, resname, enc, ext, buf, font, names}] for the page's fonts (cached per page)."""
    if page.number in _FONT_INDEX:
        return _FONT_INDEX[page.number]
    out = []
    try:
        for f in page.get_fonts(full=True):
            e = {"xref": f[0], "type": f[2], "basefont": f[3], "resname": f[4], "enc": f[5] or "", "ext": "", "buf": b"", "font": None}
            e["names"] = {norm_name(f[3])}
            try:
                _, e["ext"], _, e["buf"] = doc.extract_font(f[0])
            except Exception:
                pass
            if e["buf"] and e["ext"] in ("ttf", "otf", "cff", "pfa", "pfb", "ttc") and e["type"] != "Type3":
                try:
                    e["font"] = fitz.Font(fontbuffer=e["buf"])
                    e["names"].add(norm_name(e["font"].name))
                except Exception:
                    e["font"] = None
            out.append(e)
    except Exception:
        pass
    _FONT_INDEX[page.number] = out
    return out


def match_font(doc, page, span_font):
    """The page font entry the span is set in (by BaseFont or the embedded program's own name)."""
    n = norm_name(span_font)
    for e in page_fonts(doc, page):
        if n in e["names"]:
            return e
    return None


class CidFont:
    """Write text with a Type0/Identity-H font that already lives in the PDF, by glyph id — the exact same font
    resource the original text uses (no re-embedding, no cmap needed). Glyph ids for characters are learned from
    the text the document already sets in that font, so only those characters can be written."""

    def __init__(self, doc, entry, name):
        self.doc, self.xref, self.name, self.resname = doc, entry["xref"], name, entry["resname"]
        self.font = entry["font"]  # the embedded program (may lack a cmap)
        names = set(entry["names"]) | {norm_name(name)}
        self.gids = {}  # unicode -> gid
        for page in doc:
            for sp in page.get_texttrace():
                if sp.get("type") not in (0, 1, None) or norm_name(sp["font"]) not in names:
                    continue
                for ch in sp["chars"]:
                    u, gid = ch[0], ch[1]
                    if gid or u in (32, 160):
                        self.gids.setdefault(u, gid)
        # glyph widths from the CIDFont's /W array (1/1000 text space), default /DW
        self.dw, self.widths = 1000.0, {}
        m = re.search(r"(\d+)\s+0\s+R", self._val(self.xref, "DescendantFonts"))
        if not m:
            raise ValueError("no DescendantFonts")
        dx = int(m.group(1))
        c2g = self._val(dx, "CIDToGIDMap")
        if c2g and "Identity" not in c2g and c2g != "null":
            raise ValueError("non-identity CIDToGIDMap")
        dw = self._val(dx, "DW")
        if dw and dw != "null":
            self.dw = float(dw)
        self._parse_w(self._val(dx, "W"))

    def _val(self, xref, key):
        t, v = self.doc.xref_get_key(xref, key)
        if t == "xref":
            return self.doc.xref_object(int(v.split()[0]))
        return v

    def _parse_w(self, w):
        if not w or w == "null":
            return
        toks = re.findall(r"\[|\]|-?\d+(?:\.\d+)?", w)
        toks = toks[1:-1] if toks and toks[0] == "[" else toks
        i = 0
        while i < len(toks):
            if not re.match(r"-?\d", toks[i]):
                i += 1
                continue
            c = int(float(toks[i]))
            if i + 1 < len(toks) and toks[i + 1] == "[":
                i += 2
                while i < len(toks) and toks[i] != "]":
                    self.widths[c] = float(toks[i])
                    c += 1
                    i += 1
                i += 1
            elif i + 2 < len(toks):
                c2, wv = int(float(toks[i + 1])), float(toks[i + 2])
                for cc in range(c, min(c2, c + 65535) + 1):
                    self.widths[cc] = wv
                i += 3
            else:
                break

    def gid(self, ch):
        u = ord(ch)
        g = self.gids.get(u)
        if g is None:
            for alt in EQUIV.get(u, ()):
                if alt in self.gids:
                    g = self.gids[alt]
                    break
        if g is None and self.font is not None:
            # the embedded program has a cmap: use it, but only for glyphs the PDF's /W array knows (else spacing breaks)
            try:
                g2 = self.font.has_glyph(u)
            except Exception:
                g2 = 0
            if g2 and (g2 in self.widths or not self.widths):
                g = g2
                self.gids[u] = g
        return g

    def missing(self, text):
        return [ch for ch in set(text) if self.gid(ch) is None]

    def text_length(self, text, fontsize):
        return sum(self.widths.get(self.gid(ch), self.dw) for ch in text) * fontsize / 1000.0

    def write(self, page, x, y, text, fontsize, color, direction=(1.0, 0.0)):
        """Append a text object to the page content: baseline start (x, y) in PyMuPDF (unrotated, top-left)
        coordinates, running along `direction` (the line's unit vector, e.g. (0,-1) for text going up)."""
        if not page.is_wrapped:
            page.wrap_contents()
        # make sure the page's own resources reference the font under `resname`
        t, v = self.doc.xref_get_key(page.xref, f"Resources/Font/{self.resname}")
        if t == "null":
            self.doc.xref_set_key(page.xref, f"Resources/Font/{self.resname}", f"{self.xref} 0 R")
        dx, dy = direction
        # text space -> fitz space (x-axis along the baseline, y-axis toward the ascenders) -> PDF space
        m = fitz.Matrix(dx, dy, dy, -dx, x, y) * page.transformation_matrix
        hexs = "".join(f"{self.gid(ch):04x}" for ch in text)
        r, g, b = color
        cont = (f"\nq BT /{self.resname} {fontsize:g} Tf {r:g} {g:g} {b:g} rg 0 Tr "
                f"{m.a:.4f} {m.b:.4f} {m.c:.4f} {m.d:.4f} {m.e:.3f} {m.f:.3f} Tm <{hexs}> Tj ET Q\n")
        xrefs = page.get_contents()
        if not xrefs:
            xr = self.doc.get_new_xref()
            self.doc.update_object(xr, "<<>>")
            self.doc.update_stream(xr, b"")
            self.doc.xref_set_key(page.xref, "Contents", f"{xr} 0 R")
            xrefs = [xr]
        last = xrefs[-1]
        self.doc.update_stream(last, self.doc.xref_stream(last) + cont.encode("latin-1"))


_CID_CACHE = {}


def cid_font(doc, page, span, text):
    """The page's own Type0/Identity-H font for `span` if it can set `text`; else (None, note)."""
    if not span:
        return None, None
    sname = span["font"]
    key = ("cid", sname)
    if key not in _CID_CACHE:
        found = None
        e = match_font(doc, page, sname)
        if e and e["type"] == "Type0" and "Identity" in e["enc"]:
            try:
                found = CidFont(doc, e, sname)
            except Exception:
                found = None
        _CID_CACHE[key] = found
    cf = _CID_CACHE[key]
    if cf is None:
        return None, None
    miss = cf.missing(text)
    if miss:
        return None, f"embedded font {sname} (subset) has no glyphs for {''.join(sorted(miss))!r} (only characters already set in that font can be reused)"
    return cf, None


def original_font(doc, page, span, text):
    """The font the span is set in, as a fitz.Font usable for writing `text`, or None.

    Prefers the font program embedded in the PDF (exact same typeface, weight and slant); if that is a subset
    lacking glyphs for `text` (or not embedded / not loadable), falls back to the matching base-14 font when the
    name is a known standard family. Returns (font, note) — note explains any deviation."""
    if not span:
        return None, None
    sname = span["font"]
    e = match_font(doc, page, sname)
    f = e["font"] if e else None
    if f is not None:
        missing = [ch for ch in set(text) if not ch.isspace() and not f.has_glyph(ord(ch))]
        if not missing:
            return f, None
        note = f"embedded font {sname} lacks {''.join(sorted(missing))!r}"
    else:
        note = None
    std = BASE14.get(norm_name(sname)) or (BASE14.get(norm_name(e["basefont"])) if e else None)
    if std:
        g = fitz.Font(std)
        if all(ch.isspace() or g.has_glyph(ord(ch)) for ch in text):
            return g, (note + f"; used standard {g.name}" if note else None)
    sysf = system_font(sname) or (system_font(e["basefont"]) if e else None)
    if sysf is not None and all(ch.isspace() or sysf.has_glyph(ord(ch)) for ch in text):
        return sysf, (note + f"; used the installed {sysf.name}" if note else f"embedded font {sname} could not be reused; used the installed {sysf.name} instead")
    return None, note


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


def line_text(line):
    return "".join(s["text"] for s in line["spans"])


_RAW_CACHE = {}


def chars_in(page, rect):
    """The characters whose centre lies in `rect`, left to right (whitespace normalised)."""
    if page.number not in _RAW_CACHE:
        chars = []
        for b in page.get_text("rawdict")["blocks"]:
            if b.get("type") != 0:
                continue
            for l in b["lines"]:
                for s in l["spans"]:
                    for c in s["chars"]:
                        bb = fitz.Rect(c["bbox"])
                        chars.append(((bb.x0 + bb.x1) / 2, (bb.y0 + bb.y1) / 2, c["c"]))
        _RAW_CACHE[page.number] = chars
    r = fitz.Rect(rect)
    return "".join(c for x, y, c in sorted(_RAW_CACHE[page.number]) if r.contains(fitz.Point(x, y)))


def find_hits(doc, text, page_arg):
    """Case-sensitive occurrences of `text` (search_for itself ignores case, so hits are verified against the
    extracted text of the line). Returns (hits, case_only) — case_only counts matches that differ only in case."""
    hits = []
    case_only = 0
    flags = fitz.TEXT_DEHYPHENATE
    for pi in page_indices(doc, page_arg):
        page = doc[pi]
        rects = []
        for t in variants(text):
            rects = page.search_for(t, flags=flags)
            if rects:
                break
        for r in rects:
            span, line = best_span(page, r)
            got = fold(chars_in(page, r))
            if got and got != fold(text) and got.lower() == fold(text).lower():
                case_only += 1
                continue
            hits.append({"page": pi + 1, "rect": r, "span": span, "line": line})
    return hits, case_only


def line_geometry(hit):
    """(direction, start point on the baseline, extent along the direction) of a hit, in unrotated page coords."""
    r, s, l = hit["rect"], hit["span"], hit["line"]
    dx, dy = (l["dir"] if l else (1.0, 0.0))
    if abs(dx) < 1e-6 and abs(dy) < 1e-6:
        dx, dy = 1.0, 0.0
    d = fitz.Point(dx, dy).unit
    extent = abs(d.x) * r.width + abs(d.y) * r.height
    if s:
        o = fitz.Point(s["origin"])
        c = fitz.Point((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2)
        t = (c.x - o.x) * d.x + (c.y - o.y) * d.y
        start = o + d * (t - extent / 2)
    else:
        start = fitz.Point(r.x0, r.y1 - r.height * 0.22) if abs(d.x) >= abs(d.y) else fitz.Point(r.x0 + r.width * 0.78, r.y1 if d.y < 0 else r.y0)
    return d, start, extent


def overlaps_ahead(page, hit, needed):
    """Text spans that a run of length `needed` from the hit's start would overlap beyond the old text's extent."""
    r = hit["rect"]
    d, start, extent = line_geometry(hit)
    p1, p2 = start + d * (extent + 0.5), start + d * needed
    ext = fitz.Rect(min(p1.x, p2.x), min(p1.y, p2.y), max(p1.x, p2.x), max(p1.y, p2.y))
    if abs(d.x) >= abs(d.y):
        ext.y0, ext.y1 = r.y0, r.y1
    else:
        ext.x0, ext.x1 = r.x0, r.x1
    out = []
    for s, _ in spans_of(page):
        if s["text"].strip() and not (fitz.Rect(s["bbox"]) & ext).is_empty:
            out.append(s["text"].strip())
    if not page.rect.contains(ext):
        out.append("<page edge>")
    return out


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
    hits, case_only = find_hits(doc, text, args.get("page"))
    if not hits:
        if case_only:
            return f"no occurrences of {text!r} ({case_only} match(es) differ only in letter case)"
        return f"no occurrences of {text!r}"
    out = [f"{len(hits)} occurrence(s) of {text!r}:"]
    for i, h in enumerate(hits, 1):
        out.append(f"#{i}  " + hit_desc(h))
    return "\n".join(out)


def act_replace(doc, args, notes):
    old, new = args.get("old"), args.get("new")
    if not old or new is None:
        raise ValueError("'old' and 'new' are required for replace")
    hits, case_only = find_hits(doc, old, args.get("page"))
    if not hits:
        where = f" on page {args['page']}" if args.get("page") else ""
        if case_only:
            raise ValueError(f"{old!r} not found{where} — {case_only} match(es) differ only in letter case (matching is case-sensitive; use action=find)")
        raise ValueError(f"{old!r} not found{where} (text must lie on one line as extracted; use action=find to inspect)")
    occ = args.get("occurrence")
    if occ is not None:
        occ = int(occ)
        if occ < 1 or occ > len(hits):
            raise ValueError(f"occurrence {occ} out of range: {len(hits)} match(es)")
        hits = [hits[occ - 1]]
    fit = bool(args.get("fit", False))
    align = (args.get("align") or "left").lower()
    fixed_size = args.get("font_size")
    fixed_color = args.get("color")
    style = {k: args.get(k) for k in ("bold", "italic", "mono", "serif")}
    restyle = any(v is not None for v in style.values())

    # pick the document's own font per hit BEFORE redacting (the old text may hold the only samples of its glyphs)
    for h in hits:
        h["cf"], h["cnote"] = (None, None)
        if not restyle and new != "":
            h["cf"], h["cnote"] = cid_font(doc, doc[h["page"] - 1], h["span"], new)
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
            # font: the document's own (embedded) font unless the caller asks for a different style
            font, cf = None, h["cf"]
            if not restyle:
                if cf is None:
                    font, fnote = original_font(doc, page, s, new)
                    n = fnote if font is not None else (h["cnote"] or fnote)
                    if n:
                        notes.append(f"page {pno}: {n}")
                else:
                    font = cf
            if font is None:
                fname = font_for(s["flags"] if s else 0, **style)
                font, fnotes = font_with_glyphs(fname, new)
                notes.extend(fnotes)
                if s and not restyle and norm_name(s["font"]) not in BASE14:
                    notes.append(f"page {pno}: original font {s['font']} could not be reused; approximated with {font.name}")
            d, start, extent = line_geometry(h)
            width = font.text_length(new, fontsize=size)
            if width > extent + 0.5:
                blockers = overlaps_ahead(page, h, width)
                if fit and blockers:
                    new_size = max(4.0, size * extent / width)
                    notes.append(f"page {pno}: {new!r} is wider than {old!r} ({width:.1f} > {extent:.1f}pt); fit=true so font size {size:.1f} -> {new_size:.1f}")
                    size = new_size
                    width = font.text_length(new, fontsize=size)
                elif blockers:
                    notes.append(f"page {pno}: WARNING {new!r} is wider than {old!r} ({width:.1f} > {extent:.1f}pt) and at {size:.1f}pt overlaps {', '.join(repr(b) for b in blockers[:3])} — check with action=render; pass fit=true to shrink to the old width, or use shorter text")
                else:
                    notes.append(f"page {pno}: {new!r} is wider than {old!r} ({width:.1f} > {extent:.1f}pt); nothing to the right, so the line was extended at {size:.1f}pt")
            pos = start
            if align == "center":
                pos = start + d * ((extent - width) / 2)
            elif align == "right":
                pos = start + d * (extent - width)
            if cf is not None:
                cf.write(page, pos.x, pos.y, new, size, color, (d.x, d.y))
                fdesc = f"{cf.name}, the document's own font"
            else:
                tw = fitz.TextWriter(page.rect, color=color)
                tw.append(pos, new, font=font, fontsize=size)
                morph = None
                if abs(d.x - 1) > 1e-6 or abs(d.y) > 1e-6:
                    import math
                    morph = (pos, fitz.Matrix(-math.degrees(math.atan2(d.y, d.x))))  # morph acts in y-up space
                tw.write_text(page, morph=morph)
                fdesc = font.name
            done.append(f"page {pno}: {old!r} -> {new!r} at x={pos.x:.1f} y={pos.y:.1f} ({fdesc} {size:.1f}pt)")
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
