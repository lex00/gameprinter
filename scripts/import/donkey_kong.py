#!/usr/bin/env python3
"""One-way import of Donkey Kong from game-printer into the games repo.

    python3 -I -B scripts/import/donkey_kong.py GAME_PRINTER GAMES

Reads game-printer's folders and never writes there. Writes lex00/donkey-kong
to GAMES/donkey-kong, with Rivet Rodeo under open/, and one content package to
GAMES/paper. Scores and evidence stay behind; clips come along unchanged.
Running it again gives the same files.
"""
import hashlib
import shutil
import sys
from pathlib import Path

SKIP = {"evidence", "loop.toml", "AGENTS.md"}
PY_PATHS = [
    ('OUT = os.path.join(REPO, "templates", "rivet-rodeo")', 'OUT = os.path.join(REPO, "open")'),
    ('os.path.join(REPO, "templates", "donkey-kong-oe", "model.toml")', 'os.path.join(REPO, "model.toml")'),
]


def copy_tree(src: Path, dst: Path, skip=frozenset()) -> None:
    for p in sorted(src.rglob("*")):
        rel = p.relative_to(src)
        if rel.parts[0] in skip or p.is_dir():
            continue
        out = dst / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(p, out)


def replace(path: Path, pairs) -> None:
    text = path.read_text()
    for old, new in pairs:
        if old not in text:
            raise SystemExit(f"{path}: expected text not found: {old}")
        text = text.replace(old, new)
    path.write_text(text)


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def pin(text: str, key: str, start: int = 0) -> str:
    i = text.index(f'{key} = "', start) + len(key) + 4
    return text[i : text.index('"', i)]


def main() -> int:
    src, games = Path(sys.argv[1]), Path(sys.argv[2])
    if not (src / "templates/donkey-kong-oe/template.toml").is_file():
        raise SystemExit(f"{src} isn't game-printer")
    game = games / "donkey-kong"
    if game.exists():
        shutil.rmtree(game)
    copy_tree(src / "templates/donkey-kong-oe", game, SKIP)
    open_dir = game / "open"
    copy_tree(src / "templates/rivet-rodeo", open_dir, SKIP)
    for name, to in [
        ("donkey-kong-oe.toml", game / "assets"),
        ("donkey-kong-ju.toml", game / "assets"),
        ("rivet-rodeo.toml", open_dir / "assets"),
    ]:
        to.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src / "assets" / name, to / name)
    gen = open_dir / "generators"
    copy_tree(src / "scripts/rivet-rodeo", gen)

    replace(game / "template.toml", [
        ('asset_map = "../../assets/donkey-kong-oe.toml"', 'asset_map = "assets/donkey-kong-oe.toml"'),
        ('asset_map = "../../assets/donkey-kong-ju.toml"', 'asset_map = "assets/donkey-kong-ju.toml"'),
    ])
    replace(open_dir / "template.toml", [
        ('asset_map = "../../assets/rivet-rodeo.toml"', 'asset_map = "assets/rivet-rodeo.toml"'),
        ('from = "../donkey-kong-oe"', 'from = ".."'),
        ("python3 -I scripts/rivet-rodeo/stage.py", "python3 -I open/generators/stage.py"),
        ("$GP template derive templates/rivet-rodeo --write", "$GP template derive open --write"),
        ("$GP template build templates/rivet-rodeo --write", "$GP template build open --write"),
        ("$GP theme kit templates/rivet-rodeo", "$GP theme kit open"),
        ("$GP theme paint templates/rivet-rodeo -o templates/rivet-rodeo/themes/paper --logo-file scripts/rivet-rodeo/title.txt",
         "$GP theme paint open -o open/themes/paper --logo-file open/generators/title.txt"),
    ])
    replace(gen / "stage.py", PY_PATHS)
    replace(game / "ext/Cargo.toml", [
        ("rust-version.workspace = true", 'rust-version = "1.99"'),
        ("\nversion.workspace = true", '\nversion = "0.1.0"'),
        ("edition.workspace = true", 'edition = "2024"'),
        ("publish.workspace = true", "publish = false"),
        ("license.workspace = true", 'license = "Apache-2.0"'),
        ('gp-sim = { path = "../../../crates/gp-sim" }', 'gp-sim = { path = "../../../gameprinter/crates/gp-sim" }'),
        ('gp-cycles = { path = "../../../crates/gp-cycles" }', 'gp-cycles = { path = "../../../gameprinter/crates/gp-cycles" }'),
        ("[lints]\nworkspace = true\n", '[lints.rust]\nunsafe_code = "forbid"\n\n[workspace]\n'),
    ])

    tmpl = (game / "template.toml").read_text()
    ju_at = tmpl.index("[[loads]]")
    oe_map, ju_map = game / "assets/donkey-kong-oe.toml", game / "assets/donkey-kong-ju.toml"
    (game / "package.toml").write_text(f'''# lex00/donkey-kong: the ROM option and its open version, one game.
[package]
name = "lex00/donkey-kong"
version = "1.0.0"
kind = "game"
title = "Donkey Kong"
license = "Apache-2.0"
tier = "commercial"

[tool]
runtime-wit = "game-printer:runtime@0.4.0"
sim-api = "1"
gp = ">=0.1.0, <0.2.0"

[play]
files = ["model.toml", "mapping.toml", "profiles.toml", "assets/", "themes/paper/",
         "open/model.toml", "open/profiles.toml", "open/authored.toml", "open/themes/"]

[open]
title = "Rivet Rodeo"
model = "open/model.toml"
derive = "model.toml"
authored = "open/authored.toml"
baselines = "open/baselines/"

[rom]
how_to_get = """
Donkey Kong: Original Edition was sold for the Nintendo 3DS Virtual Console. This game reads
a dump of that release, or of a cartridge you own, made with a dumping tool you trust.
Without one, play Rivet Rodeo, this game's open version.
"""

[[rom.entry]]
id = "donkey-kong-oe"
role = "scored"
title = "Donkey Kong: Original Edition"
sha256 = "45440d8ef512b4660dce0c753e110004aef96fa30f2967f9260e946d808db07d"
platform = "nes"
mapper = 3
submapper = 2
prg_rom_bytes = 32768
chr_rom_bytes = 32768
board_status = "unverified"
asset_map = "assets/donkey-kong-oe.toml"
asset_map_sha256 = "{sha(oe_map)}"
lifted_sha256 = "{pin(tmpl, "lifted_sha256")}"

[[rom.entry]]
id = "donkey-kong-ju"
role = "play"
title = "Donkey Kong (JU)"
sha256 = "aa408f5a6b97c0d738e7e8b489a5617ad4a9ecdee2b05c4ee504210ce31b2825"
platform = "nes"
mapper = 0
prg_rom_bytes = 16384
chr_rom_bytes = 8192
board_status = "unverified"
asset_map = "assets/donkey-kong-ju.toml"
asset_map_sha256 = "{sha(ju_map)}"
levels = ["main", "m75", "m100"]
assets = {{ title_screens = "stage_screens", title_tiles = "tiles", title_sprite_tiles = "sprite_tiles" }}
fill_from = "open/"

[hooks]
crate = "ext"
models = ["donkey_kong_oe", "rivet_rodeo"]

[corpus]
name = "donkey-kong-oe-25m"
version = "6.0.0"
''')
    if pin(tmpl, "asset_map_sha256") != sha(oe_map) or pin(tmpl, "asset_map_sha256", ju_at) != sha(ju_map):
        raise SystemExit("an asset map doesn't match its template's pin")

    content = games / "paper"
    if content.exists():
        shutil.rmtree(content)
    copy_tree(src / "templates/rivet-rodeo/themes/paper", content / "theme")
    (content / "package.toml").write_text('''# lex00/paper: the paper look for Donkey Kong's open version.
[package]
name = "lex00/paper"
version = "1.0.0"
kind = "content"
title = "Paper"
license = "Apache-2.0"

[provides.theme]
spec = "theme@1"
for = { "lex00/donkey-kong" = "^1" }
files = ["theme/"]
''')
    print(f"imported {sum(1 for p in game.rglob('*') if p.is_file())} files to {game}")
    print(f"imported {sum(1 for p in content.rglob('*') if p.is_file())} files to {content}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
