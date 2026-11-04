# The MITM proxy: architecture and safety lessons

`wg-toolkit-wot`'s `wot proxy` command (`wg-toolkit-wot/src/proxy/`, built on
`wg-toolkit`'s generic `app/proxy/`) sits between a real WoT client and the real
game servers, decoding traffic for observation/logging while forwarding it. This
document covers the parts of that design that matter for *safety* — what happens when
our own decoding doesn't match reality — since two real incidents this project has hit
both trace back to the same category of mistake.

## Forwarding is unconditional; decoding is not

`App::poll()` (`app/proxy/mod.rs`) always forwards the raw bytes it received
(`forward_raw` defaults `true`, and no handler in this proxy ever calls
`suppress_forward()`), regardless of whether *our own* bundle/element decoding
succeeds. This is deliberate and important: a client and a real server only ever see
each other's actual bytes, byte-for-byte, unless something explicitly patches them (see
below). Our own decode failures are purely an observability problem — a bug there can
make our logs wrong or incomplete, but on its own can't corrupt what either real
endpoint receives.

There is exactly one place that breaks that isolation on purpose: `Peer::patch_raw`
(same file), used by the `SwitchBaseApp` handler to rewrite the embedded base-app
address so the client stays pointed at the proxy. It works by taking a decoded value,
searching for its raw bytes inside the *current* packet, and substituting different
bytes in their place before forwarding that modified copy instead of the original.
This only stays safe as long as the *decoded value driving the search* is genuinely
what the real protocol put there — see the incident below for what happens when it
isn't.

## Unknown property ids are an error, never a guessed length

When `ENTITY_PROPERTY` names an exposed id outside the entity's property table (one that
belongs to a dynamic component, for instance), decoding fails with an `Err` rather than
assuming a length. It fails before any byte of the element is consumed, so the bundle
reader rolls back and the caller stops decoding the rest of that one bundle. Raw
forwarding is unaffected (see above), so only the log of that bundle's remainder is lost.

Entity *methods* do have a fallback for unknown ids: `Variable8`, the same "read one more
header byte" the client's own `getEntityMethodStreamSize` uses. Properties look
structurally similar, sharing the `ElementIdRange`/sub-id mechanism, but they do not share
that fallback, and a wrong length is not a quiet miss. It misframes the element and
desyncs every later element in the bundle. The reader then takes unrelated bytes for other
message ids, including bogus `SwitchBaseApp` messages (`30.0.0.0:0`, `22.0.0.0:53`, ...).
Because the `SwitchBaseApp` handler uses `patch_raw`, it finds those "address" bytes inside
the real packet they were read from, rewrites them, and forwards a corrupted copy of a
legitimate packet to the game client, which crashes it.

The general rule: a fallback that holds for one kind of message is not safe for another
that merely looks similar. Without evidence for a fallback, error out and let the
raw-forwarding path carry the real bytes through untouched.

## Decode errors must not panic the app either

Code that parses data the static model does not fully understand must degrade to a logged
error, never a panic. In `wg-toolkit/src/net/bundle.rs`, an element's declared length is
not trusted blindly: `read_blob(unread_len)` can return `UnexpectedEof` for an element type
that is not fully recognized (e.g. an entity method whose exposed id is outside the known
tables), and that error is propagated with `?`, using the same rollback-and-return-`Err`
pattern as the rest of the function. A panic there would end the `base` app's forwarding
thread for good, since nothing in `wg-toolkit-wot/src/main.rs` restarts it, and all
base-app traffic would stop for the rest of the process.

## Capturing a session and replaying it

With `--capture <file>`, the proxy records every base app packet it receives, in both
directions, to a gzip-compressed capture (`app/proxy/capture.rs` documents the layout).
Packets are stored decrypted, before any handler sees them, so a capture holds
everything needed to decode the session again without its keys. The one packet per
connection that cannot be decrypted, the client's `baseAppLogin`, is stored as received
and flagged as such. The login app is not captured: its traffic is only the RSA login
exchange.

```sh
wot --game-dir <game-dir> proxy --real-login-app <addr> --real-pub-key-path <key> --capture session.wgcap
wot --game-dir <game-dir> replay session.wgcap
```

The login proxy and the base proxy can also run as separate processes, the login proxy
handing each client over to the base proxy's peer address:

```sh
wot --game-dir <game-dir> app proxy --real-login-app <addr> --capture session.wgcap
wot app login --real-login-app <addr> --real-pub-key-path <key>
```

`wot replay` feeds the capture through `app::proxy::replay` to the same `BaseHandler` the
live proxy uses, so it logs the same lines, to the console and to `replay-trace.jsonl`.
Changing a codec and replaying a capture shows the effect of the change on a whole
session, without going back in game. Nothing is sent during a replay. Timings
the handler measures itself, such as how long a command took to be answered, reflect
the replay's speed rather than the session's; the capture keeps each packet's time.

The capture is flushed every second while packets flow, and whenever the proxy has been
idle for a few seconds, so stopping the proxy loses at most the last second of traffic.
A capture cut short this way replays up to its last complete record, with a warning.
