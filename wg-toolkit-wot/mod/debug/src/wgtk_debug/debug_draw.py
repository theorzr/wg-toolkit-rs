# -*- coding: utf-8 -*-
"""Our servers' debug drawing, the client's `DebugDrawEntity`, made whole and easier to
read: its spheres and boxes drawn with models the game has, its own missing; its lines
drawn `LINE_THICKNESS` wide, opaque, of a colour, each frame, see `_draw_lines`; its
texts drawn on the screen over where they are in the world, as the vehicles' markers
are, as big however far, opaque, in a small bold font, over a black shadow; and a
group named `entity:<id>:<channel>` drawn in the frame of that entity, x right, y up,
z ahead, following it as the client moves and turns it, smoothly, while the client has
it, hidden while it doesn't.

The models, `SPHERE_MODEL` and `BOX_MODEL`, are white, half transparent: the client
scales the sphere by its radius, and the box to its size under where the server puts
it, the box being a unit cube around its middle.

A line's width is its colour, red, green and blue, packed as `0xRRGGBB`: the client's
`DebugDrawer` draws it, seen through the map, each frame its entity is in the world,
the client's own model of it left empty. Without `DebugDrawer`, the client draws it as
its white box, `LINE_WIDTH` wide. A line has two points: the client would join more
with spheres as big as its width.

A group follows its entity as each of its models and texts is placed by a matrix
product: where the client put it, in the entity's frame, then the entity's matrix, see
`_follow_entities`.

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

try:
    import DebugDrawer
except ImportError:
    DebugDrawer = None


# The font of the texts, bold, 12 pixels high, the same face as the default one.
FONT = 'default_smaller.font'
# The colour of a text's shadow, black, and how far the text is from it, up and left,
# in pixels.
SHADOW = (0, 0, 0, 255)
SHADOW_OFFSET = 1.0
# How far under its place a text hangs, in pixels, see the module.
TEXT_GAP = 8.0
# The models of the spheres and the boxes, see the module.
SPHERE_MODEL = 'objects/misc/bbox/sphere1.model'
BOX_MODEL = 'objects/misc/bbox/unit_cube_1m_proxy.model'
# How wide a line is drawn as a box, without `DebugDrawer`, in metres.
LINE_WIDTH = 0.05
# The opaque alpha of a line's colour, as `DebugDrawer` takes it, `0xAARRGGBB`.
OPAQUE = 0xFF000000
# How wide `DebugDrawer` draws a line, in metres.
LINE_THICKNESS = 0.1

_update = DebugDrawEntity._DebugDrawEntity__update
_create = DebugDrawEntity._DebugDrawEntity__create3DText
_create_line = DebugDrawEntity._DebugDrawEntity__createDirectedLine
_enter_world = DebugDrawEntity.onEnterWorld
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
    """The entity id of a group's name following an entity, else None."""
    parts = name.split(':')
    if len(parts) < 2 or parts[0] != 'entity':
        return None
    try:
        return int(parts[1])
    except ValueError:
        return None


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


def _create_directed_line(self, pointA, pointB, width):
    """The client's `__createDirectedLine`: an empty model, the line drawn each frame,
    see `_draw_lines`; without `DebugDrawer`, its box, `LINE_WIDTH` wide."""
    if DebugDrawer is None:
        return _create_line(self, pointA, pointB, LINE_WIDTH)
    model, motor = self._DebugDrawEntity__getModel('')
    return '', model, motor


def _draw_lines(entity_id):
    """Draw the lines of the debug drawing entity of this id this frame, each in its
    colour, those of a group following an entity in its frame, as the client has it
    now, then again next frame, while the entity is in the world."""
    self = BigWorld.entities.get(entity_id)
    if self is None or not getattr(self, '_wgtkDrawing', False):
        return
    try:
        for drawObject in self.drawObjects:
            if not drawObject['lines']:
                continue
            matrix = _frame(drawObject['name'])
            if matrix is None:
                continue
            for line in drawObject['lines']:
                points = [matrix.applyPoint(point) for point in line['points']]
                colour = OPAQUE | int(line['width'])
                DebugDrawer.DebugDrawer().line() \
                    .zTest(False) \
                    .blendMode(DebugDrawer.BlendMode.BM_OPAQUE) \
                    .width(LINE_THICKNESS) \
                    .colour(colour) \
                    .points(points)
    except Exception:
        LOG_CURRENT_EXCEPTION()
        return
    BigWorld.callback(0.0, lambda: _draw_lines(entity_id))


def _frame(name):
    """The matrix placing a group of this name in the world now: its entity's if it
    follows one, see `_followed`, None while the client doesn't have it, else none."""
    entity_id = _followed(name)
    if entity_id is None:
        return Math.Matrix()
    entity = BigWorld.entities.get(entity_id)
    if entity is None:
        return None
    return Math.Matrix(entity.matrix)


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
    """Each group following an entity, see `_followed`, in its frame: each of its models
    and texts placed again by its entity's matrix, as the client has it now, else
    hidden while the client doesn't have it. Where each is in the entity's frame is
    kept with the product placing it, by its motor's id: one the client placed anew
    since, reused, is where it is placed now."""
    kept = _placed(self)
    placed = self._wgtkPlaced = {}
    for drawObject in self.drawObjects:
        entity_id = _followed(drawObject['name'])
        state = self.objectStates.get(drawObject['name'])
        if entity_id is None or state is None:
            continue
        entity = BigWorld.entities.get(entity_id)
        drawn = [(motor, model) for _, model, motor in state['models']]
        drawn.extend((motor, getattr(label, 'parent', None) or label) for _, motor, label in state['3Dtexts'])
        for motor, shown in drawn:
            product, local = kept.get(id(motor), (None, None))
            if product is None or motor.signal is not product:
                local = motor.signal
            if entity is None:
                motor.signal = local
                shown.visible = False
                placed[id(motor)] = (None, local)
                continue
            product = Math.MatrixProduct()
            product.a = local
            product.b = entity.matrix
            motor.signal = product
            shown.visible = True
            placed[id(motor)] = (product, local)


def _placed(self):
    """Where each model and text following an entity is in its frame, and the product
    placing it, by its motor's id, see `_follow_entities`."""
    placed = getattr(self, '_wgtkPlaced', None)
    if placed is None:
        placed = self._wgtkPlaced = {}
    return placed


def _update_following(self):
    """The entity drawing what the server sent, then its texts deleted off the screen,
    and the groups following entities placed on them, see `_follow_entities`."""
    _update(self)
    try:
        _drop_unused(self)
        _follow_entities(self)
    except Exception:
        LOG_CURRENT_EXCEPTION()


def _on_enter_world(self, prereqs):
    """The entity drawing what the server sent, and its lines each frame, see
    `_draw_lines`."""
    _enter_world(self, prereqs)
    if DebugDrawer is not None:
        self._wgtkDrawing = True
        _draw_lines(self.id)


def _on_leave_world(self):
    """The entity leaving, its lines no longer drawn, and all its texts off the
    screen."""
    self._wgtkDrawing = False
    _leave_world(self)
    try:
        for root in _roots(self).values():
            GUI.delRoot(root)
        _roots(self).clear()
    except Exception:
        LOG_CURRENT_EXCEPTION()


DebugDrawEntity.SPHERE_MODEL = SPHERE_MODEL
DebugDrawEntity.BOX_MODEL = BOX_MODEL
DebugDrawEntity._DebugDrawEntity__update = _update_following
DebugDrawEntity._DebugDrawEntity__create3DText = _create_3d_text
DebugDrawEntity._DebugDrawEntity__createDirectedLine = _create_directed_line
DebugDrawEntity.onEnterWorld = _on_enter_world
DebugDrawEntity.onLeaveWorld = _on_leave_world
