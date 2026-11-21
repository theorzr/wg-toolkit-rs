# Code style

The formatting and code conventions of this workspace (`wg-toolkit`, the library, and
`wg-toolkit-wot`, the CLI and World of Tanks server). Follow them in every change, tests
included. Code that predates a rule is fixed when touched, or when asked.

## Layout

### Imports

Groups separated by a blank line, in this order:

1. `std`
2. external crates (`glam`, `tracing`, `blowfish`…)
3. `wgtk` (from `wg-toolkit-wot`)
4. `crate::`
5. `super::` and sibling modules

Within a group, sort by line length, longest first, like a staircase. Multi-line `{…}`
imports come first in their group. Never append a new `use` at the end of a group:
re-sort it. Module declarations (`mod x;`) go before the imports.

```rust
use std::collections::HashMap;
use std::sync::Arc;

use glam::{Affine3A, Vec3};

use crate::base::ballistics::impact::{self, Indication};
use crate::base::physics::{VehicleId, World};
use crate::base::TICK;

use super::situation::BotEnemy;
use super::CLOSE_RANGE;
```

- Import the types you use, `glam` ones included: `Vec3`, not `glam::Vec3`. The same goes
  for types in signatures: `use crate::pxml::{self, Element};`, then `&Element`.
- Don't import names that are ambiguous outside their module, like a free function
  `read_xml`: import the module and call `item_defs::read_xml`.

### Constants

A module's constants go at its top, after the imports and before its types, even when
only one function uses them. Each has a doc comment saying what it is, its unit, and
where it comes from ("Our choice", or the client source it is from).

Constants used only for debug drawing are prefixed `DEBUG_`, and functions only for it
`debug_`, except in the `debug` module itself.

### Order of functions

Callers first: a function comes before the functions it calls. Entry points (functions
nothing else in the impl calls) keep their order. Each one is followed, depth first, by
what it calls, in the order it first calls them. A new method goes after its first
caller.

Split long impls into sections with plain ASCII comments, and sort within each section:

```rust
// -- Its shells --
```

Never use box-drawing characters for section comments. They're fine in diagrams inside
doc comments' `text` blocks.

### Paragraphs in functions

When a function body has several steps, separate them with blank lines, and put a blank
line right after the opening `{` and right before the closing `}`. Short one-step bodies
stay tight.

```rust
pub fn stand(&mut self, point: Vec3, radius: f32) -> Option<Vec3> {

    self.refresh_nav();
    let nav = self.nav.as_mut()?;

    nav.stand(point, radius, &self.space, &blocking(&self.space, &self.rubble))

}
```

### Struct initializers

A short, simple struct initializer (`Foo { … }`, `Self { … }`, enum struct variants) stays
on one line: at most 3 fields and about 60 characters. The `net` module is the reference.

```rust
return Event::IoError(IoErrorEvent { error, addr: peer });
```

A longer one, or one of more fields, goes on several lines, one field per line with a
trailing comma:

```rust
app.component(marker)?.set_gunMarker(Some(GunMarker {
    gunPosition: position,
    shotVector: velocity,
    dispersion: dispersion.angle,
}))?;
```

A long line around a short initializer is no reason to split the initializer: spread the
call around it instead. Destructuring patterns (`let Foo { a, b } = x;`) are not
initializers and stay as they are.

### Long lines

When a `let`, array, tuple, call or signature would be long on one line, spread it over
several lines, one element per line with a trailing comma. Don't pack two long values into
one tuple `let`: write one `let` per value, unless one name feeds the next (a swap, say).

```rust
// Not this:
let (own, others) = ([self.axes.x_axis, self.axes.y_axis], [other.axes.x_axis, other.axes.y_axis]);

// This:
let own = [
    self.axes.x_axis,
    self.axes.y_axis,
];
let others = [
    other.axes.x_axis,
    other.axes.y_axis,
];
```

### Match arms with several patterns

When a match arm has several patterns joined with `|`, put each pattern on its own line,
with the `|` at the end of the line:

```rust
match class {
    VehicleClass::TankDestroyer |
    VehicleClass::Artillery => Self::far_off(),
    _ => Self::default(),
}
```

## Code

### Control flow

- Never `match` on a bool: write `if condition { a } else { b }`, on one line when short.
  A match on an `Option` or an enum is fine.
- No helper closures bound to a name (`let point = |v| …;`): write a private `fn` with a
  doc comment, or inline the code. Closures belong where one is needed, as an iterator
  adapter's argument (`map`, `filter`, `find_map`, `map_err`…). A filter closure that only
  one query uses is written inline at the call, not as a named `fn` returning `impl Fn`.

### Angles

Write degree constants as `25.0f32.to_radians()`, and convert for display with
`.to_degrees()`. They work in `const` items. Never `x * PI / 180.0`.

### Types and modules

- Each file owns a type and its behaviour. Never put an `impl` of a type in another
  module's files. Give a module's types methods rather than free functions over them.
- Inside a private module, declare items plain `pub`. Use `pub(crate)` only for items of
  public modules that must stay inside the crate.
- No `std::env::var` switches in the emulator's own code, such as the physics. To compare
  variants, write a temporary test, or edit the constant, and remove it afterwards.

### Naming

- Type names must be clear on their own, wherever they're imported: prefix a name that
  could mean something else in this codebase with its domain. `FoliageCrossing`, not
  `Crossing`; `DriveOrder`, not `Order`; `BotTarget`, not `Target`.
- Addresses are named after what they point to, without an `_addr` suffix: `--bind`,
  `base_app`, `real_login_app`. Where an app links to takes `_link`: `login_link`.
  Names from APIs we don't own (`peer_addr`, `local_addr`) stay.

### Doc comments

Doc comments (`///`, `//!`) describe what the code is and does now. They never mention
tests, experiments, measurements, or how the code used to be. Keep that evidence in
commit messages, test comments or notes. Stating which protocol or client behaviour the
code follows ("as the captured server does") is fine.

### Logs

In `wg-toolkit-wot`:

- The subject goes in tracing fields, not the text: `%client`, `entity_id`, `shot`…
- The message is a past-tense sentence of what happened: "Created the avatar".
- Failures are `Failed to <verb> <object>: {e}`, refusals `Refused <what>: <why>`, and
  ignored input `Ignored <what>: <value>`.
- Startup lines are `<Verb> on|to: <value>`, as in "Running on: 0.0.0.0:20017".
- Values follow a colon at the end of the message.

The proxy's per-element protocol trace (`<- Nrl update: …`) keeps its own arrow format,
which `wot replay` reproduces.
