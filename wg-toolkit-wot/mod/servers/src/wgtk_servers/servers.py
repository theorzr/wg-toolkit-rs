# -*- coding: utf-8 -*-
"""Our servers, each with its address and public key, kept in
`res_mods/wgtk_servers/servers.json` in the game's folder, out of the game version's own
`res_mods` folder so that it outlives the game's updates:

    {"servers": [
        {"short_name": "Home", "address": "192.168.1.10:20016"},
        {"short_name": "EU1", "name": "Friend EU1", "address": "203.0.113.7:20016", "public_key": "-----BEGIN PUBLIC KEY-----\n..."}
    ]}

As the client's own, in `scripts_config.xml`: a server has a short name, and a name, the
login screen's, its short name if none.

A server's key is written in `res_mods/wgtk_servers/keys`, which the client reads the key of
the server it logs into from, by a resource path out of its resources, `../`. A server
without one has none: the client logs into it without encrypting its login.

The client reads its server list once, `_PreDefinedHostList.readScriptConfig`, from
`scripts_config.xml` and the player's preferences: ours are added after, see
[`ServerList`], and follow the servers file as it changes.
"""

import json
import os
import re

import BigWorld
import ResMgr
import constants
from debug_utils import LOG_CURRENT_EXCEPTION, LOG_NOTE, LOG_WARNING
from helpers import dependency
from predefined_hosts import g_preDefinedHosts
from skeletons.gui.login_manager import ILoginManager


# The mod's folder, from the game's folder, and as a resource path, the resources
# being the game version's `res_mods` folder; the servers file and the keys in it.
DATA_DIR = 'res_mods/wgtk_servers'
DATA_RES = '../res_mods/wgtk_servers'
CONFIG_NAME = 'servers.json'
KEYS_DIR = 'keys'
# How often the servers file is looked at for changes, in seconds.
WATCH_PERIOD = 2.0


def log(message, *args):
    LOG_NOTE('[wgtk_servers] ' + message, *args)


# -- The servers file --

def data_dir():
    """The mod's folder, made if missing: the client runs from the game's folder."""
    path = os.path.abspath(DATA_DIR)
    if not os.path.isdir(path):
        os.makedirs(path)
    return path


def config_path():
    return os.path.join(data_dir(), CONFIG_NAME)


def _modified(path):
    """When a file was last changed, none if it is missing."""
    try:
        return os.path.getmtime(path)
    except OSError:
        return None


def read_servers():
    """The servers of the servers file, none if it is missing or broken; an empty one is
    written where it is missing, for the player to fill."""
    path = config_path()
    if not os.path.isfile(path):
        write_servers([])
        return []
    try:
        with open(path) as config:
            servers = json.load(config).get('servers', [])
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return []
    log('Read the servers file', path, len(servers))
    return servers


def write_servers(servers):
    """Write the servers file, then show its servers on the login screen."""
    path = config_path()
    try:
        with open(path, 'w') as config:
            json.dump({'servers': servers}, config, indent=4)
        log('Wrote the servers file', path, len(servers))
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return
    g_serverList.reload(g_preDefinedHosts)


def save_server(index, short_name, name, address, key):
    """Change the server at this index of the servers file, or add one where there is
    none, of an index below 0; its short name its address if empty, its name and key
    left out if empty."""
    servers = read_servers()
    server = {
        'short_name': short_name.strip() or address.strip(),
        'address': address.strip(),
    }
    if name.strip():
        server['name'] = name.strip()
    if key.strip():
        server['public_key'] = key.strip()
    if 0 <= index < len(servers):
        servers[index] = server
    else:
        servers.append(server)
    write_servers(servers)


def remove_server(index):
    """Remove the server at this index of the servers file."""
    servers = read_servers()
    if 0 <= index < len(servers):
        del servers[index]
        write_servers(servers)


# -- Keys --

def _slug(name):
    return re.sub(r'[^a-z0-9]+', '_', name.lower()).strip('_') or 'server'


def key_path(server):
    """The resource path of a server's key, written in the mod's folder, none if it
    gives none or it can't be written."""
    key = server.get('public_key')
    if not key:
        return None
    name = '%s.pubkey' % _slug(server.get('short_name') or server.get('name') or server['address'])
    file_path = os.path.join(data_dir(), KEYS_DIR, name)
    try:
        if not os.path.isdir(os.path.dirname(file_path)):
            os.makedirs(os.path.dirname(file_path))
        with open(file_path, 'w') as key_file:
            key_file.write(key.strip() + '\n')
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return None
    path = '%s/%s/%s' % (DATA_RES, KEYS_DIR, name)
    log('Wrote a server key', file_path, 'seen by the client:', ResMgr.isFile(path))
    return path


# -- The server list --

class ServerList(object):
    """The servers this mod adds to the client's list, after those it had: they are
    taken out and added again as the servers file changes."""

    def __init__(self):
        # How many servers the client listed itself, the first ones.
        self.own = None
        self.modified = None

    def add(self, hosts):
        """Add the servers of the servers file, those of an address already listed left
        out."""
        if self.own is None:
            self.own = len(hosts._hosts)
        self.modified = _modified(config_path())
        for server in read_servers():
            try:
                address = server['address'].strip()
                short_name = server.get('short_name') or server.get('name') or address
                name = server.get('name') or short_name
                if address in hosts._urlMap:
                    LOG_WARNING('[wgtk_servers] Left out a server of an address already listed', name, address)
                    continue
                item = hosts._makeHostItem(name, short_name, address, keyPath=key_path(server), peripheryID=constants.STANDALONE_CLUSTER_ID)
                hosts._urlMap[item.url] = len(hosts._hosts)
                hosts._nameMap[item.name] = len(hosts._hosts)
                hosts._hosts.append(item)
                log('Added a server', short_name, name, address)
            except Exception:
                LOG_CURRENT_EXCEPTION()

    def reload(self, hosts):
        """Take out the servers added, add those of the servers file again, and show them
        on the login screen."""
        if self.own is None:
            return
        for url, index in hosts._urlMap.items():
            if index >= self.own:
                del hosts._urlMap[url]
        for name, index in hosts._nameMap.items():
            if index >= self.own:
                del hosts._nameMap[name]
        del hosts._hosts[self.own:]
        self.add(hosts)
        # The login screen shows the list of the login manager's servers, made again
        # from the client's.
        servers = dependency.instance(ILoginManager).servers
        if servers is None:
            log('Reloaded the servers, without a login screen')
            return
        servers.updateServerList()
        servers.onServersStatusChanged(servers.serverList)
        log('Reloaded the servers', [server['label'] for server in servers.serverList])

    def watch(self):
        """Reload the servers whenever the servers file changes, looked at every
        `WATCH_PERIOD`."""
        try:
            if self.own is not None and _modified(config_path()) != self.modified:
                self.reload(g_preDefinedHosts)
        except Exception:
            LOG_CURRENT_EXCEPTION()
        BigWorld.callback(WATCH_PERIOD, self.watch)


g_serverList = ServerList()
