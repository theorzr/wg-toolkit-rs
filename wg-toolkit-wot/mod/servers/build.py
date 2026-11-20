#!/usr/bin/env python3
"""Build the client mod `wgtk_servers_<version>.wotmod`, see `mod/wotmod.py`:

    python3 wg-toolkit-wot/mod/servers/build.py [--install <game folder>]
"""

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))

from wotmod import Mod, main

MOD = Mod(
    dir=HERE,
    id='wgtk_servers',
    version='0.3.0',
    description='wg-toolkit for the client: servers of our own in the login screen, each with its address and key.',
)


if __name__ == '__main__':
    main(MOD, __doc__)
