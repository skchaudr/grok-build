#!/usr/bin/env python3
"""Port Zed themes to Grok Build: emits theme/zed_ports.rs and assets/<slug>.tmTheme.

Usage: zed_port.py <zed-themes-dir> <repo-root>
The Zed JSON is not vendored; fetch it read-only from an install
(~/Library/Application Support/Zed/extensions/installed/<ext>/themes/*.json).
Mapping is documented in docs/custom/THEMES.md; keep the two in sync.
"""
import json
import pathlib
import re
import subprocess
import sys

# (slug, rust fn, Zed file, Zed theme name)
PORTS = [
    ("kanso-mist", "kanso_mist", "kanso_themes_kanso.json", "Kanso Mist (Borderless + No italics)"),
    ("kanagawa-wave", "kanagawa_wave", "kanagawa-themes_themes_Kanagawa.json", "Kanagawa Wave"),
    ("kanagawa", "kanagawa", "kanagawa-themes_themes_Kanagawa.json", "Kanagawa"),
    ("jetbrains-islands-dark", "jetbrains_islands_dark",
     "jetbrains-themes_themes_jetbrains-islands-dark.json", "JetBrains Islands Dark"),
]

# Zed syntax key -> TextMate scopes. Later rows win ties (syntect: more specific selector wins).
SYNTAX_SCOPES = [
    ("variable", "variable, variable.other"),
    ("punctuation", "punctuation"),
    ("punctuation.bracket", "punctuation.section, punctuation.definition.block, meta.brace"),
    ("punctuation.delimiter", "punctuation.separator, punctuation.terminator, punctuation.accessor"),
    ("punctuation.special", "punctuation.definition.template-expression, punctuation.section.embedded"),
    ("punctuation.list_marker", "punctuation.definition.list.begin, punctuation.definition.list_item"),
    ("comment", "comment, punctuation.definition.comment"),
    ("comment.doc", "comment.block.documentation, comment.line.documentation"),
    ("string", "string, punctuation.definition.string"),
    ("string.escape", "constant.character.escape"),
    ("string.regex", "string.regexp"),
    ("string.special", "string.other, string.interpolated"),
    ("string.special.symbol", "constant.other.symbol"),
    ("number", "constant.numeric"),
    ("constant", "constant, variable.other.constant, support.constant"),
    ("boolean", "constant.language, constant.language.boolean"),
    ("keyword", "keyword, keyword.control, storage.type, storage.modifier"),
    ("operator", "keyword.operator"),
    ("operator.controlFlow", "keyword.control.flow, keyword.control.return"),
    ("function", "entity.name.function, support.function, variable.function, meta.function-call"),
    ("function.definition", "meta.definition.function entity.name.function, entity.name.function.definition"),
    ("function.special", "support.function.macro, entity.name.function.macro"),
    ("constructor", "entity.name.function.constructor, variable.function.constructor"),
    ("type", "entity.name.type, entity.name.class, support.type, support.class, entity.other.inherited-class"),
    ("type.builtin", "support.type.primitive, storage.type.primitive, storage.type.numeric, storage.type.boolean"),
    ("type.interface", "entity.name.type.interface, entity.name.type.trait, entity.name.interface"),
    ("type.parameter", "entity.name.type.parameter, variable.type.parameter"),
    ("enum", "entity.name.type.enum, entity.name.enum"),
    ("type.enum", "entity.name.type.enum, entity.name.enum"),
    ("variant", "variable.other.enummember, entity.name.constant.enum"),
    ("type.enum.member", "variable.other.enummember, entity.name.constant.enum"),
    ("property", "variable.other.property, variable.other.member, support.type.property-name, meta.object-literal.key, entity.name.tag.yaml"),
    ("variable.member", "variable.other.member"),
    ("variable.parameter", "variable.parameter"),
    ("variable.special", "variable.language"),
    ("namespace", "entity.name.namespace, entity.name.module, support.other.namespace"),
    ("lifetime", "storage.modifier.lifetime, entity.name.type.lifetime, punctuation.definition.lifetime"),
    ("label", "entity.name.label"),
    ("attribute", "entity.other.attribute-name, meta.attribute, meta.annotation"),
    ("tag", "entity.name.tag"),
    ("selector", "entity.name.tag.css, entity.other.attribute-name.class.css, entity.other.attribute-name.id.css"),
    ("selector.pseudo", "entity.other.attribute-name.pseudo-class, entity.other.attribute-name.pseudo-element"),
    ("preproc", "meta.preprocessor, keyword.control.directive"),
    ("title", "markup.heading, entity.name.section, punctuation.definition.heading"),
    ("emphasis", "markup.italic"),
    ("emphasis.strong", "markup.bold"),
    ("text.literal", "markup.raw, markup.inline.raw, markup.fenced_code"),
    ("link_text", "string.other.link, markup.link"),
    ("link_uri", "markup.underline.link"),
]


def load(path):
    text = re.sub(r"^\s*//.*$", "", path.read_text(), flags=re.M)
    return json.loads(text)


def parse(hexstr):
    h = hexstr.lstrip("#")
    r, g, b = (int(h[i:i + 2], 16) for i in (0, 2, 4))
    a = int(h[6:8], 16) / 255 if len(h) == 8 else 1.0
    return r, g, b, a


def over(fg, bg):
    """Composite a possibly-translucent Zed color onto an opaque background."""
    r, g, b, a = parse(fg)
    br, bgg, bb = bg
    return tuple(round(c * a + d * (1 - a)) for c, d in zip((r, g, b), (br, bgg, bb)))


def mix(fg, bg, t):
    return tuple(round(c * t + d * (1 - t)) for c, d in zip(fg, bg))


class Port:
    def __init__(self, style):
        self.s = style
        self.syn = style.get("syntax", {})
        self.base = parse(style["editor.background"])[:3]

    def ui(self, *keys):
        """First key with a visible (alpha > 0) value, composited over the editor bg."""
        for k in keys:
            v = self.s.get(k)
            if v and parse(v)[3] > 0:
                return over(v, self.base)
        raise KeyError(keys)

    def syntax(self, *keys):
        for k in keys:
            v = (self.syn.get(k) or {}).get("color")
            if v:
                return over(v, self.base)
        raise KeyError(keys)

    def first_distinct(self, *keys):
        for k in keys:
            v = self.s.get(k)
            if v and parse(v)[3] > 0 and over(v, self.base) != self.base:
                return over(v, self.base)
        raise KeyError(keys)

    def colors(self):
        p, s = self, self.ui
        selection = over(self.s["players"][0]["selection"], self.base)
        deleted, created = s("deleted"), s("created")
        def band(key):
            try:
                return self.first_distinct(key)
            except KeyError:
                return None
        del_bg, ins_bg = band("deleted.background"), band("created.background")
        if del_bg is None or ins_bg is None or del_bg == ins_bg or self.base in (del_bg, ins_bg):
            # Zed themes that share one band colour (or lack it) get 22% tints of each fg instead.
            del_bg = mix(deleted, self.base, 0.22)
            ins_bg = mix(created, self.base, 0.22)
        code_bg = p.first_distinct("elevated_surface.background", "surface.background",
                                   "panel.background", "editor.active_line.background", "element.hover")
        return {
            "bg_base": self.base,
            "bg_light": p.first_distinct("editor.active_line.background", "element.hover"),
            "bg_dark": code_bg,
            "bg_highlight": s("element.selected", "ghost_element.selected"),
            "bg_hover": s("element.hover", "ghost_element.hover"),
            "bg_terminal": s("terminal.background", "editor.background"),
            "accent_user": s("editor.foreground", "text"),
            "accent_assistant": p.syntax("keyword"),
            "accent_thinking": p.syntax("keyword"),
            "accent_tool": s("text.muted"),
            "accent_system": s("info"),
            "accent_error": s("error"),
            "accent_success": s("success"),
            "accent_running": p.syntax("keyword"),
            "accent_skill": s("info"),
            "text_primary": s("text"),
            "text_secondary": s("editor.foreground", "text"),
            "gray_dim": s("text.placeholder", "editor.line_number"),
            "gray": p.syntax("comment"),
            "gray_bright": s("text.muted"),
            "command": s("terminal.ansi.yellow"),
            "path": p.syntax("boolean", "constant"),
            "running": s("terminal.ansi.cyan"),
            "warning": s("warning"),
            "fuzzy_accent": s("text.accent"),
            "accent_plan": s("modified", "terminal.ansi.yellow"),
            "accent_verify": s("terminal.ansi.magenta"),
            "accent_remember": s("created", "success"),
            # Borderless variants paint borders at alpha 0; fall back to the selected-element fill.
            "selection_border": s("border.selected", "element.selected"),
            "hover_border": s("border.variant", "element.hover"),
            "prompt_border": s("border", "element.hover"),
            "prompt_border_active": s("border.focused", "element.selected"),
            "accent_model": p.syntax("type"),
            "scrollbar_bg": s("scrollbar.track.background", "editor.background"),
            "scrollbar_fg": s("scrollbar.thumb.background"),
            "diff_delete_bg": del_bg,
            "diff_delete_fg": deleted,
            "diff_insert_bg": ins_bg,
            "diff_insert_fg": created,
            "diff_equal_fg": p.syntax("comment"),
            "diff_gutter_fg": s("editor.line_number"),
            "bg_visual": selection,
            "paste_bg": code_bg,
            "paste_fg": s("text.muted"),
            "paste_dim": s("text.placeholder"),
            "md_heading_h1": p.syntax("title"),
            "md_heading_h2": p.syntax("title"),
            "md_heading_h3": p.syntax("title"),
            "md_heading_h4": p.syntax("title"),
            "md_heading_h5": s("text.muted"),
            "md_heading_h6": p.syntax("comment"),
            "md_code": p.syntax("text.literal", "string"),
            "md_task_checked": s("created", "success"),
            "md_task_unchecked": s("text.muted"),
            "md_muted": p.syntax("comment"),
            "md_code_bg": code_bg,
            "md_text": s("editor.foreground", "text"),
            "link_fg": p.syntax("link_text", "link_uri"),
        }

    def heading_mods(self):
        # Grok strips the `#` markers Zed keeps visible, so h1-h4 stay bold even when `title` is regular weight.
        italic = (self.syn.get("title") or {}).get("font_style") == "italic"
        top = "Modifier::BOLD.union(Modifier::ITALIC)" if italic else "Modifier::BOLD"
        return {f"md_heading_h{i}_mod": top if i <= 5 else "Modifier::empty()" for i in range(1, 7)}


FIELD_ORDER = [
    "bg_base", "bg_light", "bg_dark", "bg_highlight", "bg_hover", "bg_terminal",
    "accent_user", "accent_assistant", "accent_thinking", "accent_tool", "accent_system",
    "accent_error", "accent_success", "accent_running", "accent_skill",
    "text_primary", "text_secondary", "gray_dim", "gray", "gray_bright",
    "command", "path", "running", "warning", "fuzzy_accent", "accent_plan", "accent_verify",
    "accent_remember", "selection_border", "hover_border", "prompt_border", "prompt_border_active",
    "accent_model", "scrollbar_bg", "scrollbar_fg",
    "diff_delete_bg", "diff_delete_fg", "diff_insert_bg", "diff_insert_fg", "diff_equal_fg", "diff_gutter_fg",
    "bg_visual", "paste_bg", "paste_fg", "paste_dim",
    "md_heading_h1", "md_heading_h1_mod", "md_heading_h2", "md_heading_h2_mod",
    "md_heading_h3", "md_heading_h3_mod", "md_heading_h4", "md_heading_h4_mod",
    "md_heading_h5", "md_heading_h5_mod", "md_heading_h6", "md_heading_h6_mod",
    "md_code", "md_task_checked", "md_task_unchecked", "md_muted", "md_code_bg", "md_text", "link_fg",
]


def rust_fn(fn, name, port):
    colors, mods = port.colors(), port.heading_mods()
    lines = [f"    /// {name} (Zed), ported by scripts/themes/zed_port.py.",
             f"    pub const fn {fn}() -> Self {{", "        Self {"]
    for f in FIELD_ORDER:
        if f in mods:
            lines.append(f"            {f}: {mods[f]},")
        else:
            r, g, b = colors[f]
            lines.append(f"            {f}: rgb({r}, {g}, {b}), // #{r:02x}{g:02x}{b:02x}")
    lines += ["        }", "    }"]
    return "\n".join(lines)


def hexc(c):
    return "#%02x%02x%02x" % c


def tmtheme(name, port):
    s = port.s
    out = ['<?xml version="1.0" encoding="UTF-8"?>',
           '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
           '<plist version="1.0">', "<dict>",
           "\t<key>name</key>", f"\t<string>{name}</string>",
           "\t<key>settings</key>", "\t<array>"]

    def entry(label, scope, settings):
        out.append("\t\t<dict>")
        if label:
            out.extend(["\t\t\t<key>name</key>", f"\t\t\t<string>{label}</string>",
                        "\t\t\t<key>scope</key>", f"\t\t\t<string>{scope}</string>"])
        out.extend(["\t\t\t<key>settings</key>", "\t\t\t<dict>"])
        for k, v in settings.items():
            out.extend([f"\t\t\t\t<key>{k}</key>", f"\t\t\t\t<string>{v}</string>"])
        out.extend(["\t\t\t</dict>", "\t\t</dict>"])

    entry(None, None, {
        "background": hexc(port.base),
        "caret": hexc(over(s["players"][0]["cursor"], port.base)),
        "foreground": hexc(port.ui("editor.foreground", "text")),
        "lineHighlight": hexc(port.first_distinct("editor.active_line.background", "element.hover")),
        "selection": hexc(over(s["players"][0]["selection"], port.base)),
    })
    for key, scope in SYNTAX_SCOPES:
        tok = port.syn.get(key) or {}
        if not tok.get("color"):
            continue
        settings = {"foreground": hexc(over(tok["color"], port.base))}
        style = []
        if (tok.get("font_weight") or 400) >= 700:
            style.append("bold")
        if tok.get("font_style") == "italic":
            style.append("italic")
        settings["fontStyle"] = " ".join(style)
        entry(key, scope, settings)
    for label, scope, key in [("inserted", "markup.inserted", "created"),
                              ("deleted", "markup.deleted", "deleted"),
                              ("changed", "markup.changed", "modified"),
                              ("invalid", "invalid, invalid.illegal", "error")]:
        entry(label, scope, {"foreground": hexc(port.ui(key))})
    out += ["\t</array>", "</dict>", "</plist>", ""]
    return "\n".join(out)


def main():
    src, root = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    render = root / "crates/codegen/xai-grok-pager-render"
    fns = []
    for slug, fn, file, name in PORTS:
        themes = load(src / file)["themes"]
        theme = next(t for t in themes if t["name"] == name)
        port = Port(theme["style"])
        fns.append(rust_fn(fn, name, port))
        (render / "assets" / f"{slug}.tmTheme").write_text(tmtheme(name, port))
    body = "\n\n".join(fns)
    rs_text = (
        "//! Faithful ports of Zed themes. Generated by `scripts/themes/zed_port.py`; edit the script, not this file.\n"
        "//! Token mapping: docs/custom/THEMES.md.\n\n"
        "use ratatui::style::{Color, Modifier};\n\n"
        "use super::tokyonight::Theme;\n\n"
        "const fn rgb(r: u8, g: u8, b: u8) -> Color {\n    Color::Rgb(r, g, b)\n}\n\n"
        f"impl Theme {{\n{body}\n}}\n")
    out = render / "src/theme/zed_ports.rs"
    out.write_text(rs_text)
    # Match the repo's rustfmt output so regeneration leaves no diff; skip if rustfmt is absent.
    cmd = ["rustfmt", "--edition", "2024", str(out)]
    if (root / "rustfmt.toml").exists():
        cmd[1:1] = ["--config-path", str(root / "rustfmt.toml")]
    try:
        subprocess.run(cmd, check=True)
    except FileNotFoundError:
        pass


if __name__ == "__main__":
    main()
