#!/usr/bin/env python3
"""Build the client mod `wgtk_debug_<version>.wotmod`, see `mod/wotmod.py`:

    python3 wg-toolkit-wot/mod/debug/build.py [--install <game folder>]
"""

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

from wotmod import Mod, main

MOD = Mod(
    dir=HERE,
    id='wgtk_debug',
    version='0.1.0',
    description="wg-toolkit for the client: our servers' debug drawing in battle, whole and easier to read.",
)


if __name__ == '__main__':
    main(MOD, __doc__)
