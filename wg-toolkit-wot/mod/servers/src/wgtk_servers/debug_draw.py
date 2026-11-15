# -*- coding: utf-8 -*-
"""Our servers' debug drawing, the client's `DebugDrawEntity`, made easier to read: its
texts drawn opaque, blended rather than added, in a heavier font, on a semi-opaque
black square, and those of a group named `entity:<id>:<height>` following that entity
as the client moves it, smoothly, `height` metres over it, while the client has it,
else where the server put them.

A text is the child of its square, a `GUI.Simple` component of a plain white texture,
coloured: a component draws itself, then its children over it, so the square is always
behind its text. Both are
sized and placed in the same coordinates, the attachment's, `LEGACY`: the square is the
text's size, as the text measures itself once laid out, and `PADDING` more around.
"""

import BigWorld
import GUI
import Math
import math_utils
from debug_utils import LOG_CURRENT_EXCEPTION
from DebugDrawEntity import DebugDrawEntity


# The font of the texts, heavier than the default one.
FONT = 'default_medium.font'
# The texture of a text's square, plain white, and its colour, black, 160 of 255
# opaque: a component without a texture isn't drawn, only its children.
BACKGROUND_TEXTURE = 'system/maps/col_white.dds'
BACKGROUND = (0, 0, 0, 160)
# How much larger than its text a square is on each side, of the text's size.
PADDING = 0.06

_update = DebugDrawEntity._DebugDrawEntity__update
_create = DebugDrawEntity._DebugDrawEntity__create3DText


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


def _legacy(component):
    """Size and place a component in the attachment's coordinates, see the module."""
    component.widthMode = GUI.Simple.eSizeMode.LEGACY
    component.heightMode = GUI.Simple.eSizeMode.LEGACY
    component.horizontalPositionMode = GUI.Simple.ePositionMode.LEGACY
    component.verticalPositionMode = GUI.Simple.ePositionMode.LEGACY
    component.horizontalAnchor = GUI.Simple.eHAnchor.CENTER
    component.verticalAnchor = GUI.Simple.eVAnchor.BOTTOM
    component.materialFX = GUI.Simple.eMaterialFX.BLEND


def _new_text(spaceID):
    """A new text on its square, in the world of this space: its model, the motor
    placing it, and the text."""
    background = GUI.Simple(BACKGROUND_TEXTURE)
    _legacy(background)
    background.colour = BACKGROUND
    label = GUI.Text('')
    _legacy(label)
    label.multiline = True
    label.explicitSize = True
    label.filterType = GUI.Simple.eFilterType.LINEAR
    label.font = FONT
    background.addChild(label, 'label')
    attachment = GUI.Attachment()
    attachment.component = background
    attachment.faceCamera = True
    motor = BigWorld.Servo(Math.Matrix())
    model = BigWorld.Model('')
    model.addMotor(motor)
    BigWorld.addModel(model, spaceID)
    model.root.attach(attachment)
    background.visible = True
    label.visible = True
    return model, motor, label


def _create_3d_text(self, position, text, color, textSize):
    """The client's `__create3DText`, a text reused or made anew, on its square, see the
    module's documentation: the client's own, plain, if that fails."""
    try:
        return _create_squared(self, position, text, color, textSize)
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return _create(self, position, text, color, textSize)


def _create_squared(self, position, text, color, textSize):
    """A text reused or made anew, on its square, see `_create_3d_text`."""
    if self.reuse3DTexts:
        model, motor, label = self.reuse3DTexts.pop()
    else:
        model, motor, label = _new_text(self.spaceID)
    label.text = text
    label.size = (0, textSize)
    label.colour = color
    # Laid out, the text measures itself: its square is as large, and more around.
    padding = PADDING * textSize
    background = label.parent
    background.size = (label.width + 2.0 * padding, label.height + 2.0 * padding)
    background.position = (0.0, -padding, 0.0)
    motor.signal = math_utils.createTranslationMatrix(position)
    return model, motor, label


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
    """The entity drawing what the server sent, then its texts on the entities they
    follow, see `_follow_entities`."""
    _update(self)
    try:
        _follow_entities(self)
    except Exception:
        LOG_CURRENT_EXCEPTION()


DebugDrawEntity._DebugDrawEntity__update = _update_following
DebugDrawEntity._DebugDrawEntity__create3DText = _create_3d_text
