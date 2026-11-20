"""Build a client mod of ours, `<id>_<version>.wotmod`, from its folder under `mod/`: its
scripts, `src/**.py`, compiled by Python 2.7, which the client runs, and its resources,
`res/**`, if any, in a zip without compression, as the client reads mods. Each mod's own
`build.py` gives its id, version and description, see [`Mod`].

With `--install`, the mod is copied into the game's `mods/<version>` folder, its version
read from the game's `version.xml`, in place of its other versions there. Python 2.7
runs as `micromamba run -n py27 python`, or as the `WGTK_PY27` variable says.
"""

import argparse
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path

# Where the client loads mods' scripts from, in a mod's `res`.
SCRIPTS_DIR = 'res/scripts/client/gui/mods'


@dataclass
class Mod:
    """A mod: its folder, holding `src` and `res`, and as its `meta.xml` has it, its
    id, its version and its description."""
    dir: Path
    id: str
    version: str
    description: str


def compile_script(source: Path, target: Path, name: str) -> None:
    """Compile a script for the client, by Python 2.7, as it is named in the client,
    from its scripts' folder."""
    python = shlex.split(os.environ.get('WGTK_PY27', 'micromamba run -n py27 python'))
    code = 'import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], dfile=sys.argv[3], doraise=True)'
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([*python, '-c', code, str(source), str(target), f'scripts/client/gui/mods/{name}'], check=True)


def meta_xml(mod: Mod) -> str:
    return (
        '<root>\n'
        f'    <id>{mod.id}</id>\n'
        f'    <version>{mod.version}</version>\n'
        f'    <name>{mod.id}</name>\n'
        f'    <description>{mod.description}</description>\n'
        '</root>\n'
    )


def build(mod: Mod) -> Path:
    """Build the mod into its `build` folder, its path."""
    out_dir = mod.dir / 'build'
    out_dir.mkdir(parents=True, exist_ok=True)
    wotmod = out_dir / f'{mod.id}_{mod.version}.wotmod'
    with tempfile.TemporaryDirectory() as tmp:

        # The scripts, the mod's own `mod_*` and its packages.
        files = {}
        for source in sorted((mod.dir / 'src').rglob('*.py')):
            name = source.relative_to(mod.dir / 'src').with_suffix('.pyc').as_posix()
            compiled = Path(tmp) / name
            compile_script(source, compiled, name[:-1])
            files[f'{SCRIPTS_DIR}/{name}'] = compiled
        # Resources besides, if any.
        for resource in sorted((mod.dir / 'res').rglob('*')):
            if resource.is_file():
                files['res/' + resource.relative_to(mod.dir / 'res').as_posix()] = resource

        # The client wants every folder listed, and nothing compressed.
        with zipfile.ZipFile(wotmod, 'w', zipfile.ZIP_STORED) as archive:
            archive.writestr('meta.xml', meta_xml(mod))
            folders = sorted({'/'.join(name.split('/')[:depth]) + '/' for name in files for depth in range(1, name.count('/') + 1)})
            for folder in folders:
                archive.writestr(zipfile.ZipInfo(folder), '')
            for name, path in sorted(files.items()):
                archive.write(path, name)

    # Its older builds go.
    for old in out_dir.glob(f'{mod.id}_*.wotmod'):
        if old != wotmod:
            old.unlink()
    return wotmod


def game_version(game_dir: Path) -> str:
    """The game's version, as its `mods` folders are named: `2.4.1.1`."""
    text = (game_dir / 'version.xml').read_text(errors='replace')
    found = re.search(r'<version>\s*v\.([\d.]+)', text)
    if found is None:
        sys.exit(f'Failed to read the game version from {game_dir / "version.xml"}')
    return found.group(1)


def install(mod: Mod, wotmod: Path, game_dir: Path) -> None:
    """Copy a mod built into the game's mods folder, in place of its other versions,
    which the game would load too."""
    target = game_dir / 'mods' / game_version(game_dir)
    target.mkdir(parents=True, exist_ok=True)
    for old in target.glob(f'{mod.id}_*.wotmod'):
        if old.name != wotmod.name:
            old.unlink()
            print(f'Removed {old}')
    shutil.copy2(wotmod, target / wotmod.name)
    print(f'Installed {target / wotmod.name}')


def main(mod: Mod, description: str) -> None:
    """Build a mod, and install it if asked, as its `build.py` is run."""
    parser = argparse.ArgumentParser(description=description, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--install', type=Path, metavar='GAME_DIR', help="copy the mod into the game's mods folder")
    args = parser.parse_args()

    wotmod = build(mod)
    print(f'Built {wotmod}')
    if args.install is not None:
        install(mod, wotmod, args.install)
