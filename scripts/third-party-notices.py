#!/usr/bin/env python3
"""Third-party notices for the shipped Linux binaries (task 15).

    scripts/third-party-notices.py [--out FILE] [--check FILE]

Walks `cargo metadata` for x86_64-unknown-linux-gnu from the two shipped
binaries (pigment-studio, pigment-cli) through normal and build dependencies
(dev-dependencies are never shipped), and writes one Markdown file with:

- a table of every crate: version, declared license, the license this
  project uses it under (for `A OR B` expressions, the first permissive
  choice), and its source;
- the fonts compiled into the studio and their licenses;
- the full text of every license, copyright and NOTICE file each crate
  ships, with identical texts printed once and listed against every crate
  that ships them.

It fails (exit 1) if a crate declares a license outside the permissive
allowlist or ships no license file, so a dependency change cannot slip a new
obligation into a package unnoticed. `--check FILE` regenerates and compares
with FILE instead of writing (exit 1 if stale). Stdlib only; the cargo
registry must hold the crate sources (any `cargo build --locked` fetches
them).
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

TARGET = "x86_64-unknown-linux-gnu"
SHIPPED = ["pigment-studio", "pigment-cli"]
# Licenses whose terms this project can meet by shipping this file. None of
# them restricts what users do with images the program makes.
PERMISSIVE = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unlicense",
    "Unicode-3.0",
    "BSL-1.0",
    "CC0-1.0",
    "MIT-0",
    # Font licenses (epaint_default_fonts): ship the text; the fonts may not
    # be sold on their own. Nothing about what the program makes.
    "OFL-1.1",
    "Ubuntu-font-1.0",
]
# The canonical Apache License 2.0 text, for crates that are available under
# it but ship no license file in their published package.
APACHE_TEXT = "packaging/linux/licenses/Apache-2.0.txt"
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense|copyright)", re.I)
# Fonts compiled into pigment-studio: (family, files, license, license
# files relative to the repository or to the named crate).
FONTS = [
    (
        "Atkinson Hyperlegible Next (Regular, SemiBold)",
        "crates/pigment-studio/fonts/AtkinsonHyperlegibleNext-*.ttf",
        "OFL-1.1",
        ("repo", "crates/pigment-studio/fonts/OFL.txt"),
    ),
    (
        "Atkinson Hyperlegible Mono (Regular)",
        "crates/pigment-studio/fonts/AtkinsonHyperlegibleMono-Regular.ttf",
        "OFL-1.1",
        ("repo", "crates/pigment-studio/fonts/OFL-mono.txt"),
    ),
    (
        "Ubuntu Light (egui default font)",
        "epaint_default_fonts: fonts/Ubuntu-Light.ttf",
        "Ubuntu Font Licence 1.0",
        ("epaint_default_fonts", "fonts/UFL.txt"),
    ),
    (
        "Hack Regular (egui default monospace)",
        "epaint_default_fonts: fonts/Hack-Regular.ttf",
        "MIT and Bitstream Vera",
        ("epaint_default_fonts", "fonts/Hack-Regular.txt"),
    ),
    (
        "Noto Emoji Regular (egui default fallback)",
        "epaint_default_fonts: fonts/NotoEmoji-Regular.ttf",
        "OFL-1.1",
        ("epaint_default_fonts", "fonts/OFL.txt"),
    ),
    (
        "emoji-icon-font (egui default fallback)",
        "epaint_default_fonts: fonts/emoji-icon-font.ttf",
        "MIT",
        ("epaint_default_fonts", "fonts/emoji-icon-font-mit-license.txt"),
    ),
]


def parse_spdx(expr):
    """An SPDX license expression as nested tuples: ("or", [...]),
    ("and", [...]) or ("id", "MIT") (a `WITH` exception stays in the id).
    The legacy `/` separator reads as OR."""
    toks = re.findall(r"\(|\)|[A-Za-z0-9.+-]+", expr.replace("/", " OR "))
    pos = 0

    def peek():
        return toks[pos] if pos < len(toks) else None

    def take():
        nonlocal pos
        pos += 1
        return toks[pos - 1]

    def atom():
        if peek() == "(":
            take()
            e = either()
            assert take() == ")", expr
            return e
        name = take()
        if peek() == "WITH":
            take()
            name += " WITH " + take()
        return ("id", name)

    def both():
        parts = [atom()]
        while peek() == "AND":
            take()
            parts.append(atom())
        return parts[0] if len(parts) == 1 else ("and", parts)

    def either():
        parts = [both()]
        while peek() == "OR":
            take()
            parts.append(both())
        return parts[0] if len(parts) == 1 else ("or", parts)

    e = either()
    assert pos == len(toks), expr
    return e


def choose(node, prefer=None):
    """The licenses (a list) to use `node` under, or None: every part of an
    AND, and for an OR the alternative containing `prefer` if any, else the
    first that is allowed."""
    kind, val = node
    if kind == "id":
        return [val] if val in PERMISSIVE else None
    if kind == "and":
        out = []
        for part in val:
            c = choose(part, prefer)
            if c is None:
                return None
            out += c
        return out
    options = [c for c in (choose(part, prefer) for part in val) if c is not None]
    for c in options:
        if prefer in c:
            return c
    return options[0] if options else None


def chosen_license(expr, prefer=None):
    """The allowed license expression a crate is used under, or None."""
    c = choose(parse_spdx(expr), prefer)
    return None if c is None else " AND ".join(c)


def shipped_packages(meta):
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    roots = [i for i in members if pkgs[i]["name"] in SHIPPED]
    assert len(roots) == len(SHIPPED), roots
    seen, stack = set(), list(roots)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k["kind"] in (None, "build") for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    return sorted(
        (pkgs[i] for i in seen if i not in members),
        key=lambda p: (p["name"], p["version"]),
    )


def license_files(pkg):
    root = os.path.dirname(pkg["manifest_path"])
    names = {n for n in os.listdir(root) if LICENSE_FILE.match(n)}
    for sub in ("LICENSES", "licenses"):
        d = os.path.join(root, sub)
        if os.path.isdir(d):
            names |= {os.path.join(sub, n) for n in os.listdir(d)}
    if pkg.get("license_file"):
        names.add(pkg["license_file"])
    out = []
    for n in sorted(names):
        path = os.path.join(root, n)
        if os.path.isfile(path):
            with open(path, encoding="utf-8", errors="replace") as f:
                out.append((n, f.read().replace("\r\n", "\n").strip() + "\n"))
    return out


def source_of(pkg):
    src = pkg.get("source") or ""
    if src.startswith("registry+"):
        return "crates.io"
    return src.split("+", 1)[0] or "local"


def generate(repo):
    meta = json.loads(
        subprocess.run(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--filter-platform",
                TARGET,
            ],
            cwd=repo,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    pkgs = shipped_packages(meta)
    by_name = {p["name"]: p for p in pkgs}
    problems = []
    rows = []
    texts = {}  # digest -> (text, [crate labels])
    with open(os.path.join(repo, APACHE_TEXT), encoding="utf-8") as f:
        apache = f.read().replace("\r\n", "\n").strip() + "\n"
    for p in pkgs:
        label = f'{p["name"]} {p["version"]}'
        lic = p.get("license") or ""
        files = license_files(p)
        # Without a license file, only a license whose text alone meets its
        # terms will do: Apache-2.0 (MIT needs the authors' copyright line).
        use = chosen_license(lic, None if files else "Apache-2.0") if lic else None
        if use is None:
            problems.append(f"{label}: license {lic or '(none declared)'} is not in the allowlist")
        elif not files:
            if "Apache-2.0" not in use.split(" AND "):
                problems.append(f"{label}: ships no license file and is not available under Apache-2.0")
            else:
                files = [("no license file in the published crate; Apache License 2.0", apache)]
        for name, text in files:
            key = hashlib.sha256(text.encode()).hexdigest()
            texts.setdefault(key, (text, []))[1].append(f"{label} ({name})")
        rows.append((p["name"], p["version"], lic, use or "?", source_of(p)))

    lines = [
        "# Third-party notices",
        "",
        "Pigment Prose is MIT-licensed (see `LICENSE`). The Linux binaries "
        "`pigment-studio` and `pigment-prose` also contain the third-party "
        "code and fonts below, used under the licenses shown. These notices "
        "concern the software only: nothing in them applies to images you "
        "make with it, which carry no watermark, attribution or other added "
        "content.",
        "",
        f"Generated by `scripts/third-party-notices.py` from `cargo metadata "
        f"--locked --filter-platform {TARGET}` for the shipped binaries "
        "(normal and build dependencies; development-only crates are not "
        "shipped and not listed).",
        "",
        f"## Crates ({len(rows)})",
        "",
        "| Crate | Version | Declared license | Used under | Source |",
        "| --- | --- | --- | --- | --- |",
    ]
    for name, ver, lic, use, src in rows:
        lines.append(f"| {name} | {ver} | {lic} | {use} | {src} |")
    lines += [
        "",
        "## Fonts compiled into pigment-studio",
        "",
        "They draw the studio's interface only. Exported paintings contain no text or glyphs.",
        "",
        "| Font | Files | License |",
        "| --- | --- | --- |",
    ]
    font_texts = []
    for family, files, lic, (where, rel) in FONTS:
        lines.append(f"| {family} | `{files}` | {lic} |")
        if where == "repo":
            path = os.path.join(repo, rel)
        elif where in by_name:
            path = os.path.join(os.path.dirname(by_name[where]["manifest_path"]), rel)
        else:
            problems.append(f"font {family}: crate {where} is not shipped")
            continue
        with open(path, encoding="utf-8", errors="replace") as f:
            font_texts.append((family, f.read().replace("\r\n", "\n").strip() + "\n"))
    lines += ["", "### Font license texts", ""]
    for family, text in font_texts:
        lines += [f"#### {family}", "", "```text", text.rstrip("\n"), "```", ""]
    lines += [
        "## License texts shipped by the crates",
        "",
        "Each text appears once; the list above it names every crate (and "
        "file) that ships it.",
        "",
    ]
    for key, (text, owners) in sorted(texts.items(), key=lambda kv: kv[1][1][0]):
        lines.append("### " + ", ".join(owners[:3]) + (f" and {len(owners) - 3} more" if len(owners) > 3 else ""))
        lines.append("")
        if len(owners) > 3:
            lines += ["Also shipped by: " + ", ".join(owners[3:]), ""]
        lines += ["```text", text.rstrip("\n").replace("```", "'''"), "```", ""]
    return "\n".join(lines), problems


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", help="write the notices here (default: stdout)")
    ap.add_argument("--check", help="compare with this file instead of writing")
    a = ap.parse_args()
    repo = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    text, problems = generate(repo)
    for p in problems:
        print(f"third-party-notices: {p}", file=sys.stderr)
    if problems:
        return 1
    if a.check:
        with open(a.check, encoding="utf-8") as f:
            if f.read() != text:
                print(
                    f"third-party-notices: {a.check} is stale; regenerate with "
                    f"scripts/third-party-notices.py --out {a.check}",
                    file=sys.stderr,
                )
                return 1
        print(f"third-party-notices: {a.check} is current")
        return 0
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
