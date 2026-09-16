# spirit — Developer API (Overview)

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status: pre-implementation sketch.** Nothing here is built yet. These pages
> describe the *intended* developer surface so we can pressure-test it against
> the design before writing code. Signatures are illustrative and will change.
> When something here contradicts a design doc, the design doc wins — open an
> issue so we reconcile them.

## What this is

This folder sketches the API a developer touches when building a CLI tool or an
app on spirit — before that API exists. It is a design instrument: writing the
calls a real app would make is the fastest way to find out whether the
primitives compose the way the design claims they do.

The protocol layers (`core`, `index`, `routing`, `schema`) are unopinionated;
`sdk` is where app-facing convenience lives. The mock surface mixes both — it is
organized by *what a developer is trying to do*, not by crate boundaries. Each
entry notes which crate would own it.

## Who it's for

- **App developers** — music players, manga readers, package managers, file
  browsers — who want a shared content store and cross-app identity without
  running a server.
- **CLI authors** — `oasis`, `bumi`, and the like — who drive resolution and
  builds directly.

If you are implementing the protocol itself, start with the
[design docs](../design/architecture.md), not here.

## The two lifecycles to internalize first

Almost everything an app does is one of these two flows. The
[mock API](mock-api.md) expands both into real calls.

**Read path — resolve a content identity to bytes:**

```
1. core    ci = blake3(canonical(cir))              // address the identity
2. index   atts = index.attestations_for(ci)        // local (ci,td)→blob claims
3. core    atts = atts.filter(trust_policy)          // keep only trusted signers
4. routing blob = resolution_policy.pick(atts)       // quality / cache / trust
   └─ on miss: routing federates to trusted peers (local → cluster → network)
5. blobs   bytes = blobs.fetch(blob)                 // from ANYONE, trusted or not
6. core    assert blake3(bytes) == blob              // self-verify; trust the bytes
```

**Write path — produce bytes for a CI and attest them:**

```
1. author  unlocked_tdr  (recipe with {query}/{ci} inputs)
2. routing locked_tdr = lock(unlocked_tdr)           // pin each input CI→blob
3. runtime bytes = execute(locked_tdr)               // build / transcode / fetch
4. core    blob = blake3(bytes)
5. core    att = sign((ci, td) → blob, group_key)    // content attestation
6. index   index.put(att); gossip.publish(att); blobs.add(bytes)
```

The single rule under both: **trust gates the CI→blob mapping, never the bytes.**
You verify bytes by hash and may fetch them from anyone.

## The two UX mechanisms apps surface

Most apps expose content as **collections** (playlists, package sets, reading
lists, release feeds). Two verbs cover how a user relates to someone else's
collection:

- **Follow** — the default. Trust the collection's owner, pull their ops
  automatically, and optionally *suggest* changes back as a signed op the owner
  can approve and append. The user maintains no variant of their own; they ride
  the living, curated collection.
- **Fork** — make the collection yours: re-sign its ops under your own key while
  retaining the originals for provenance. A fork does not track its source (in
  v1). Forking is for when you disagree with upstream curation.

See [collections.md](../../schema/wiki/design/collections.md) for the model and
[mock-api.md](mock-api.md#collections) for the calls.

## How to read these pages

- [terminology.md](../design/terminology.md) — record names and the blob-hash
  convention these signatures use.
- [mock-api.md](mock-api.md) — the full sketched surface, grouped by task:
  node/session, identity & groups, CIRs, resolution, build & attest, blobs,
  collections (follow/fork/suggest/append), and relations.

## Stability

Pre-1.0, pre-implementation. Treat every name here as a placeholder. The value
is in the *shape* — the arguments a call needs, the order of operations, where
trust enters — not the spelling.
</content>
</invoke>
