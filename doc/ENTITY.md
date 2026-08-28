# BigWorld entities, briefly

What an "entity" is in BigWorld, how a client and server interact with one, and how that
turns into multiplayer — as background for `wg-toolkit`'s `net/app/entity.rs` and
the `base::App`/`cell::App` work in `net/app/base/`, `net/app/cell/`.

## The three slices

An entity type is declared once (`entities.xml` + a per-type `.def` file, e.g.
`Account.def`) and assigned a numeric type id (1-indexed, `entities.xml` declaration
order — `Entity::TYPE_ID` in this codebase). A single entity *type* can be split across
up to three "slices", each potentially running on a different machine:

- **Base** — the persistent, account-scoped half. Lives on a `BaseApp` process, exists
  for as long as the player is connected, regardless of what space/arena they're
  currently in. This is the half that actually owns the client connection.
- **Cell** — the spatial half. Lives on a `CellApp` process, only exists while the
  entity is placed into some "space" (a hangar, a battle arena) — has a position,
  physics, neighbors.
- **Client** — the local half on the player's own machine. Rendering/UI state.

Not every entity type has all three. In WoT:

| Entity | Base | Cell | Client | Notes |
|---|:-:|:-:|:-:|---|
| `Account`, `Login` | Y | — | Y | No Cell — never physically placed anywhere, just "who is this player and their persistent state". Strictly 1:1 with the one client that owns it (see "Base entities are never shared" below). |
| `Avatar`, `Vehicle` | Y | Y | Y | A player's tank in a battle: account-side state (Base), physically exists/moves in the arena (Cell), local representation (Client). |
| `HangarVehicle` | — | — | Y | 100% client-local — confirmed from source (`ClientHangarSpace.py`'s `BigWorld.createSpace(isHangar=True)` + `BigWorld.createEntity('HangarVehicle', ...)` are pure local engine calls, no Base/Cell round trip). Its `.def` has empty `Properties`/`Volatile` and no method sections at all — it exists purely so the client engine has a class to instantiate for the garage screen. See `re-work/doc/HANGAR_LOADING.md`. |

`entity_id` is the addressing key across all of this — confirmed from the leaked
BigWorld C++ source (`server/baseapp/bases.hpp`'s `BW::map<EntityID, Base*> container_`,
`lib/network/basictypes.hpp`'s doc comment) to be a single global namespace per process,
constant across that entity's Base/Cell/Client instances — not scoped per-connection.

## Interacting with an entity: properties and methods

Two mechanisms, each scoped per-slice:

- **Properties** — synced state. A subset marked `Volatile` (position/orientation-type
  things that change every tick) gets dedicated high-frequency wire treatment (the
  `AVATAR_UPDATE_*` combinatorial messages in `net/app/client/element.rs`) instead of
  the generic property-update path, since sending a full diff 20x/second would be
  wasteful.
- **Methods** — RPCs. The *direction* determines which slice's method table a call
  belongs to:
  - `ClientMethod` — Base or Cell calling *on* the client (e.g. `Account_showGUI`,
    `Login_setPeripheryRoutingGroup`).
  - `BaseMethod` — the client calling its own Base entity (e.g.
    `ClientCommandsPort.doCmdInt3`).
  - `CellMethod` — the client calling the Cell (movement-adjacent RPCs; no `cell::App`
    implementation exists in this project yet, ids only in `net/app/cell/element.rs`).

Every call is addressed by `entity_id` plus a compact "exposed id" packed into the
`ENTITY_METHOD`/`BASE_ENTITY_METHOD` wire id ranges (`ElementIdRange`,
`net/element.rs`) rather than by name — this is what `AnyMethod` and the generated
per-(entity, direction) enums (`Account_Client`, `Account_Base`, ...) encode/decode.

## Multiplayer: Base entities are never shared, Cell entities are

This is the key asymmetry, confirmed against the leaked BigWorld C++ source:

- `server/baseapp/proxy.hpp`'s `Proxy` (the Base-side class that actually has a client
  attached) holds a single `Mercury::ChannelPtr pClientChannel_` — never a collection.
  Even `giveClientTo()` is an *exclusive handoff* (old owner loses it, new owner gains
  it), never simultaneous sharing. A Base entity is structurally incapable of being
  visible to more than one client, ever.
- Cell entities *are* shared, via Area-of-Interest (AoI): a `CellApp` hosting one space
  (e.g. a battle arena) runs every entity physically in it. Each client watching that
  space gets its own `Witness` (`cellapp/witness.hpp`, one per observing client)
  maintaining that client's personal AoI, plus an `EntityCache`
  (`cellapp/entity_cache.hpp`) entry per entity it's currently tracking. So the *same*
  `Vehicle` entity — one `entity_id`, one Cell-side instance — fans out independently to
  every nearby player: each client's own `Witness` drives its own `EnterAoi`/
  `CreateEntity` when the vehicle comes into range, its own stream of property/volatile
  updates while visible, and its own `LeaveAoi` when it drops out. One authoritative
  Cell entity, many independent per-client visibility windows onto it — that's the
  actual multiplayer mechanism.

Base entities opt out of this by construction: `Account`/`Login` have no Cell half, so
they never enter a space, never get a `Witness`/AoI, and can't be shared no matter what.

## Removing an entity from a client

No dedicated wire message exists for removing a single Base-player entity — consistent
with a client only ever having one at a time, there's no protocol need to target one
specifically. The only Base-side lifecycle messages (`net/app/client/element.rs`) are
`CREATE_BASE_PLAYER` (create) and `RESET_ENTITIES` (`ResetEntities { keep_player_on_base:
bool }`, all-or-nothing, no entity id parameter). `LEAVE_AOI` fills the single-entity
removal role, but only for Cell/AoI entities (an entity leaving a client's area of
interest) — it's still an undecoded placeholder in this codebase
(`DebugElementVariable16`), and isn't something a Base app would ever send.

## Giving an entity its Cell slice: `CreateCellPlayer`

`CreateCellPlayer` (`net/app/client/element.rs`) is what turns a client's existing Base
entity into one with a Cell presence too (e.g. entering a battle arena). Its wire layout
deviates from vanilla BigWorld the same way `CreateBasePlayer`'s does (extra WoT-specific
fields), and unlike most of this codebase's other elements, it could **not** be pinned
down from live captures alone — every plausible reinterpretation of the raw bytes kept
producing physically absurd numbers (positions like `1e29`, headings outside `0..2π`).
It was only resolved by disassembling the actual client binary:

1. The live `ClientInterface` message table (same technique as
   `re-work/frida/dump_interfaces.js`) gives `createCellPlayer`'s registered handler
   *object* address for the running build — not a bare function pointer. BigWorld stores
   a small handler struct there instead of using C++ virtual dispatch; its first two
   fields point back into a `.rdata` string/pointer table (a red herring for this
   purpose), and the *third* field (`+0x10`) is the real code address in `.text`.
   (`re-work/frida/dump_cellplayer_handler.js`, `dump_cellplayer_handler2.js`.)
2. Disassembling that address in radare2 (a single small function, not a full-binary
   `aaa` pass) shows the real read order directly: a leading byte (always `0` in every
   capture so far, meaning unconfirmed), then `space_id` (`u32`), then a `u16` (also
   always `0` so far), then a `u32`, then a `Position3D` (3 raw `f32`s) read via a shared
   stream-decode helper, then `packed_xz_scale` (`f32`), then `direction` (3 more raw
   `f32`s via the same helper) — 3 bytes longer overall than the vanilla-shaped guess.

This is now `CreateCellPlayer`'s confirmed layout (`unk_flag`, `space_id`, `unk_short`,
`vehicle_id`, `position`, `packed_xz_scale`, `direction`, `cell_data`). Confirmed correct
empirically too: `packed_xz_scale` came out byte-for-byte identical (`0.0076444...`)
across unrelated battles (a per-server-config constant, as it should be), and `position`/
`direction` for repeated entries into the *same* historical-battle spawn point matched to
4+ decimal places across three independent sessions.

**The `vehicle_id` field's real meaning is still open, and it is almost certainly not a
vehicle reference despite the name** (kept only for continuity with the pre-fix struct
field). It doesn't fit a real vehicle identifier at all: WoT's actual vehicle compact
descriptor is `(innationID << 8) + itemTypeID(vehicle=1) + (nationID << 4)` (confirmed
against `wot-src/sources/res/scripts/common/items/vehicles.py`'s
`getVehicleTypeCompactDescr`/`items/__init__.py`'s `makeIntCompactDescrByID`, and checked
against real per-nation ids in `res/scripts/item_defs/vehicles/<nation>/list.xml` via
`wgtk res`/`wgtk pxml`) — which tops out in the low hundreds of thousands, while captured
`vehicle_id` values have ranged from ~13.1M to ~550M. Instead, one session's `vehicle_id`
(`13910923`) landed almost exactly on that same session's `Account_onPrebattleJoined`
`prebattle_id` (`13910991`), captured moments earlier — consistent with both being drawn
from one shared, monotonically-increasing BigWorld-wide session/allocation counter, not
anything vehicle-specific. (The *actual* vehicle used in a battle does show up
elsewhere, as a real compact descriptor, e.g. `vehTypeCompDescr` inside the post-battle
`Chat_onChatAction` payload — that field decomposes cleanly via the formula above.)

## What this project implements today

`base::App` (`net/app/base/`) implements the Base side only. The Cell side exists only
as element ids (`net/app/cell/element.rs`), no socket-server implementation. For the
*login → hangar* flow specifically this is sufficient — the hangar is 100% client-local
(`HangarVehicle` has no Cell), so reaching it never needs Cell/AoI machinery at all.
Real multiplayer (battle arenas) would need a `cell::App` implementation, a substantially
separate, larger effort from what's currently built.

## Components: assembling an entity from interfaces

WoT entities aren't authored monolithically — most of their properties and methods
actually come from reusable building blocks, and understanding how those get flattened
into the generated Rust (`wg-toolkit-cli/src/wot/gen/{entity,interface,alias}.rs`) matters
for reading that code, or for reverse-engineering a new game version's tables.

Two related concepts, both parsed in `wg-toolkit-cli/src/bootstrap/`:

- **`Interface`** (`bootstrap/model.rs`) — a reusable named bundle: a `properties` list and
  three separate method lists (`client_methods`, `base_methods`, `cell_methods`). An
  interface can itself `implements` other interfaces (nested composition). An `Entity` is
  modeled as nothing more than an `Interface` plus a numeric `id` — there's no structural
  distinction between "the entity's own stuff" and "stuff it composes".
- **`Component`** (`bootstrap/model.rs`) — a WoT-extension-specific wrapper around an
  `Interface` (`res/<ext>/extension.xml`'s `Components`), carrying `of_entities: Vec<String>`
  (parsed from `<ofEntity>`) naming which entities it folds into. Two flavors:
  - **Static** (`Model::static_components`) — folded into every targeted entity's property
    struct and method tables at codegen time. This is what shows up as
    `AccountBattleRoyaleTournamentComponent`, `LaPingerComponent`, etc.
  - **Dynamic** (`Model::dynamic_components`) — attached to individual entity *instances* at
    runtime instead (e.g. only for the duration of a battle mode), and are deliberately
    *not* folded into any entity's static tables — see "Open question" below.

### Properties: components are inlined, in declaration order

Each interface (whether the entity's own or a component's) becomes a plain data struct via
`__struct_simple_codec!` in `interface.rs` (e.g. `Wheels`, `VehicleObserver`,
`Perks_Vehicle`). An entity's own struct in `entity.rs` embeds each implemented interface as
a single field, named `i_<InterfaceName>`, placed *before* the entity's own directly
declared properties:

```rust
pub struct Vehicle {
    pub i_VehicleObserver: VehicleObserver,
    pub i_Wheels: Wheels,
    pub i_Perks_Vehicle: Perks_Vehicle,
    pub isStrafing: BOOL,
    // ...
}
```

Because the generated `Codec` impl for a struct just serializes fields in declaration order,
this *is* the wire layout: interface properties are inlined in place, in interface
declaration order, ahead of the entity's own properties. There's no length prefix or tag
marking where one component's data ends and the next begins — a reader must already know
the entity's exact schema (i.e. have the same generated tables) to decode it at all.

### Methods: size-sorted own table, then components appended untouched

Each per-direction method enum (`Account_Client`, `Account_Base`, `Account_Cell`, ...,
implementing `AnyMethod`) is built in two passes:

1. The entity's *own* methods are assigned sequential `exposed_id`s sorted first by fixed
   argument length ascending (`0, 0, 0, 1, 1, ..., 4, 8, ...`), then the variable-length
   tiers (`var8`, `var16`, `var24`, `var32`) last. E.g. in `Account_Client`, ids `0x00`
   through `0x28` are the entity's own methods, size-sorted this way.
2. **Each static component targeting this entity is then appended afterward**, each keeping
   its own internal size-sort but never merged back into the entity's sort — confirmed live
   and documented at `bootstrap/model.rs`'s `Component` doc comment. In `Account_Client`,
   `AccountBattleRoyaleTournamentComponent` contributes `0x29` (var8) then `0x2A` (var16),
   followed by `LaPingerComponent` at `0x2B` (var8) — note that a var8 id sits *after* a
   var16 id there, which only makes sense once you know each component sorts
   independently rather than joining the entity's own global sort. This is *why* adding or
   removing an unrelated extension never shifts existing exposed ids for other components or
   for the entity's own methods.

So a component's methods are dispatched as ordinary entries in the *same* enum as the
entity's own methods — there's no runtime notion of "this call belongs to component X";
component boundaries are erased entirely at codegen time, and calling one is
indistinguishable from calling any other method on that entity/direction.

### Wire framing recap

Both directions reserve a contiguous element-id range for entity method calls
(`ElementIdRange` in `net/element.rs`): server→client `ENTITY_METHOD = 0x4E..=0xA6`
(`net/app/client/element.rs`), client→base `BASE_ENTITY_METHOD = 0x88..=0xFE`, client→cell
`CELL_ENTITY_METHOD = 0x10..=0x87` (`net/app/base/element.rs`). A method's 16-bit
`exposed_id` maps onto that fixed 8-bit range: if the entity's total method count for that
direction fits within the range's slots, each method gets its own dedicated wire id
(`element_id = first + exposed_id`); if it overflows, the trailing slots become "sub-id"
slots, and one extra byte following the wire id disambiguates within that slot
(`ElementIdRange::{from,to}_exposed_id`). Reading a call is a 2-tier dispatch: wire
`element_id` (+ optional sub-id byte) → `exposed_id` → `AnyMethod::read(exposed_id)`,
sized per that method's declared length.

### Open question: dynamic components

Dynamic components have no confirmed wire encoding yet, but their presence is now
confirmed live: a battle-entry `CreateBasePlayer` for the `Account` entity was captured
with `entity_components_count: 17` (previously only `0` had ever been observed).
`CreateBasePlayerAny::read` (`wg-toolkit-cli/src/wot/proxy/mod.rs`) consumes exactly
`entity_data` + the trailing count byte with zero leftover bytes (confirmed via the
bundle reader's "remaining data" check, see `net/bundle.rs`), so the 17 components'
actual data is NOT inline in this message -- it must arrive later via `ENTITY_PROPERTY`
update messages.

This is corroborated directly: `Account`'s own static client-visible property table
(now decoded via `Entity::Property`, see the "Client-visible properties" section
below) only spans exposed ids `0..3` (`incarnationID`, `requiredVersion_2310`, `name`,
`initialServerSettings`), yet live battle traffic continuously sends property updates at
exposed id `19` and others well outside that range. Per elimination, these must belong
to one of the 17 attached dynamic components -- consistent with `Model::dynamic_components`'s
doc comment that dynamic components claim no fixed, statically-predictable exposed id.
Decoding these specific ids would require either observing the exact attach order live
(e.g. via a Frida hook on the client's component-attach path) or brute-forcing candidate
dynamic components' property sizes against the captured byte lengths. Not yet attempted.

**New lead (2026-08-29)**: the live `EntityDescriptionMap` dump described below (done to
confirm extension entity ids) turned up something not modeled by this codebase at all --
past the last real entity (`SPGZone`, id `0x32`), the *same* id sequence keeps going for
dozens more slots with clearly component-shaped names (`DogTagComponent`,
`HealthComponent`, `VehicleBuff`, `Radar`, ... past `0x100`, ending around `BunkerLogicComponent`).
So static and dynamic extension components apparently get their own real slots in this
same `EntityTypeID` space too -- distinct from, and in addition to, the already-confirmed
exposed-method/property-id folding into their owning entity's tables. This is plausibly
the missing piece for decoding dynamic component attachment (a component "type" would
have its own id here, the same way an entity does), but that connection hasn't been
tested yet -- nothing has confirmed what (if anything) reads these particular ids off the
wire. Worth a follow-up dump (extend `re-work/frida/dump_entity_types.js`'s walk further
and correlate names 1:1 against `Model::static_components`/`dynamic_components`) before
trying to use these ids for anything.

## Confirmed: extension entities continue the main list's `TYPE_ID` numbering

The generator's assumption that entities declared inside an extension's own
`Entities/ClientServerEntities` (`Entity::from_extension`, `wg-toolkit-cli/src/bootstrap/`)
continue the main `scripts/entities.xml` list's numbering -- extensions in alphabetical
directory order, declaration order within each -- was, until now, only an analogy with the
separately-confirmed static-component method-folding rule, never itself checked against a
live client.

**Confirmed live (2026-08-29)**, by locating a running client's actual
`BW::EntityDescriptionMap` and reading its backing array directly out of process memory
(`re-work/frida/dump_entity_types.js`). The approach needed no prior knowledge of the
map's address or `EntityDescription`'s field layout:

1. Static disassembly (radare2, on the local `re-work/bin-2.3.1.3/WorldOfTanks.exe` copy)
   found `EntityDescriptionMap::checkCount`'s inlined bounds-check via its distinctive
   `"FATAL ERROR: invalid entity type id index"` assert string (an `IF_NOT_MF_ASSERT_DEV`
   that, unlike most such asserts, was NOT compiled out of this release build). Its
   indexing code (`imul rbx, rax, 0x328; add rbx, [r14]`) confirms `sizeof(EntityDescription)
   == 0x328` (808 bytes) -- the array's stride.
2. Chasing the map's own storage address further (who calls `parse()`, where's the
   persistent singleton) turned into the same kind of static-analysis rabbit hole as this
   project's earlier `InterfaceMinder` id investigation -- abandoned in favor of a live
   read, same lesson as before.
3. Since the process is already running, the object's address doesn't matter: scan live
   memory for a known entity name string (tried several short ones, since `BW::string`'s
   inline/SSO storage means a name under ~15 bytes shows up as raw bytes at a fixed
   struct-relative offset), then check readability at `hit ± k*808` for consecutive `k`.
   Every element's name lives at the *same* relative offset, so once one hit scores well
   on this periodicity test, walking outward at that exact stride reads off the whole
   table in true index order -- no struct-offset knowledge needed at all.

Result: positions `1..0x32` matched `Account` through `SPGZone` exactly, including every
one of the 10 extension entities that exist today (`battle_royale`'s `Mine`/`Loot`/
`Placement`/`InfluenceZone`/`BattleRoyaleRadio`/`ThunderStrike` at `0x29..0x2E`, `comp7`'s
`Comp7Lighting` at `0x2F`, `comp7_core`'s `ApplicationPoint` at `0x30`,
`server_side_replay`'s `ReplayAccount` at `0x31`, `story_mode`'s `SPGZone` at `0x32`) --
each landing on precisely the id this generator already assigned it. Position `0` resolved
to `GeneralSpaceData`, which is not a declared entity at all and has no valid neighbors of
its own further back -- almost certainly the tail element of a *different*, adjacently-
allocated table sharing the same `EntityDescription`-shaped 808-byte stride (BigWorld also
has a separate `UserDataObjectDescriptionMap` for `scripts/user_data_objects.xml`, built
from the same base class), not a real `EntityTypeID` slot -- consistent with this
generator's existing 1-indexed convention (`Account` = 1, not 0) being correct.

## Client-visible properties

Properties visible to the client (any of BigWorld's `ALL_CLIENTS`/`OWN_CLIENT`/
`BASE_AND_CLIENT` flags -- confirmed against the same rationale as
`DataDescription::isClientServerData()` in the leaked engine source, `entitydef/
data_description.cpp`/`.ipp`) get their own flat, per-entity exposed-id table, generated
via `Entity::Property` (`wg-toolkit/src/net/app/entity.rs`'s `AnyProperty` trait +
`__enum_entity_properties!` macro, `wg-toolkit-cli/src/bootstrap/mod.rs`'s
`generate_entity_properties`). Exposed ids are assigned by the **exact same stable-sort
rule as methods** (fixed-size first ascending, then variable-size ascending, static
components appended afterward keeping their own order) -- confirmed against
`entitydef/entity_description.cpp`'s `allocateClientServerFullIndexes`/
`ClientServerPropertiesSortHelper`, which explicitly uses the same bandwidth-multiplexing
rationale as method sorting. Wire framing is identical to `ENTITY_METHOD` too: a 1-byte
exposed id (with the same `ElementIdRange` sub-id overflow scheme for entities with more
properties than fit in `ENTITY_PROPERTY`'s 0xA7..0xFE range) followed directly by the
property's own value in its native codec -- no extra framing, unlike the bit-packed,
path-addressed `NestedEntityProperty`/`SliceEntityProperty` messages (still undecoded
placeholders) used for updates to individual elements *within* a complex/array property.

**Unlike `EntityMethod`, an unrecognized exposed id here is always a hard `Err`, never a
guessed fallback framing** (`EntityPropertyInner` in `net/app/client/element.rs`) -- see
`doc/PROXY.md`'s crash case study for why guessing was tried once, confirmed live to be
unsafe, and reverted.
