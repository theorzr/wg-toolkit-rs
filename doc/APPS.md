# BigWorld/WoT server apps, briefly

WoT's server side is a cluster of specialized processes (BigWorld calls them "apps"),
each with their own `Mercury::InterfaceMinder`-based message interface. `wg-toolkit`
only needs to speak the interfaces a real *client* actually touches directly.

## LoginApp (`net/app/login/`)

The first server the client talks to. Handles authentication (`login`) and returns
either an error or the address of a `BaseApp` to connect to next, plus a login key
used to authenticate that follow-up connection. Deliberately tiny/stateless protocol
(`login`, `probe`, `ping`, `challengeResponse`, `mtuProbe`) — it doesn't know about
entities, spaces, or anything gameplay-related.

## BaseApp (`net/app/base/`, server→client half in `net/app/client/`)

The app the client stays connected to for the whole session, independent of whatever
"space" (hangar, battle arena, ...) the player is currently in. Owns the *base* half
of the player's entity (`Account`, persistent state, chat, inventory/tech-tree style
RPCs like `CMD_SYNC_DATA`), streams resources to the client (`ResourceHeader`/
`ResourceFragment`), and can hand the client off to a different `BaseApp` instance for
load-balancing or failover (`SwitchBaseApp` — the client fully disconnects and
reconnects to the new address, confirmed live).

`BaseAppExtInterface` (the "Ext" meaning client-facing, as opposed to an internal,
server-to-server interface this project has no reason to implement) is what
`base/element.rs`'s `id` module encodes. Messages this app can't handle itself
(spatial/gameplay stuff) get forwarded on to the `CellApp` via `sendToCell` /
`nrlMsgToCell`, and the base app relays the cell's replies back down to the client —
*unless* the client has been hand-shaked into a direct connection (see below).

## CellApp (`net/app/cell/`, ids only so far, no socket-server implementation yet)

Owns the *cell* half of the entity: position, orientation, physics, and everything
spatially scoped to one "space" — this is what actually runs the hangar and each
battle arena. Area-of-interest (AoI) updates, avatar movement, and entity methods
that only make sense in a spatial context (as opposed to `BaseApp`'s account-wide
RPCs) live here.

Normally the base app proxies cell traffic for the client. But `client/element.rs`'s
id list also has `SET_CELL_APP_EXT_ADDRESS` and
`LAST_PROXY_MESSAGE_AFTER_DIRECT_CELL_APP_CONNECTION` — strong evidence that, once
established, the base app can also hand the client the cell app's own external
address so movement/AoI traffic (latency-sensitive, high frequency) goes straight to
the cell app instead of being relayed through the base app. `CellAppInterface`'s id
list (`cellAppLogin`, `authenticate`, `avatarUpdateImplicit`, ...) mirrors
`BaseAppExtInterface`'s early ids almost exactly, which fits: it's the same kind of
"Ext" client-facing interface, just for direct cell app traffic instead of
base-app-relayed traffic.

## Apps this project doesn't (and won't need to) implement

A full BigWorld deployment also has purely server-internal apps a client never talks
to directly: `BaseAppMgr`/`CellAppMgr` (spawn and load-balance `BaseApp`/`CellApp`
instances, and for `CellAppMgr`, coordinate space/cell boundaries across multiple
`CellApp` processes for large seamless worlds — WoT's discrete hangar/arena spaces
likely don't stress this much), and a `DBApp`/`DBMgr` for persistent account storage.
These only matter if we ever wanted to emulate the *server* side for real, not just
proxy/decode a real client's traffic.

## Practical implication for `wg-toolkit`

Every element `wg-toolkit` currently decodes or has an id for is on a client-facing
interface: `LoginInterface`, `BaseAppExtInterface` (in `base/element.rs`),
`ClientInterface` (server→client, in `client/element.rs`), and now `CellAppInterface`
(`cell/element.rs`, ids only). If the direct-cell-app-connection theory above is
right, a full "garage/hangar working end-to-end" trace should show a
`SetCellAppExtAddress` message at some point after login, followed by the client's
avatar-update-style traffic moving from the base app's `Out` bundles onto a brand new
UDP channel/socket straight to that address — worth specifically watching for once we
have a live capture.
