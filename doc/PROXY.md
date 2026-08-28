# The MITM proxy: architecture and safety lessons

`wg-toolkit-cli`'s `wot` subcommand (`wg-toolkit-cli/src/wot/proxy/`, built on
`wg-toolkit`'s generic `net/app/proxy/`) sits between a real WoT client and the real
game servers, decoding traffic for observation/logging while forwarding it. This
document covers the parts of that design that matter for *safety* — what happens when
our own decoding doesn't match reality — since two real incidents this project has hit
both trace back to the same category of mistake.

## Forwarding is unconditional; decoding is not

`App::poll()` (`net/app/proxy/mod.rs`) always forwards the raw bytes it received
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

## Incident: a wrong length guess corrupted a real client's connection

While adding `ENTITY_PROPERTY` decoding (server → client property updates), an
unrecognized exposed id (one belonging to a *dynamic* component — see `doc/ENTITY.md`'s
"Open question: dynamic components") was given a guessed fallback framing
(`ElementLength::Variable8`, "assume one length-prefix byte"), copying the pattern
already used for unrecognized *entity method* ids (`EntityMethodInner::Unknown`).

That pattern is safe for methods because it's empirically confirmed: a previous session
hooked the live client's own `getEntityMethodStreamSize` and observed it return exactly
that same "read one more header byte" sentinel for an unrecognized method id. **The same
confirmation was never done for properties** — it was copied over as a plausible-looking
assumption, and it turned out to be wrong.

The consequence was worse than a missed decode. Guessing the wrong length misframes the
element, which desyncs every subsequent element in that bundle — our reader's idea of
"where the next element starts" permanently diverges from reality for the rest of that
bundle. Live testing confirmed this cascades into misinterpreting unrelated garbage
bytes as *other, unrelated* message ids — observed directly as a string of nonsense
`SwitchBaseApp` events (`Switch base app to: 30.0.0.0:0`, `22.0.0.0:53`, ...), none of
them real. Because `SwitchBaseApp`'s handler uses `patch_raw` (see above), it then
searched for those garbage "address" bytes *inside that same real packet* — and found
them, since they were literally read from there — and rewrote them, forwarding a
corrupted copy of an otherwise completely unrelated, legitimate packet to the live game
client. That corruption is what crashed it.

**Fix** (`EntityPropertyInner`, `net/app/client/element.rs`): an unrecognized property
exposed id is now always a hard `Err`, never a guessed length. `read_length` fails
immediately, before any bytes of that element are consumed, so the bundle reader safely
rolls back and the caller (`receive_bundle`'s `if let Err(e) = ... { error!(...) }`,
`wg-toolkit-cli/src/wot/proxy/mod.rs`) just stops decoding the rest of that one bundle —
which is safe precisely because of the forwarding/decoding isolation above: raw
forwarding was never at risk, only our own logging of the remainder of that bundle.

**The general lesson**: a "confirmed live for X" fallback is not transitively safe for a
structurally-similar-looking Y. Method framing and property framing happen to share an
`ElementIdRange`/sub-id mechanism, but that doesn't mean they share a fallback
convention for ids outside the known table — and guessing wrong here isn't a quiet
no-op, it can actively corrupt real traffic via `patch_raw`. When there's no live
confirmation for a fallback, erroring out (and letting the unconditional raw-forwarding
path carry the real bytes through untouched) is the safe default, not a guess.

## Related: don't let a decode bug panic the whole app either

A separate, earlier incident (`wg-toolkit/src/net/bundle.rs`): a `.read_blob(unread_len)`
call was `.unwrap()`ed on the assumption that `unread_len` (computed from an element's
own *declared* length) could always be trusted. For an element type this project doesn't
fully recognize — e.g. an entity method call with an exposed id outside our generated
tables — that assumption breaks, `read_blob` returns `UnexpectedEof`, and `.unwrap()`
turned that into a hard panic that killed the entire `base` app's forwarding thread for
good (nothing in `wg-toolkit-cli/src/main.rs` restarts it — it just logs "Unexpected hard
error" and the thread ends), silently dropping all base-app traffic for the rest of the
process's life. Fixed by propagating via `?` (with the same rollback-and-return-Err
pattern used elsewhere in that function) instead of unwrapping. Same underlying
principle as above: code that parses data our own static model doesn't fully understand
must degrade to a logged error, never a panic or a guess that can propagate further.
