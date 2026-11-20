# -*- coding: utf-8 -*-
"""wg-toolkit for the client: our servers' debug drawing in battle, whole and easier to
read, see `wgtk_debug.debug_draw`.

The client loads this script at its start: it hooks into the client's
`DebugDrawEntity` there.
"""

from debug_utils import LOG_CURRENT_EXCEPTION, LOG_NOTE

try:
    import gui.mods.wgtk_debug.debug_draw
    LOG_NOTE('[wgtk_debug] Loaded')
except Exception:
    LOG_CURRENT_EXCEPTION()


def init():
    pass


def fini():
    pass
