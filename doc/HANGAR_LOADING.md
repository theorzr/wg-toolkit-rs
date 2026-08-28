# Hangar loading protocol

Reverse-engineered from a live capture of v2.3.1.3 (2026-08-25) via the `wgtk wot`
proxy, cross-checked against `wg-toolkit-cli`'s generated entity method/property
tables. Goal: document everything the client does, end to end, between "Login
success" and a fully-loaded, interactive Hangar screen (account currencies, owned
tanks, dossier stats included) precisely enough to reimplement a client from
scratch.

Trace file used: `proxy-trace.jsonl` at repo root, captured right after restarting
the proxy with the sub-id/var8-fallback changes, from a fresh login through several
minutes idling in hangar. Raw resource payloads referenced below were extracted and
saved separately (paths noted per section) since they're too large to inline.

## TL;DR

- Money, XP, gold, bonds, garage slots, and the entire owned-tank/module/consumable
  inventory are **not** synced via BigWorld entity properties at all. They arrive as
  one giant pickled Python blob (`Account.cache`, ~368 KB) pushed once, right after
  login, over the "Resource" chunked-transfer mechanism -- triggered by the client
  calling `ClientCommandsPort.doCmdInt3` with `AccountCommands.CMD_SYNC_DATA`
  (already noted in `wg-toolkit-cli/src/wot/proxy/mod.rs:360`).
- Two more one-shot Resource pushes follow: a ~200 KB shop/economy rules cache
  (prices, exchange rates -- not player-specific), and a tiny (193-byte) per-vehicle
  binary "dossier" stats update for one tank.
- After those three transfers land, the observed traffic goes fully idle apart from
  periodic heartbeats/telemetry -- **no further entity or property traffic appears
  for the Hangar itself**. This strongly implies the 3D Hangar screen is built
  entirely client-side from the cached data already delivered; no `Vehicle` /
  `HangarVehicle` cell entities are created over the wire just to show the garage.
- The recurring `Account` method `0x2B` is **fully decoded and live-confirmed**:
  `LaPingerComponent.pingMeAndThenJustTouchMe(ip, port, dbID, iterations,
  timeout)`, a native (C++) periodic ping probe of candidate periphery/CDN
  servers. Root cause: `wg-toolkit-cli`'s bootstrap only read an entity's own
  `<Implements>` list, missing the ~35 methods that WoT's optional "extension"
  packages (`la_pinger`, `battle_royale`, `comp7`, `frontline`, `story_mode`, ...)
  fold into core entities at build time via a WG-specific `<ofEntity>` mechanism
  with no trace in either the decompiled source or vanilla BigWorld. Fixed by
  scanning all extensions' `StaticComponents` and folding them in; the exact
  append rule was determined empirically against this live capture. See "Bootstrap
  fix" below.
- Hangar itself is **confirmed, from source**, to be 100% client-local:
  `BigWorld.createSpace(isHangar=True)` + `BigWorld.createEntity('HangarVehicle',
  ...)` are local API calls with no BaseApp/CellApp round trip, and
  `HangarVehicle.def` has no methods/properties at all -- it exists purely so the
  engine has a class to instantiate. Confirms the "no wire traffic for hangar
  itself" conclusion drawn independently from the capture.
- The vehicle binary compact descriptor (`compDescr`) format is now **fully
  decoded and byte-verified** against this session's own captured data -- see
  below. So is the simpler integer `vehTypeCompDescr` and its relationship to the
  binary form.
- A real decoder gap remains: the client's outgoing `doCmdInt3` call that kicks
  off the whole sync is never observed in the trace, and the base-app bundle
  containing it dies with `"Error while reading bundle: failed to fill whole
  buffer"` right after 3 `AccountDebugger` calls. See "Known gaps" below.

## Session timeline (as observed)

All from `proxy-trace.jsonl`, base-app span, times relative to `Login success`:

| t (s) | Event |
|---|---|
| 0.00 | `Login success` (LoginApp) |
| +0.01 | `Forwarding new peer` -- handed to first BaseApp |
| +0.03 | `Create base player` entity 1571864, `entity_type_id=11` (transient `Login` entity) |
| +0.03 | `Select player entity: 1571864` |
| +0.03 | `Login_setPeripheryRoutingGroup(a0="default", a1=None)` -- client tells server which periphery/routing group it's in |
| +0.28 | `Switch base app to <addr> (reset entities: true)` -- handed off to the *real* BaseApp for this account |
| +0.33 | `Forwarding reconnected peer (post-switch)` |
| +1.47 | `Create base player` entity 1679316, `entity_type_id=1` (**`Account`**, the real player entity) |
| +1.53 | `Account_showGUI(data)` -- server tells client to display the login/loading UI; `data` is a small dict: `databaseID`, `serverUTC`, `sessionStartedAt`, `isAogasEnabled`, ... |
| +1.53 | unrecognized `Account` method `0x2B` fires for the first time (see below) -- repeats every ~1-2s from here on |
| +2.50 | (client\->server) 3x `AccountDebugger_accountDebugger_registerDebugTaskResult(a0, a1, a2)` -- client-side perf/debug telemetry, then **bundle read fails** (see "Known gaps") |
| +2.62 | `ClientCommandsPort_onCmdResponse(request_id=221, result_id=1, error="")` |
| +2.62 | `Resource header: {id: 221, description: ...}` -- Resource transfer begins for `res_id=221` |
| +2.65 | (client\->server) `AccountAuthTokenProvider_requestToken(request_id=3, token_type=1)` |
| +2.65 | `ClientCommandsPort_onCmdResponse(request_id=222, ...)` |
| +2.75 | `ClientCommandsPort_onCmdResponse(request_id=223, ...)` |
| +2.85 | `AccountAuthTokenProviderClient_onTokenReceived(request_id=3, token_type=1, data={databaseID, token})` |
| +7.10 | `Resource completed` for `res_id=221` -- **367,800 bytes**, the big `Account.cache` sync blob |
| +7.32 | `Resource header: {id: 222, ...}` |
| +7.59 | (client\->server) `AccountDebugger_...` |
| +7.68 | `ClientCommandsPort_onCmdResponseExt(request_id=224, result_id=0, error="", ext={"prevRev": 56, "rev": 56})` -- cache revision ack |
| +9.20 | `Resource completed` for `res_id=222` -- **200,739 bytes**, shop/economy rules |
| +9.34 | `Resource header: {id: 223, ...}`, immediately `Resource completed` -- **193 bytes**, per-vehicle dossier update |
| +9.34 | `Resource header: {id: 2, ...}` then `Resource completed but failed to decode as python: unresolved global reference` -- a small `collections.deque`-based blob (chat/action-log queue, our pickle decoder doesn't register `collections.deque` yet) |
| +9.4 onward | steady state: periodic `Account_receiveServerStats(clusterCCU, regionCCU)` (~every 5s), periodic `AccountDebugger_...` telemetry (~every 5s), periodic unrecognized `0x2B` method (~every 1-2s). **Nothing else** -- confirmed over an extended idle window (trace grew from 122 to 1000+ lines while sitting in hangar with no client interaction) with exactly the same fixed set of message types recurring; no new element types ever appeared. |

`entity_type_id` numbering is 1-indexed matching `entities.xml`'s
`<ClientServerEntities>` declaration order (`Account`=1, ..., `Login`=11).

## The account cache sync (`res_id` 221 -- `AccountCommands.CMD_SYNC_DATA`)

Triggered by `ClientCommandsPort.doCmdInt3` on `Account_Base` (exposed id `0x0E`,
`Fixed(28)` bytes on the wire per our generated table). Confirmed against
`wot-src` (`common/AccountCommands.py`):

```python
CMD_SYNC_DATA = 100
RES_SUCCESS = 0
RES_STREAM  = 1   # "answer is coming as a chunked Resource transfer"
RES_CACHE   = 2   # "nothing changed, use your local persistent cache"
```

The `request_id`/`res_id` correlation isn't a lookup table, it's the *same
number* end to end -- confirmed in `client/Account.py`:
```python
# __getRequestID(): a wrapping per-connection counter, range
# [REQUEST_ID_UNRESERVED_MIN=220, REQUEST_ID_UNRESERVED_MAX) from streamIDs.py
# -- exactly why our observed request_ids (221-224) are small sequential
# numbers right above 220: they're the first commands issued after onBecomePlayer.
requestID = self.__getRequestID()
getattr(self.base, doCmdMethod)(requestID, cmd, *args)   # -> wire call
```
and when the server's `onCmdResponse` comes back with `resultID == RES_STREAM`,
that same `requestID` is subscribed as the resource/stream id
(`self._subscribeForStream(requestID, ...)` in `Account.py`) -- so `res_id` you
see on the wire literally *is* the original `request_id`.

The call site itself, `client/account_helpers/AccountSyncData.py`:
```python
self.__account._doCmdInt3(AccountCommands.CMD_SYNC_DATA, self.revision,
                           crc if crc else 0, 0, proxy)
```
fired from `AccountSyncData.onAccountBecomePlayer()`, itself called from
`PlayerAccount.onBecomePlayer()` (`Account.py:246-260`) alongside sibling calls
`self.inventory.onAccountBecomePlayer()`, `self.shop.onAccountBecomePlayer()`,
`self.dossierCache.onAccountBecomePlayer()` -- exactly the fan-out that produces
the near-simultaneous 221/222/223/224 requests observed in the trace.

Payload framing, confirmed in `client/account_helpers/SyncController.py`:
```python
data = zlib.decompress(data)
data = cPickle.loads(data)
```
i.e. **zlib-compressed pickle**, matching what our own proxy already does
(`wg-toolkit-cli/src/wot/proxy/mod.rs:12,630`, `ZlibDecoder` + `serde_pickle`) --
this is why decoding already worked without any changes needed this session.

The response is applied via `PlayerAccount._update()` (`Account.py:1145-1206`),
which fans the unpickled dict back out by top-level key: `self.inventory.
synchronize(diff)`, `self.stats.synchronize(diff)`, `self.achievements20.
synchronize(diff)`, etc. -- each helper pulls its own key
(`diff.get('inventory', ...)`, `diff.get('stats', ...)`) out of the same dict,
which is exactly the "one big cache dict with many top-level sections" shape
observed below.

The decoded pickle is a single dict, saved to `/tmp/res221.txt` this session
(1.55 MB as Python-repr text; not committed, regenerate from the trace if needed).
Full top-level key list, with an approximate serialized size per value to gauge
what's "big config" vs "small player state":

| Key | ~size | What it is |
|---|--:|---|
| `stats` | 93 KB | **The actual account state** -- see below |
| `inventory` | 173 KB | **Owned vehicles/modules/consumables** -- see below |
| `("eventsData", "_r")` | 942 KB (!) | opaque/raw sub-blob, event configs |
| `("serverSettings", "_r")` | 260 KB | opaque/raw sub-blob |
| `economics` | 28 KB | quest/mission reward-table config (not player state) |
| `tokens` | 16 KB | owned "tokens" (quest/event currencies), keyed by string id |
| `epicMetaGame` | 6.3 KB | Ranked/Epic battles progress |
| `quests` | 7.6 KB | quest definitions/progress |
| `sessionStats` | 7.5 KB | per-session battle stat breakdowns |
| `newYear26` | 4.1 KB | seasonal event state |
| `potapovQuests` | 2.8 KB | |
| `blueprints` | 2.7 KB | vehicle "blueprint fragment" counts, keyed by `vehTypeCompDescr` |
| `dogTags` | 1.4 KB | |
| `battlePass` | 427 B | level, points, per-vehicle points |
| `battleRoyale`, `comp7`, `ranked`, `pm2_progress`, `journey`, `lootBoxes`, `dailyQuests`, `weeklyQuests`, `pets_system`, `goodies`, `piggyBank`, `commendations`, `achievements20`, `wtr`, `storyMode`, `wotPlusProBoost`, `groupLocks`, `preferredMaps`, `renewableSub`, `resourceWell`, `recycleBin`, `premium`, `cache`, `anonymizer`, `mapsTraining`, `offersData`, `limitedUi`, `LS_info`, `LS_inventory`, `abFeatureTest`, `challenges`, `freePremiumCrew`, `isDemoAccount`, `questsRewards`, `platformBlueprintsConvertSaleLimits`, `("intUserSettings","_r")`, `("prebattleInvites","_r")`, `("sessionStats","_r")`, `("vehiclesGroupMapping","_r")` | small | assorted feature/event state, self-explanatory from key names |
| `account` | small | `attrs`, `autoBanTime`, `clanDBID`, `globalRating`, `premiumExpiryTime` |
| `rev` | 1 int | cache revision number, echoed back in `onCmdResponseExt`'s `{"prevRev", "rev"}` |

Keys written as `("name", "_r")` are 2-tuples, not plain strings -- worth noting
since a naive `dict[str]` assumption in a reimplementation will break on them.

### Money / gold / XP / bonds (the actual answer to "how to load money/experience")

All under the top-level **`stats`** dict, flat (not nested further):

```
"credits": 14031963        # silver, the main currency
"gold": 11284               # premium currency
"freeXP": 106198             # free experience (untied XP)
"crystal": 88420             # bonds
"equipCoin": 3820            # a secondary coin currency (event/battle-pass?)
"eventCoin": 0
"xp": ...                    # NOT present at this level in this account snapshot
                              #   (per-vehicle XP lives in "inventory", see below)
"berths": 91                 # crew barracks slots
"slots": 164                 # garage (vehicle) slots
"denunciationsLeft": 10
"freeTMenLeft": 20
"freeVehiclesLeft": 10
"vehicleSellsLeft": 5
"language": "fr"
"hasFinPassword": False
"finPswdAttemptsLeft": 5
```

`stats` also has a `"dummySessionStats": {"base": {"credits", "xp"}, "premium":
{"credits", "xp"}}` sub-dict -- an illustrative "what you'd earn with/without
premium" preview, *not* real balances (don't confuse the two).

### Owned tanks / modules / consumables (`inventory`)

`inventory` is a dict keyed by small integers that are **item-type categories**,
each holding item-id (or, for vehicles, an account-local "vehicle inventory id")
-> data:

- **`1`**: vehicles (`ITEM_TYPE_INDICES['vehicle'] == 1`, `common/items/
  __init__.py`). Keyed by a small per-account integer -- this is literally called
  **`vehInvID`** in source (see `constants.py`: `VEHICLE_NO_INV_ID = -1`, the "no
  such vehicle" sentinel) -- the server-authoritative DB row id of that specific
  owned tank *instance*, stable across sessions, distinct from the tank *type*.
  Each entry is a dict with sub-keys `compDescr`, `crew`, `shells`,
  `shellsLayout`, `eqs`, `eqsLayout`, `boosters`, `boostersLayout`,
  `devicesLayout`, `rent`, `repair`, `postProgression`, `customRoleSlots`,
  `enhancements`, `lastCrew`, `layoutIndexes`, `telecomOrders`,
  `customizationExpiryTime`, `extraSettings`, `settings`,
  `igrCustomizationLayout` -- confirmed read generically as
  `itemsInvData.get(key, {}).get(vehInvID, default)` in `client/gui/shared/utils/
  requesters/InventoryRequester.py`.
  - `compDescr` here is a **packed binary blob per vehicle** -- fully decoded,
    see the dedicated section below.
  - There's also a `"vehicle"` key at the *outer* `inventory` level (sibling of
    `1`, `2`, ...) mapping *module inventory ids* -> owning `vehInvID` -- i.e. a
    reverse index ("this engine is installed on vehicle 102").
- **`2`..`7`+**: other item categories (equipment, optional devices, shells,
  boosters, consumables, ...), each a flat `{item_id: count}` map. Exact
  category-number -> name mapping not pinned down this session, but it's a fixed
  enum sitting right next to the vehicle one already confirmed --
  `ITEM_TYPE_NAMES`/`ITEM_TYPE_INDICES` in `common/items/__init__.py` -- so this
  is a trivial follow-up read, not an open mystery. Category `8` observed to
  *also* use a `compDescr`-style sub-dict (standalone/unmounted modules sitting
  in the depot, keyed by module id -> a longer binary blob than the per-vehicle
  one -- likely a chassis/engine/turret/gun/radio compact descriptor in the same
  general family as the vehicle one below, not decoded this session).
- The account's actual list of owned tank types (nation + in-nation index, from
  which nation/tier/tank-tag are derivable) is now **fully decoded** -- see
  below.

## Vehicle compact descriptor -- fully decoded, byte-verified

Two related formats exist side by side, both defined in `common/items/
vehicles.py` and `common/items/__init__.py` (`wot-src`), and both confirmed
byte-for-byte against this account's own captured data.

### Binary form (`inventory[1][vehInvID]["compDescr"]`)

This is what a specific owned vehicle *instance* carries (type + full module
fitting). Decode logic: `common/items/vehicles.py`, `_splitVehicleCompactDescr`
(and the inverse, `_combineVehicleCompactDescr` / `VehicleDescriptor.
makeCompactDescr`). Layout, little-endian throughout:

```
byte 0        header:  bits 0-3 = ITEM_TYPES.vehicle (always 1)
                        bits 4-7 = nationID
                        bit 0x02 = EXTENDED_VEHICLE_TYPE_ID_FLAG (vehicleTypeID > 255)
byte 1        vehicleTypeID low byte
byte 2 (opt)  vehicleTypeID high byte -- only present if the extended flag is set
--- components (all in-nation short indices, resolved via g_cache.chassis(nationID)[id] etc.) ---
  chassisID   u16
  engineID    u16
  fuelTankID  u16
  radioID     u16
  turretID    u16   \_ repeated per turret; effectively all tanks have exactly 1
  gunID       u16   /
--- end components ---
flags byte    bits 0-3 = which of the 4 optional-device slots are populated (popcount = how many u16s follow)
              bit 0x10 = enhancements block present
              bit 0x20 = emblems/inscriptions block present
              bit 0x40 = 1 extra legacy/unused byte follows
              bit 0x80 = camouflages block present
optionalDevices: 2 bytes * (number of set bits in flags & 0xF)
[enhancements]:  present iff bit 0x10 -- 1-byte count + 6 bytes each
[emblems/inscr]: present iff bit 0x20 -- 1-byte positions + 6B/entry emblems + 7B/entry inscriptions
[unused byte]:   present iff bit 0x40
[camouflages]:   present iff bit 0x80 -- rest of buffer, 6 bytes/entry
```

Verified byte-for-byte against a real captured blob (vehInvID 102, this
session's account):
```
[33, 51, 103,0, 163,0, 205,0, 11,0, 88,0, 52,1, 7, 102,0, 82,0, 77,0]
 hdr veh chassis engine fuelTk radio turret gun  flags   3 optional devices
```
- `header=33=0x21`: `0x21 & 0x02 == 0` -> not extended; `nationID = 33>>4 = 2`
  (`usa`, see `nations.NAMES` order below)
- `vehicleTypeID = 51` (single byte, not extended)
- `chassis=103, engine=163, fuelTank=205, radio=11, turret=88, gun=308(=52+1*256)`
- `flags=7=0b0111` -> 3 optional-device slots populated, no enhancements/emblems/camo
- `optionalDevices = [102, 82, 77]`

21 bytes in, 21 bytes accounted for exactly -- no leftover, format fully explained.

### Integer form (`vehTypeCompDescr`, e.g. `9265`, `51841`, `67617`)

This is the simpler *tank type only* form, used everywhere a vehicle type (not a
specific owned instance) needs referencing compactly -- quest configs, dossiers,
shop prices, `battlePass.vehiclePoints`, `hero_vehicles`, `recycleBin`, etc.
Confirmed formula, `common/items/vehicles.py:getVehicleTypeCompactDescr` /
`common/items/__init__.py:makeIntCompactDescrByID`:

```
vehTypeCompDescr = ITEM_TYPES.vehicle(=1) | (nationID << 4) | (vehicleTypeID << 8)
```

Note this is *exactly* the binary form's first 2 bytes reinterpreted as one
little-endian `u16` (for the non-extended case) -- which is why, empirically,
just reading `compDescr[0:2]` as a `u16` already gives you the correct
`vehTypeCompDescr` for cross-referencing against the rest of the cache (verified:
`compDescr[0:2] == 13089`, an exact match against an independent occurrence of
`13089` in this same account's `battlePass.vehiclePoints`).

The client tells the two forms apart purely by **Python type**, not a tag byte:
`isVehicleTypeCompactDescr()` just checks `type(x) in (int, long)` vs `bytes`.

28/28 `vehTypeCompDescr`-looking integers collected from this session's capture
decode with `itemTypeID == 1` and `nationID` in `0..10` (WoT's 11 nations),
confirming the formula against live data independent of the source read:
e.g. `9265 = 0x2431` -> nation 3 (`china`), vehicleTypeID 36; `51841 = 0xca81` ->
nation 8 (`sweden`), vehicleTypeID 202; `67617` -> nation 2 (`usa`), vehicleTypeID
264.

Nation order, `common/nations.py`:
```python
NAMES = ('ussr', 'germany', 'usa', 'china', 'france', 'uk', 'japan', 'czech',
         'sweden', ...)
```

**What's still needed to go from `(nationID, vehicleTypeID)` to an actual tank
name/tag** (e.g. "IS-7"): each nation's vehicle list/index, which is shipped
*game data* (not in the decompiled Python source tree) -- `wg-toolkit-cli`'s
bootstrap command already parses similarly-shipped XML for entity defs, so
extending it to index per-nation vehicle lists is the natural next step.

## Shop / economy rules (`res_id` 222 -- `AccountCommands.CMD_SYNC_SHOP`)

Same Resource mechanism, not player-specific -- prices and static economy config:
`battlePassLevelCost`, `berthsPrices`, `camouflageCost`, `crystalExchangeRate`,
`exchangeRate`, `slotsPrices`, `tankmanCost`, `paid*RemovalCost`, and two very
large sections, `goodies` (99.7 KB) and `items` (658 KB, the full priced-item
catalog). Also carries its own `rev`. Confirmed: `CMD_SYNC_SHOP = 300`
(`common/AccountCommands.py`), sent from `client/account_helpers/Shop.py`:
```python
self.__account._doCmdInt3(AccountCommands.CMD_SYNC_SHOP, clientRev, dataLen, dataCrc, proxy)
```
-- same `doCmdInt3` RPC, same request/response/stream plumbing as `CMD_SYNC_DATA`
above, just a different command id and a different per-feature helper
(`self.shop`) issuing it from `onAccountBecomePlayer`.

## Per-vehicle dossier update (`res_id` 223 -- `AccountCommands.CMD_SYNC_DOSSIERS`)

```
(24, [(9265, 1787687471, b[...binary...])])
```
Confirmed: `CMD_SYNC_DOSSIERS = 600`, sent from `client/account_helpers/
DossierCache.py`. The response tuple is unpacked there as exactly
`(actualCacheVersion, dossiersList)`, where `dossiersList` is `[(ownerID,
changeTime, dossierCompDescr), ...]` -- so `24` is the dossier cache's
`actualCacheVersion` (not a count), `9265` is `ownerID` (a `vehTypeCompDescr`,
per the fully-decoded format above), `1787687471` is `changeTime` (Unix
timestamp), and the trailing bytes are the packed binary dossier itself
(per-vehicle battle stats: battles, wins, kills, damage, etc). The dossier
binary format is a separately-known, previously-reverse-engineered structure in
the broader WoT modding community and worth cross-referencing rather than
re-deriving from scratch -- not decoded further this session.

## The recurring `Account` method `0x2B` -- fully decoded: `LaPingerComponent`

Fires on `Account` roughly every 1-2s, the entire time observed, both immediately
after login and steady-state in hangar. Originally unrecognized by our generated
`Account_Client` table; **now fully decoded and confirmed live** after the
bootstrap fix below. Raw live capture, post-fix:

```
<- Entity method: (1595419) LaPingerComponent_pingMeAndThenJustTouchMe {
    a0: Utf8("185.12.240.69"), a1: 32812, a2: 518858105, a3: 15, a4: 1800000 }
```

`LaPingerComponent` (`la_pinger/scripts/component_defs/LaPingerComponent.def`,
confirmed by reading it directly out of the shipped `la_pinger.pkg`, not just the
decompiled source mirror) is a script component *attached to* `Account`:
```xml
<BaseMethods>
  <tillICanGetMySatisfaction>
    <Arg>PYTHON</Arg><Arg>UINT16</Arg>
  </tillICanGetMySatisfaction>
</BaseMethods>
<ClientMethods>
  <pingMeAndThenJustTouchMe>
    <Arg>STRING</Arg>   <!-- ip: a0 -->
    <Arg>UINT16</Arg>   <!-- port: a1 -->
    <Arg>DB_ID</Arg>    <!-- dbID: a2 -->
    <Arg>UINT16</Arg>   <!-- iterations: a3 -->
    <Arg>UINT32</Arg>   <!-- timeout: a4 -->
  </pingMeAndThenJustTouchMe>
</ClientMethods>
<ofEntity><Account/></ofEntity>
```
`a2` (`518858105`) matches this account's `databaseID` from `Account_showGUI`
exactly, `a1` (`32812`+) sits in the same ephemeral port range as this session's
real BaseApp handoffs, `a3=15` iterations and `a4=1800000` timeout (ms) are exactly
the two trailing fields that were previously unidentified byte guesses. The
client-side impl (`la_pinger/scripts/client/LaPingerComponent.py`) forwards
straight into a native engine call (`BigWorld.pingMeAndThenJustTouchMe(...)`) --
the actual ping is done in C++. `LaPingerComponent` is a global static component
(`la_pinger/extension.xml`, always enabled), matching the "fires constantly while
on `Account`" cadence. Round trip: the server periodically tells the client to
ping a candidate periphery/CDN server via `pingMeAndThenJustTouchMe`; the result
presumably comes back via the paired exposed *base* method
`tillICanGetMySatisfaction(PYTHON result, UINT16 ...)` (not directly observed
this session, since it's client-\>server and we didn't trigger/capture one).

## Bootstrap fix: extension "static components" weren't folded into entities

Root cause of the `0x2B` gap (and others like it): `wg-toolkit-cli`'s bootstrap
command only ever read an entity's own `.def` file's `<Implements>` list. But WoT
ships ~16 optional "extension" feature packages at the root of `res/`
(`la_pinger`, `battle_royale`, `comp7`, `frontline`, `story_mode`, ...), each with
an `extension.xml` that can declare `Components/StaticComponents` -- component
`.def` files (same shape as an interface) living under
`<extension>/scripts/component_defs/`, each carrying an `<ofEntity>` tag naming
which entity/entities it folds its methods into. This folding happens at a WG
build step with no trace in either the decompiled Python source or vanilla
BigWorld's C++ `entity_description.cpp` (confirmed by grepping both), so instead
of guessing, this was determined **empirically** against the live capture:

- Enumerated all 18 extension packages under `res/` (`res ls` at root, filter for
  an `extension.xml`), and all 35 `StaticComponents` across them (`DynamicComponents`
  were deliberately excluded -- those attach to specific entity *instances* at
  runtime, e.g. only during a battle mode, not to every instance's static method
  table).
- Of those, exactly two contribute `Account` **client** methods:
  `battle_royale/AccountBattleRoyaleTournamentComponent` (`setTournamentToken`,
  `setParticipants`) and `la_pinger/LaPingerComponent` (`pingMeAndThenJustTouchMe`).
  Appending them, in alphabetical-extension-directory order, right after
  `Account`'s own last (interface-derived) client method (`Account_showGUI` at
  `0x28`), lands them at `0x29`, `0x2A`, `0x2B` -- an exact match for the live
  `0x2B` capture, with zero disruption to any of the other 41 already-correct
  entries (`0x00`-`0x28` unchanged).
- Crucially, this also revealed that component methods are **not** re-sorted
  together with the entity's own size-sorted method table -- `setTournamentToken`
  (var8) landing at `0x29` *before* `setParticipants` (var16) at `0x2A`, ahead of
  `pingMeAndThenJustTouchMe` (var8, `0x2B`) rules out a global re-sort by stream
  size (which would have interleaved them with the entity's own var16 methods and
  shifted `showGUI` off `0x28`). They're simply appended in encounter order
  (extension, then declared-component order, then declared-method order) after
  the entity's own sorted table is finalized -- which is also *why* enabling an
  unrelated extension never perturbs any other extension's or the core entity's
  ids.

Implemented in `wg-toolkit-cli/src/bootstrap/`: `parse.rs` gained
`parse_of_entity()`; `model.rs` gained a `Component` struct and `Model.components`;
`mod.rs`'s `load()` now scans `res/` root for `extension.xml` files (sorted
alphabetically) and parses their `StaticComponents`; `generate_entity_methods()`
appends matching components' exposed methods after the existing sort (not merged
into it, per the finding above); `generate_interfaces()` also emits the method
argument structs for each component (previously only emitted for regular
interfaces). Regenerated `wg-toolkit-cli/src/wot/gen/{entity,interface}.rs`
against the live game resources -- diff is 100% additive (new `Account`/`Avatar`
component methods only; `Vehicle`, `TeamInfo`, etc. untouched since no active
extension currently targets them). Live-reconnect after rebuilding confirmed
`0x2B` now decodes correctly and no `(unknown exposed id)` warnings remain in the
trace at all.

**Residual caveat**: the "alphabetical extension order, appended after the
sorted table" rule is empirically confirmed for exactly the one entity/direction
we had ground truth for (`Account`'s client table). It's applied uniformly to
*all* entities/directions for consistency. Only 4 new methods actually made it
into the generated base/cell tables this way (`StoryModeAccountComponent_
setDevelopmentFeature` on `Account_Base`, `StoryModeAvatarComponent_
setDevelopmentFeature`/`checkPositionForEquipment` on `Avatar_Base`/`Avatar_Cell`)
-- every other Account/Avatar-targeting component method we found (e.g.
`LaPingerComponent`'s own `tillICanGetMySatisfaction`, `FLAccountComponent`'s
`onAccountArenaCreated`, `LSAccountComponent`'s `onArenaCreated`) correctly stays
excluded because it has no `<Exposed>` tag in its `.def` -- i.e. it's a genuine
base-to-base/cell-only RPC never sent to the client, not a gap. The 4 that *were*
added aren't independently cross-validated against a live capture the way `0x2B`
was, since we don't have traffic exercising them this session. If a future
capture shows one of these off by a fixed offset, it's almost certainly an
extension-ordering-among-themselves question, not a wrong mechanism.

## `AccountDebugger` telemetry (client -> server)

`AccountDebugger_accountDebugger_registerDebugTaskResult(a0, a1, a2)` fires
periodically client->server (both in bursts right after login and steady-state
every ~5s). `a1` is always `0` in every capture; `a0` varies per call (task/metric
id, e.g. `6553821`, `19661022`, `32833738`); `a2` varies too. One steady-state
value repeats exactly (`a0=32833738, a1=0, a2=0`), suggesting a fixed per-tick
"heartbeat" debug task alongside occasional one-off task results. Purely
client-side performance/debug telemetry, not relevant to hangar content itself.

Every capture of this call also logs a `remaining data while reading element`
warning with exactly 8 trailing zero bytes unread -- our generated struct for this
method (`Fixed(20)` bytes, 3 fields) is short by one 8-byte field relative to the
real wire size. Doesn't corrupt parsing (bundle.rs correctly discards the extra
bytes) but the struct is incomplete. See "Known gaps".

## Known gaps / bugs to fix

1. **The `doCmdInt3` call itself is never observed.** Right after 3
   `AccountDebugger_...` calls in the same client->server bundle, our reader dies
   with `"Error while reading bundle: failed to fill whole buffer"` and the rest of
   that bundle is lost -- which is almost certainly where `doCmdInt3` (or another
   `doCmd*` variant) lives. Candidates: a genuine multi-packet bundle reassembly
   timing issue, or a Codec bug in one of the `var8`-framed `doCmd*` variants
   (`doCmdIntStr`, `doCmdIntStrArr`, `doCmdIntArrStrArr`, etc. -- several take
   arrays/strings whose Codec impl may not exactly match the real wire format).
   Worth instrumenting further (e.g. dump raw packet bytes on this specific error
   path) before trying to fix blind.
2. **`AccountDebugger_accountDebugger_registerDebugTaskResult`'s generated struct
   is missing a field** -- 8 bytes short of the declared `Fixed(20)` length every
   single time. Low priority (telemetry only) but easy to fix once the real
   4-field signature is known.
3. **`collections.deque` isn't registered in our pickle decoder** -- causes
   `Resource completed but failed to decode as python: unresolved global
   reference` for at least one small resource (`res_id=2`, a chat/action-log
   queue). Low priority, small blast radius.
4. Two resources (`res_id` 221's big `AccountCommands.CMD_SYNC_DATA` cache and
   presumably 222) contain **recursive Python structures** that our serde-pickle
   fork doesn't support (already flagged as a FIXME in
   `wg-toolkit-cli/src/wot/proxy/mod.rs:637-639`) -- decoding still mostly
   succeeded for this capture, so recursion may only affect specific rare
   sub-structures, but worth keeping in mind.

## Hangar entry is 100% client-local (confirmed)

`client/gui/ClientHangarSpace.py`:
```python
self.__space = BigWorld.createSpace(isHangar=True)
self.__spaceMappingId = BigWorld.addSpaceGeometryMapping(self.__space.id, None, spacePath, spaceVisibilityMask)
self.__vEntityId = BigWorld.createEntity('HangarVehicle', self.__space.id, 0,
    _CFG['v_start_pos'], (...), {})
```
`BigWorld.createSpace(...)` and `BigWorld.createEntity(...)` are **local
client-only** engine calls -- no BaseApp/CellApp round trip, no "enter arena"
handshake. The Hangar is not a networked space the server hands you into; it's a
local 3D scene the client builds entirely from already-cached data. Confirms,
from source, what the capture already strongly suggested independently (zero new
entity/property traffic across an extended idle window in hangar).

`HangarVehicle`/`Vehicle` are real declared entity types (`entity_defs/
entities.xml`), but `HangarVehicle.def` has **empty** `Properties`/`Volatile`
sections and no `BaseMethods`/`CellMethods`/`ClientMethods` at all -- it exists
purely so `BigWorld.createEntity()` has a registered client-side class to
instantiate locally; it is never created by a server-side cell. Only one
`HangarVehicle` client entity exists at a time (the currently-displayed tank);
switching the selected tank in the carousel does not create/destroy entities
over the wire, it re-skins the existing local entity in place
(`ClientHangarSpace.recreateVehicle()`), fed from the cached `inventory` data via
`VehicleDescr(compactDescr=...)` (see above). The real `Vehicle` entity type is
what's actually used once you enter a battle arena -- a genuinely different,
server-driven flow not touched by hangar loading at all.

## Open questions for implementing a real client

1. **Rest of the per-vehicle `compDescr` binary blob past the header** -- fully
   decoded now (see above): components, flags byte, optional devices, and
   conditionally enhancements/emblems/camouflages. Nothing left open here
   structurally; only the in-nation index tables themselves (chassis/engine/
   turret/gun/radio/optionalDevice catalogs per nation) are still needed to turn
   an id into a name, same caveat as vehicle names below.
2. **Vehicle/module name resolution**: turning `(nationID, vehicleTypeID)` (or
   the equivalent in-nation module ids) into actual names/tags requires each
   nation's item list, which is shipped *game data*, not present in the
   decompiled Python source tree. `wg-toolkit-cli`'s bootstrap command already
   parses similarly-shipped XML for entity defs, so extending it to index
   per-nation vehicle/module lists is the natural next step.
3. Exact numeric `inventory` category enum beyond `1`=vehicle (confirmed) --
   `common/items/__init__.py`'s `ITEM_TYPE_NAMES`/`ITEM_TYPE_INDICES` has the
   full list, just needs reading; not attempted this session.
4. Exact argument signature of `ClientCommandsPort.doCmdInt3` and friends
   (`AccountCommands.CMD_SYNC_DATA`/`CMD_SYNC_SHOP`'s actual int args --
   `revision`/`crc`/`clientRev`/`dataLen`/`dataCrc` per the call sites above) --
   needed both to correctly *send* these requests from a from-scratch client and
   to fix gap #1 in "Known gaps" below (we never observe the outgoing call in our
   own capture).
5. ~~`LaPingerComponent` wire-id verification~~ -- **resolved**: live-confirmed
   after the bootstrap fix, `0x2B` decodes cleanly as
   `LaPingerComponent_pingMeAndThenJustTouchMe` with sensible field values
   (`a2` = this account's `databaseID`, `a1` in the expected port range). See
   "Bootstrap fix" above.
6. The handful of newly-generated `Account`/`Avatar` **base**/**cell** component
   methods beyond `LaPingerComponent` itself (`StoryModeAccountComponent_
   setDevelopmentFeature`, `StoryModeAvatarComponent_setDevelopmentFeature`/
   `checkPositionForEquipment`) aren't independently live-validated the way
   `0x2B` was -- see the residual caveat in "Bootstrap fix" above. Low priority
   (dev/QA-only methods, unlikely to appear in normal play) but worth
   double-checking against a capture that exercises them if one ever surfaces.
7. Whether the **inter-extension** ordering assumption (alphabetical by
   directory name) generalizes beyond the one confirmed pair
   (`battle_royale` before `la_pinger`) -- still untested for extensions that
   don't happen to share a target entity with another extension in this
   session's data.
