# -*- coding: utf-8 -*-
"""The mod's windows on the login screen, Gameface views of its own, see
`res/gui/gen_resources/wgtk_servers.json`: a button in a corner, which opens the servers'
editor, listing them, to add, change or remove one, each with its address and key.

Each page sizes its window two frames after it is ready, as the game's own pages do, the
client ignoring a size given before, then giving up after 3 s at a default size. Both
windows are made as the login screen shows, hidden: the button shows once sized, and the
editor is shown and hidden as asked, never made again, so that it opens at once: a
window shows itself once ready, ours only when wanted, see [`ModWindow`].

Each view's page reads its model, `window.model`, and calls its commands, which these
views handle: the servers file changes, and the login screen with it.
"""

from debug_utils import LOG_CURRENT_EXCEPTION
from frameworks.wulf import Array, PositionAnchor, ViewModel, ViewSettings, WindowFlags
from gui.impl.gen import R
from gui.impl.pub import ViewImpl
from gui.impl.pub.window_impl import WindowImpl

from gui.mods.wgtk_servers.servers import log, read_servers, remove_server, save_server


# How far from the screen's bottom right corner the button is, in pixels.
BUTTON_MARGIN = 24


# -- Models --

class ServerModel(ViewModel):
    """A server, as the editor lists it."""

    def __init__(self, properties=4, commands=0):
        super(ServerModel, self).__init__(properties=properties, commands=commands)

    def setShortName(self, value):
        self._setString(0, value)

    def setName(self, value):
        self._setString(1, value)

    def setAddress(self, value):
        self._setString(2, value)

    def setKey(self, value):
        self._setString(3, value)

    def _initialize(self):
        super(ServerModel, self)._initialize()
        self._addStringProperty('shortName', '')
        # Empty where the server gives none, its short name then.
        self._addStringProperty('name', '')
        self._addStringProperty('address', '')
        self._addStringProperty('key', '')


class EditorModel(ViewModel):
    """The editor: the servers, and what its page asks of them, a server by its index
    in the list, a new one below 0."""
    __slots__ = ('onSave', 'onRemove', 'onClose', 'onLog')

    def __init__(self, properties=1, commands=4):
        super(EditorModel, self).__init__(properties=properties, commands=commands)

    def getServers(self):
        return self._getArray(0)

    def _initialize(self):
        super(EditorModel, self)._initialize()
        self._addArrayProperty('servers', Array())
        # `{index, shortName, name, address, key}`.
        self.onSave = self._addCommand('onSave')
        # `{index}`.
        self.onRemove = self._addCommand('onRemove')
        self.onClose = self._addCommand('onClose')
        # `{message}`, of the page, for the client's log.
        self.onLog = self._addCommand('onLog')


class ButtonModel(ViewModel):
    """The button: clicked."""
    __slots__ = ('onClick',)

    def __init__(self, properties=0, commands=1):
        super(ButtonModel, self).__init__(properties=properties, commands=commands)

    def _initialize(self):
        super(ButtonModel, self)._initialize()
        self.onClick = self._addCommand('onClick')


# -- Views --

class EditorView(ViewImpl):
    """The servers' editor, see [`EditorModel`]."""

    def __init__(self):
        settings = ViewSettings(R.mods.wgtk_servers.views.ServersEditor(), model=EditorModel())
        super(EditorView, self).__init__(settings)

    @property
    def viewModel(self):
        return self.getViewModel()

    def _initialize(self, *args, **kwargs):
        super(EditorView, self)._initialize(*args, **kwargs)
        self.viewModel.onSave += self.__onSave
        self.viewModel.onRemove += self.__onRemove
        self.viewModel.onClose += self.__onClose
        self.viewModel.onLog += self.__onLog

    def _finalize(self):
        self.viewModel.onSave -= self.__onSave
        self.viewModel.onRemove -= self.__onRemove
        self.viewModel.onClose -= self.__onClose
        self.viewModel.onLog -= self.__onLog
        g_loginWindows.editor = None
        super(EditorView, self)._finalize()

    def _onLoading(self, *args, **kwargs):
        super(EditorView, self)._onLoading(*args, **kwargs)
        self.refresh()

    def refresh(self):
        """Show the servers of the servers file."""
        with self.viewModel.transaction() as model:
            servers = model.getServers()
            servers.clear()
            for server in read_servers():
                item = ServerModel()
                item.setShortName(server.get('short_name') or server.get('name', ''))
                item.setName(server.get('name', '') if 'short_name' in server else '')
                item.setAddress(server.get('address', ''))
                item.setKey(server.get('public_key', ''))
                servers.addViewModel(item)
            servers.invalidate()

    def __onSave(self, args):
        try:
            save_server(
                int(args.get('index', -1)),
                args.get('shortName', ''),
                args.get('name', ''),
                args.get('address', ''),
                args.get('key', ''),
            )
            self.refresh()
        except Exception:
            LOG_CURRENT_EXCEPTION()

    def __onRemove(self, args):
        try:
            remove_server(int(args.get('index', -1)))
            self.refresh()
        except Exception:
            LOG_CURRENT_EXCEPTION()

    def __onClose(self, *args):
        g_loginWindows.hide_editor()

    def __onLog(self, args):
        log('The editor page said', args.get('message', ''))


class ButtonView(ViewImpl):
    """The button opening the servers' editor."""

    def __init__(self):
        settings = ViewSettings(R.mods.wgtk_servers.views.ServersButton(), model=ButtonModel())
        super(ButtonView, self).__init__(settings)

    @property
    def viewModel(self):
        return self.getViewModel()

    def _initialize(self, *args, **kwargs):
        super(ButtonView, self)._initialize(*args, **kwargs)
        self.viewModel.onClick += self.__onClick

    def _finalize(self):
        self.viewModel.onClick -= self.__onClick
        g_loginWindows.button = None
        super(ButtonView, self)._finalize()

    def __onClick(self, *args):
        g_loginWindows.show_editor()


# -- Windows --

class ModWindow(WindowImpl):
    """A window of the mod, without frame, above the login screen, not holding it,
    hidden until wanted: a window shows itself once ready, ours only if wanted then. It
    is placed once ready and as its size changes, by `place`, a move before being ready
    being lost."""
    __slots__ = ('__wanted', '__focus', '__place')

    def __init__(self, content, place):
        # As the game's own offer banner: its layer the one of its flags.
        super(ModWindow, self).__init__(WindowFlags.WINDOW, content=content, parent=None, areaID=R.areas.default())
        self.__wanted = False
        self.__focus = False
        self.__place = place
        self.onSizeChanged += self.__onSizeChanged

    def want(self, shown, focus=True):
        """Show or hide it, once ready if it isn't yet."""
        self.__wanted = shown
        self.__focus = focus
        if not self.isReady or self.isHidden() != shown:
            return
        if shown:
            self.show(focus)
            self.__place(self)
        else:
            self.hide()

    def _onReady(self):
        if self.__wanted:
            self.show(self.__focus)
        self.__place(self)

    def _finalize(self):
        self.onSizeChanged -= self.__onSizeChanged
        super(ModWindow, self)._finalize()

    def __onSizeChanged(self, *args):
        if self.isReady:
            self.__place(self)


class LoginWindows(object):
    """The mod's windows on the login screen, made as it shows, see the module's
    documentation: the button, shown once ready, and the editor, once asked."""

    def __init__(self):
        self.button = None
        self.editor = None

    def show(self):
        """Make both windows, the button shown once ready, the editor hidden."""
        if self.button is not None:
            return
        try:
            self.button = self.__make(ButtonView(), self.__placeButton)
            self.editor = self.__make(EditorView(), self.__placeEditor)
            self.button.want(True, focus=False)
            log('Made the servers windows')
        except Exception:
            LOG_CURRENT_EXCEPTION()

    def show_editor(self):
        if self.editor is None or not self.editor.isHidden():
            return
        if self.editor.isReady:
            self.editor.content.refresh()
        self.editor.want(True)
        log('Opened the servers editor')

    def hide_editor(self):
        if self.editor is not None:
            self.editor.want(False)

    def close(self):
        """Close both, the login screen gone."""
        windows = (self.editor, self.button)
        self.button = None
        self.editor = None
        for window in windows:
            if window is not None:
                try:
                    window.destroy()
                except Exception:
                    LOG_CURRENT_EXCEPTION()

    @staticmethod
    def __make(view, place):
        """A window of the view, hidden until shown, placed by `place`, its page sizing
        it once laid out."""
        window = ModWindow(view, place)
        window.load()
        return window

    @staticmethod
    def __placeButton(window):
        """Place the button in the screen's bottom right corner."""
        moved = window.move(BUTTON_MARGIN, BUTTON_MARGIN, PositionAnchor.RIGHT, PositionAnchor.BOTTOM)
        log('Placed the servers button', moved)

    @staticmethod
    def __placeEditor(window):
        """Place the editor in the screen's middle."""
        window.center()


g_loginWindows = LoginWindows()
