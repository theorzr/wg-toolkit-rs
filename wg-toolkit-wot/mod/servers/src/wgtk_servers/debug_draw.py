# -*- coding: utf-8 -*-
"""Our servers' debug drawing, the client's `DebugDrawEntity`, made easier to read: its
texts drawn on the screen over where they are in the world, as the vehicles' markers
are, as big however far, opaque, in a small bold font, over a black shadow; and those of a
group named `entity:<id>:<height>` following that entity as the client moves it,
smoothly, `height` metres over it, while the client has it, else where the server put
them.

Every text hangs `TEXT_GAP` pixels under its place: the server puts them where its
vehicles stand, so that they are under them, never over their markers, which are drawn
over them as big as the camera's distance to them says, see the client's
`markerScaleSettings`.

A text is a root of the screen's GUI, moved over its place in the world by a
`GUI.MatrixShader` of a `GUI.WorldToClipMP`, which projects it there each frame, hidden
behind the camera. The client keeps its texts as a model, its motor and the text: the
model is still added to the space, for the client to delete, and the motor is an
`_Anchor`, which also aims the projection at its signal. A text deleted by the client,
its model, is taken off the screen after the client's update, see `_drop_unused`.

A text is the child of its shadow, the same text in black: a component draws itself,
then its children over it, so the shadow is always behind its text, and its shader
moves its children with it. The text is `SHADOW_OFFSET` pixels up and left of its
shadow. The server's text size is ignored: a text is as big as its font.
"""

import BigWorld
import GUI
import Math
import math_utils
from debug_utils import LOG_CURRENT_EXCEPTION
from DebugDrawEntity import DebugDrawEntity


# The font of the texts, bold, 12 pixels high, the same face as the default one.
FONT = 'default_smaller.font'
# The colour of a text's shadow, black, and how far the text is from it, up and left,
# in pixels.
SHADOW = (0, 0, 0, 255)
SHADOW_OFFSET = 1.0
# How far under its place a text hangs, in pixels, see the module.
TEXT_GAP = 8.0

_update = DebugDrawEntity._DebugDrawEntity__update
_create = DebugDrawEntity._DebugDrawEntity__create3DText
_leave_world = DebugDrawEntity.onLeaveWorld


class _Anchor(object):
    """The motor of a text's model, as the client sees it: setting its signal, where
    the text is in the world, also aims the text's projection on the screen at it."""

    def __init__(self, servo, projection):
        self.servo = servo
        self.projection = projection

    @property
    def signal(self):
        return self.servo.signal

    @signal.setter
    def signal(self, signal):
        self.servo.signal = signal
        self.projection.target = signal


def _followed(name):
    """The entity id and height of a group's name following an entity, else None."""
    parts = name.split(':')
    if len(parts) != 3 or parts[0] != 'entity':
        return None
    try:
        return int(parts[1]), float(parts[2])
    except ValueError:
        return None


def _follow(entity, texts, height):
    """Put these texts `height` metres over this entity, wherever the client moves it."""
    for _, motor, _ in texts:
        signal = Math.MatrixProduct()
        signal.a = math_utils.createTranslationMatrix(Math.Vector3(0.0, height, 0.0))
        signal.b = entity.matrix
        motor.signal = signal


def _text(colour):
    """A text of the debug drawing, empty, in this colour, see the module: as big as its
    font, centred under its top, on the screen."""
    text = GUI.Text('')
    text.widthMode = GUI.Simple.eSizeMode.PIXEL
    text.heightMode = GUI.Simple.eSizeMode.PIXEL
    text.horizontalPositionMode = GUI.Simple.ePositionMode.CLIP
    text.verticalPositionMode = GUI.Simple.ePositionMode.CLIP
    text.horizontalAnchor = GUI.Simple.eHAnchor.CENTER
    text.verticalAnchor = GUI.Simple.eVAnchor.TOP
    text.materialFX = GUI.Simple.eMaterialFX.BLEND
    text.multiline = True
    text.explicitSize = False
    text.filterType = GUI.Simple.eFilterType.POINT
    text.font = FONT
    text.colour = colour
    text.visible = True
    return text


def _new_text(spaceID):
    """A new text over its shadow, on the screen over a point of this space: its model,
    its motor, an `_Anchor`, and the text."""
    shadow = _text(SHADOW)
    label = _text((255, 255, 255, 255))
    # Pixels in clip space: a child is placed on the screen as its parent, not in it,
    # only its parent's shader moving it with its parent.
    width, height = GUI.screenResolution()
    pixel = (2.0 / width, 2.0 / height)
    shadow.position = (0.0, -TEXT_GAP * pixel[1], 1.0)
    label.position = (
        -SHADOW_OFFSET * pixel[0],
        (SHADOW_OFFSET - TEXT_GAP) * pixel[1],
        0.0,
    )
    shadow.addChild(label, 'label')

    projection = GUI.WorldToClipMP()
    try:
        projection.onlyFront = True
    except AttributeError:
        pass
    shader = GUI.MatrixShader()
    shader.blend = False
    shader.target = projection
    shadow.addShader(shader, 'world')
    GUI.addRoot(shadow)

    servo = BigWorld.Servo(Math.Matrix())
    model = BigWorld.Model('')
    model.addMotor(servo)
    BigWorld.addModel(model, spaceID)
    return model, _Anchor(servo, projection), label


def _create_3d_text(self, position, text, color, textSize):
    """The client's `__create3DText`, a text reused or made anew, over its shadow, see
    the module's documentation: the client's own, plain, if that fails."""
    try:
        return _create_projected(self, position, text, color)
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return _create(self, position, text, color, textSize)


def _create_projected(self, position, text, color):
    """A text reused or made anew, over its shadow, see `_create_3d_text`."""
    if self.reuse3DTexts:
        model, motor, label = self.reuse3DTexts.pop()
    else:
        model, motor, label = _new_text(self.spaceID)
        _roots(self)[id(label)] = label.parent
    label.parent.text = text
    label.text = text
    label.colour = color
    motor.signal = math_utils.createTranslationMatrix(position)
    return model, motor, label


def _roots(self):
    """The screen's roots of this entity's texts, by their text's id, see the module."""
    roots = getattr(self, '_wgtkRoots', None)
    if roots is None:
        roots = self._wgtkRoots = {}
    return roots


def _drop_unused(self):
    """Take off the screen the texts the client deleted, those it no longer has."""
    kept = set(id(label) for state in self.objectStates.values() for _, _, label in state['3Dtexts'])
    kept.update(id(label) for _, _, label in self.reuse3DTexts)
    roots = _roots(self)
    for key in [key for key in roots if key not in kept]:
        GUI.delRoot(roots.pop(key))


def _follow_entities(self):
    """The texts of each group following an entity, on it, see `_followed`."""
    for drawObject in self.drawObjects:
        followed = _followed(drawObject['name'])
        state = self.objectStates.get(drawObject['name'])
        if followed is None or state is None:
            continue
        entity = BigWorld.entities.get(followed[0])
        if entity is not None:
            _follow(entity, state['3Dtexts'], followed[1])


def _update_following(self):
    """The entity drawing what the server sent, then its texts deleted off the screen,
    and those left on the entities they follow, see `_follow_entities`."""
    _update(self)
    try:
        _drop_unused(self)
        _follow_entities(self)
    except Exception:
        LOG_CURRENT_EXCEPTION()


def _on_leave_world(self):
    """The entity leaving, and all its texts off the screen."""
    _leave_world(self)
    try:
        for root in _roots(self).values():
            GUI.delRoot(root)
        _roots(self).clear()
    except Exception:
        LOG_CURRENT_EXCEPTION()


DebugDrawEntity._DebugDrawEntity__update = _update_following
DebugDrawEntity._DebugDrawEntity__create3DText = _create_3d_text
DebugDrawEntity.onLeaveWorld = _on_leave_world
