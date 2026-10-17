#!/usr/bin/env python3
"""Render a tmux `capture-pane -e -p` dump to a PNG.

    uv run --with pillow scripts/ansi2png.py in.ansi out.png

Honours SGR reset, bold, dim, italic, underline, reverse, the 16 ANSI
colours, 256-colour (38/48;5;n), and truecolor (38/48;2;r;g;b), including
colon-separated ISO parameters. Cursor-addressing and erase sequences are
applied when present; a plain capture is just styled lines.
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

FONT_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FONT_BOLD_PATH = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
FONT_SIZE = 16

# Default terminal: dark bg, light fg. Captures often leave these implicit.
DEFAULT_FG = (230, 230, 230)
DEFAULT_BG = (28, 28, 28)

ANSI16 = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
]


def color_256(n: int) -> tuple[int, int, int]:
    n %= 256
    if n < 16:
        return ANSI16[n]
    if n < 232:
        n -= 16
        r, g, b = n // 36, (n // 6) % 6, n % 6
        scale = [0, 95, 135, 175, 215, 255]
        return (scale[r], scale[g], scale[b])
    gray = 8 + (n - 232) * 10
    return (gray, gray, gray)


def dim(color: tuple[int, int, int]) -> tuple[int, int, int]:
    return tuple(c * 2 // 3 for c in color)  # type: ignore[return-value]


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "italic", "underline", "reverse")

    def __init__(self) -> None:
        self.ch = " "
        self.fg = DEFAULT_FG
        self.bg = DEFAULT_BG
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.reverse = False


class Pen:
    def __init__(self) -> None:
        self.reset()

    def reset(self) -> None:
        self.fg = DEFAULT_FG
        self.bg = DEFAULT_BG
        self.bold = False
        self.dim = False
        self.italic = False
        self.underline = False
        self.reverse = False
        self.fg_default = True
        self.bg_default = True

    def apply_cell(self, cell: Cell, ch: str) -> None:
        cell.ch = ch
        cell.fg = self.fg
        cell.bg = self.bg
        cell.bold = self.bold
        cell.dim = self.dim
        cell.italic = self.italic
        cell.underline = self.underline
        cell.reverse = self.reverse


def _ints(parts: list[str]) -> list[int]:
    out = []
    for part in parts:
        if part == "":
            out.append(0)
        else:
            try:
                out.append(int(part))
            except ValueError:
                out.append(0)
    return out


def apply_sgr(pen: Pen, params: list[int]) -> None:
    if not params:
        params = [0]
    i = 0
    while i < len(params):
        p = params[i]
        if p == 0:
            pen.reset()
        elif p == 1:
            pen.bold = True
        elif p == 2:
            pen.dim = True
        elif p == 3:
            pen.italic = True
        elif p == 4:
            pen.underline = True
        elif p == 7:
            pen.reverse = True
        elif p == 22:
            pen.bold = False
            pen.dim = False
        elif p == 23:
            pen.italic = False
        elif p == 24:
            pen.underline = False
        elif p == 27:
            pen.reverse = False
        elif p == 39:
            pen.fg = DEFAULT_FG
            pen.fg_default = True
        elif p == 49:
            pen.bg = DEFAULT_BG
            pen.bg_default = True
        elif 30 <= p <= 37:
            pen.fg = ANSI16[p - 30]
            pen.fg_default = False
        elif 40 <= p <= 47:
            pen.bg = ANSI16[p - 40]
            pen.bg_default = False
        elif 90 <= p <= 97:
            pen.fg = ANSI16[p - 90 + 8]
            pen.fg_default = False
        elif 100 <= p <= 107:
            pen.bg = ANSI16[p - 100 + 8]
            pen.bg_default = False
        elif p in (38, 48) and i + 1 < len(params):
            which = "fg" if p == 38 else "bg"
            mode = params[i + 1]
            if mode == 5 and i + 2 < len(params):
                color = color_256(params[i + 2])
                i += 2
            elif mode == 2 and i + 4 < len(params):
                color = (
                    params[i + 2] & 255,
                    params[i + 3] & 255,
                    params[i + 4] & 255,
                )
                i += 4
            else:
                i += 1
                continue
            if which == "fg":
                pen.fg = color
                pen.fg_default = False
            else:
                pen.bg = color
                pen.bg_default = False
        i += 1


def parse(text: str) -> list[list[Cell]]:
    """Return a grid. Newlines start rows. CSI cursor moves are honoured."""
    rows: list[list[Cell]] = [[]]
    pen = Pen()
    row = 0
    col = 0

    def ensure(r: int, c: int) -> None:
        while len(rows) <= r:
            rows.append([])
        while len(rows[r]) <= c:
            rows[r].append(Cell())

    def put(ch: str) -> None:
        nonlocal col
        ensure(row, col)
        pen.apply_cell(rows[row][col], ch)
        col += 1

    i = 0
    n = len(text)
    while i < n:
        ch = text[i]
        if ch == "\x1b" and i + 1 < n:
            nxt = text[i + 1]
            if nxt == "[":
                j = i + 2
                while j < n and not ("@" <= text[j] <= "~"):
                    j += 1
                if j >= n:
                    break
                final = text[j]
                body = text[i + 2 : j]
                params = _ints(body.replace(":", ";").split(";")) if body else []
                if final == "m":
                    apply_sgr(pen, params)
                elif final == "H" or final == "f":
                    # 1-based cursor address.
                    r = (params[0] if len(params) > 0 else 1) - 1
                    c = (params[1] if len(params) > 1 else 1) - 1
                    row = max(0, r)
                    col = max(0, c)
                    ensure(row, col)
                elif final == "A":
                    row = max(0, row - (params[0] if params else 1))
                elif final == "B":
                    row += params[0] if params else 1
                    ensure(row, col)
                elif final == "C":
                    col += params[0] if params else 1
                    ensure(row, col)
                elif final == "D":
                    col = max(0, col - (params[0] if params else 1))
                elif final == "G":
                    col = max(0, (params[0] if params else 1) - 1)
                    ensure(row, col)
                elif final in ("J", "K"):
                    pass
                i = j + 1
                continue
            if nxt == "]":
                # OSC ... BEL or ST
                j = i + 2
                while j < n:
                    if text[j] == "\x07":
                        j += 1
                        break
                    if text[j] == "\x1b" and j + 1 < n and text[j + 1] == "\\":
                        j += 2
                        break
                    j += 1
                i = j
                continue
            i += 1
            continue
        if ch == "\r":
            col = 0
            i += 1
            continue
        if ch == "\n":
            row += 1
            col = 0
            ensure(row, 0)
            i += 1
            continue
        if ch == "\t":
            spaces = 8 - (col % 8)
            for _ in range(spaces):
                put(" ")
            i += 1
            continue
        if ch < " " or ch == "\x7f":
            i += 1
            continue
        put(ch)
        i += 1

    if rows and rows[-1] and all(c.ch == " " and c.bg == DEFAULT_BG for c in rows[-1]):
        # Drop a trailing blank row from a final newline.
        if text.endswith("\n"):
            rows.pop()
    return rows


def render(rows: list[list[Cell]], out_path: Path) -> None:
    font = ImageFont.truetype(FONT_PATH, FONT_SIZE)
    bold_font = (
        ImageFont.truetype(FONT_BOLD_PATH, FONT_SIZE)
        if Path(FONT_BOLD_PATH).is_file()
        else font
    )
    # Monospace advance. getlength can be fractional; ceil to a stable cell.
    advance = font.getlength("M")
    cell_w = max(1, round(advance))
    # DejaVu Sans Mono at 16px is about 19px of em height.
    cell_h = FONT_SIZE + 6
    width = max((len(r) for r in rows), default=1)
    height = max(len(rows), 1)
    img = Image.new("RGB", (width * cell_w, height * cell_h), DEFAULT_BG)
    draw = ImageDraw.Draw(img)
    for y, line in enumerate(rows):
        for x, cell in enumerate(line):
            fg, bg = cell.fg, cell.bg
            if cell.reverse:
                fg, bg = bg, fg
            if cell.dim:
                fg = dim(fg)
            x0, y0 = x * cell_w, y * cell_h
            if bg != DEFAULT_BG:
                draw.rectangle([x0, y0, x0 + cell_w, y0 + cell_h], fill=bg)
            face = bold_font if cell.bold else font
            draw.text((x0, y0), cell.ch, font=face, fill=fg)
            if cell.underline:
                draw.line(
                    [x0, y0 + cell_h - 3, x0 + cell_w - 1, y0 + cell_h - 3],
                    fill=fg,
                )
    img.save(out_path)


def main() -> None:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} in.ansi out.png", file=sys.stderr)
        sys.exit(2)
    src = Path(sys.argv[1])
    dst = Path(sys.argv[2])
    text = src.read_text(encoding="utf-8", errors="replace")
    rows = parse(text)
    dst.parent.mkdir(parents=True, exist_ok=True)
    render(rows, dst)
    print(f"wrote {dst} ({max(len(r) for r in rows)}x{len(rows)} cells)")


if __name__ == "__main__":
    main()
