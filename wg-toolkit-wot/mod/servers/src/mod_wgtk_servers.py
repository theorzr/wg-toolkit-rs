# -*- coding: utf-8 -*-
"""wg-toolkit for the client: our servers in the login screen's server list, each with
its address and public key, see `wgtk_servers.servers`, and a button on the login screen
opening their editor, see `wgtk_servers.ui`.

The client loads this script at its start, before the login screen: it hooks into the
client there.
"""

import BigWorld
from debug_utils import LOG_CURRENT_EXCEPTION
from gui.Scaleform.daapi.view.login.LoginView import LoginView
from predefined_hosts import _PreDefinedHostList, g_preDefinedHosts

from gui.mods.wgtk_servers.servers import WATCH_PERIOD, config_path, g_serverList, log
from gui.mods.wgtk_servers.ui import g_loginWindows


_read_script_config = _PreDefinedHostList.readScriptConfig
_populate_login = LoginView._populate
_dispose_login = LoginView._dispose


def _readScriptConfig(self, dataSection, userDataSection=None):
    """The client reading its servers: ours are added after."""
    loaded = self._isDataLoaded
    _read_script_config(self, dataSection, userDataSection)
    if not loaded and self._isDataLoaded:
        g_serverList.add(self)


def _populate(self):
    """The login screen shown: with the servers' button."""
    _populate_login(self)
    g_loginWindows.show()


def _dispose(self):
    """The login screen gone: with the mod's windows."""
    try:
        g_loginWindows.close()
    except Exception:
        LOG_CURRENT_EXCEPTION()
    _dispose_login(self)


_PreDefinedHostList.readScriptConfig = _readScriptConfig
LoginView._populate = _populate
LoginView._dispose = _dispose
if g_preDefinedHosts._isDataLoaded:
    g_serverList.add(g_preDefinedHosts)
BigWorld.callback(WATCH_PERIOD, g_serverList.watch)
log('Loaded', config_path())


def init():
    pass


def fini():
    pass
