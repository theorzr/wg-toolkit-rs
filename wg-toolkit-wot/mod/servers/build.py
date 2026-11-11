#!/usr/bin/env python3
"""Build the client mod, `wgtk_servers_<version>.wotmod`: its scripts compiled by Python
2.7, which the client runs, and its resources, in a zip without compression, as the
client reads mods.

    python3 wg-toolkit-wot/mod/servers/build.py [--install <game folder>]

With `--install`, the mod is copied into the game's `mods/<version>` folder, its version
read from the game's `version.xml`. Python 2.7 runs as `micromamba run -n py27 python`,
or as the `WGTK_PY27` variable says.
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
from pathlib import Path

HERE = Path(__file__).resolve().parent
MOD_ID = 'wgtk_servers'
MOD_VERSION = '0.1.0'
MOD_DESCRIPTION = 'wg-toolkit for the client: servers of our own in the login screen, each with its address and key.'

# Where the client loads mods' scripts from, in a mod's `res`.
SCRIPTS_DIR = 'res/scripts/client/gui/mods'


def compile_script(source: Path, target: Path, name: str) -> None:
    """Compile a script for the client, by Python 2.7, as it is named in the client,
    from its scripts' folder."""
    python = shlex.split(os.environ.get('WGTK_PY27', 'micromamba run -n py27 python'))
    code = 'import py_compile, sys; py_compile.compile(sys.argv[1], cfile=sys.argv[2], dfile=sys.argv[3], doraise=True)'
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([*python, '-c', code, str(source), str(target), f'scripts/client/gui/mods/{name}'], check=True)


def meta_xml() -> str:
    return (
        '<root>\n'
        f'    <id>{MOD_ID}</id>\n'
        f'    <version>{MOD_VERSION}</version>\n'
        f'    <name>{MOD_ID}</name>\n'
        f'    <description>{MOD_DESCRIPTION}</description>\n'
        '</root>\n'
    )


def build(out_dir: Path) -> Path:
    """Build the mod into `out_dir`, its path."""
    out_dir.mkdir(parents=True, exist_ok=True)
    wotmod = out_dir / f'{MOD_ID}_{MOD_VERSION}.wotmod'
    with tempfile.TemporaryDirectory() as tmp:

        # The scripts, the mod's own `mod_*` and its packages.
        files = {}
        for source in sorted((HERE / 'src').rglob('*.py')):
            name = source.relative_to(HERE / 'src').with_suffix('.pyc').as_posix()
            compiled = Path(tmp) / name
            compile_script(source, compiled, name[:-1])
            files[f'{SCRIPTS_DIR}/{name}'] = compiled
        # Resources besides, if any.
        for resource in sorted((HERE / 'res').rglob('*')):
            if resource.is_file():
                files['res/' + resource.relative_to(HERE / 'res').as_posix()] = resource

        # The client wants every folder listed, and nothing compressed.
        with zipfile.ZipFile(wotmod, 'w', zipfile.ZIP_STORED) as archive:
            archive.writestr('meta.xml', meta_xml())
            folders = sorted({'/'.join(name.split('/')[:depth]) + '/' for name in files for depth in range(1, name.count('/') + 1)})
            for folder in folders:
                archive.writestr(zipfile.ZipInfo(folder), '')
            for name, path in sorted(files.items()):
                archive.write(path, name)

    return wotmod


def game_version(game_dir: Path) -> str:
    """The game's version, as its `mods` folders are named: `2.4.1.1`."""
    text = (game_dir / 'version.xml').read_text(errors='replace')
    found = re.search(r'<version>\s*v\.([\d.]+)', text)
    if found is None:
        sys.exit(f'Failed to read the game version from {game_dir / "version.xml"}')
    return found.group(1)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--install', type=Path, metavar='GAME_DIR', help="copy the mod into the game's mods folder")
    args = parser.parse_args()

    wotmod = build(HERE / 'build')
    print(f'Built {wotmod}')
    if args.install is not None:
        target = args.install / 'mods' / game_version(args.install)
        target.mkdir(parents=True, exist_ok=True)
        shutil.copy2(wotmod, target / wotmod.name)
        print(f'Installed {target / wotmod.name}')


if __name__ == '__main__':
    main()
