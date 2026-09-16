# spirit — Collections

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): the op-set, the fold and the head record are
> implemented** in `spirit_core::collection`, single-signer, ordered by
> `(seq, op hash)` — the seam where witness receipts will land. Modules are the
> first real collection (`spirit_schema::modules`); a publish over someone
> else's collection carries their items forward under your key and records
> `forked_from`. Follow/suggest, sealed checkpoints and multi-signer
> reconciliation are still design.

## What a Collection Is

A Collection is a named, ordered set of CI references — not output-blob references. Sharing a collection shares *what the content is*, not which encoding or build you happen to have.

The same type handles playlists, reading lists, package sets, game mod lists, and CI feeds from build farms. No special cases.

---

## Why CI-Sharing Matters

If you share an output blob, the recipient gets exactly those bytes forever — that specific mp3 at 128k, that specific build for x86.

If you share a CI, the recipient gets the content identity. Their node resolves it to the best available output blob in their local index. When a trusted peer later publishes a new attestation for that CI (a remaster, a mod, a new build), the collection automatically benefits.

```
You share:       Collection → [ci:<7f3a...>, ci:<4d1e...>, ...]
                              #  ↑ "Leaves from the Vine"   ↑ "Brave Soldier Boy"
                              # ci: values are always hashes — labels live in the collection item

Friend resolves: ci:<7f3a...> → best output blob they have (flac, mp3, whatever)
Later:           trusted peer attests ci:<7f3a...> + td:<remaster-hash> → blob:<new-hash>
Friend's player: automatically finds the remaster next time
```

---

## Collection Record Format

A collection is identified by `(DGID, name)` and is, underneath, an **append-only
op-set**: a set of signed ops (add / remove / reorder an item) that every node
folds into the same materialized state by [trusted-witness order](#reconciling-multi-signer-collections).
The TOML below shows that *materialized* view — the current items — not a single
file you overwrite; an edit appends an op, it does not rewrite the whole record.
Optional [sealed checkpoints](#collection-structure) compact the op history for
fast sync. See also [Following and Forking](#following-and-forking).

```toml
[collection]
kind = "playlist"
name = "uncle-iroh-favorites"
owner = "dgid:abc123..."
created = "2026-05-20"
description = "Uncle Iroh songs for late night coding"
forked_from = "dgid:xyz.../uncle-iroh-favorites"   # optional — tracks fork origin

[[items]]
ci = "ci:..."
label = "Leaves from the Vine"
default_td = "td:..."     # author's recommended version (UX default only)

[[items]]
ci = "ci:..."
label = "Brave Soldier Boy"
default_td = "td:..."
```

```toml
[collection]
kind = "package-set"
name = "dev-tools"
owner = "dgid:abc123..."

[[items]]
ci = "ci:..."
default_td = "td:..."    # specific nix derivation for aarch64-darwin
```

```toml
[collection]
kind = "recipe"
name = "tea-recipes"
owner = "dgid:uncle-iroh..."
description = "My finest blends, for Zuko"

[[items]]
ci = "ci:..."
label = "Ginseng Tea"

[[items]]
ci = "ci:..."
label = "White Dragon Brew"
```

```toml
[collection]
kind = "ci-feed"
name = "nixpkgs-packages"
owner = "dgid:nixpkgs..."
description = "All packages built by the nixpkgs cache"
# items updated continuously as new packages are built
```

---

## Default TD and Resolution Policy

Two layers for "which version do I get?":

1. **Default TD** — the collection author's recommended version for each item. UX default — shown first, plays immediately on tap. The author says "this is the remastered flac, start here." Optional; if absent, resolution policy applies.

2. **Resolution policy** — the recipient's global or per-collection fallback. Fields: quality preference, cache preference (prefer locally available), trust preference (prefer more attestors).

The recipient can override the default TD per-item in their own view without modifying the shared collection.

---

## Following and Forking

Two verbs cover how you relate to someone else's collection. **Following is the
default**; forking is the exception for when you reject upstream curation.

### Follow

Following means: trust the collection's author and pull every new head
automatically. New versions appear live in your app — no merge step, because you
are reading the author's chain, not maintaining your own. This is the experience
we expect almost everyone to have: subscribe to a living, curated collection and
receive corrections and new content as the curators publish them.

We do not expect most people to maintain a private variant of a public
collection. Instead we expect **communities and libraries** to run curated,
always-ongoing public collections with a quality bar for metadata, where members
*suggest* changes rather than fork.

**Suggesting changes (curation loop):** a follower proposes an edit by sending the
author a **signed attestation** describing the change (add/remove/reorder an item,
or a CI relation such as a correction). The author reviews pending suggestions in
their app, approves the ones they want, and the approved edits become the next
ops the owner signs — which then propagate to every follower. The follower never
mutates the shared collection directly; the owner remains the single signer of
the canonical op-set.

### Fork

Forking means: take ownership. You create a new collection that **re-signs every
attestation under your own key**, while **retaining the original signatures** for
provenance and attribution. `forked_from` records the origin head you branched
from.

In v1 a fork is a clean break — it does not track or pull from its source, and
there is no merge. Keeping the original signatures alongside your own is what
makes the post-MVP features tractable: because both lineages are preserved and
the chains are append-only, later versions can fast-forward a fork to a newer
upstream head, rebase local edits onto it, or run overlay/auto-merge — the same
moves git makes, deferred until they're needed. See
[future-ideas.md](../../../wiki/design/future-ideas.md).

Fork when you disagree with the upstream curation process itself. Otherwise,
follow and suggest.

---

## CI Feeds

A CI feed is a collection where the items are CIs published by a group as they become available — typically a build farm announcing newly built packages, or a publisher announcing new releases.

Following a feed (adding the group at Cache trust level):
- Their CI records flow into your local index
- Their attestations become visible to your resolver
- `spirit lock update` automatically sees new versions matching your constraints

This is how your package resolver learns about `ffmpeg 1.1.0` without you manually knowing it exists — the build farm published a CI feed entry, it gossiped into your index, and the resolver found it on the next lock update.

---

## Collections of Collections

Collections can reference other collections by snapshot hash or by `(DGID, name)`. A library is a collection of playlists; a package environment is a collection of package sets. Cycle detection required.

---

## Collection Structure

A collection is an **append-only op-set**, not a strict linear blockchain. Each
edit is a signed op (`add` / `remove` / `reorder` an item) appended to the set; the
current collection is the set folded by [trusted-witness order](#reconciling-multi-signer-collections).
We avoid a mandatory `prev`-linked chain because the order comes from the witness
fold, not from chain position — a single linear head would just create contention
between the owner's own devices (all of which hold the `shared-key`).

- **Append-only** — an edit appends an op; ops are immutable and addressable by hash.
- **Owner-signed** — every op is signed by the owner DGID. Followers' suggestions are folded only when the owner approves and re-signs (see [Following](#following-and-forking)). Multi-owner collaboration is out of scope for v1.
- **Verifiable history** — the op-set plus witness receipts make the full edit log auditable and replayable.

**Sealed checkpoints (optional).** Periodically the owner may `seal` a run of
finalized ops into a `prev`-linked block — a compact, tamper-evident snapshot for
fast sync (a new follower starts from the latest checkpoint instead of replaying
every op) and for publishing a single head hash to an RSS feed or a public
blockchain. A checkpoint is a snapshot of an order the fold already settled — not
a consensus step, and not required.

```
ops (the source of truth):   add(A) … add(B) … reorder … remove(B) …   (folded by witness order)
checkpoints (optional):      seal#0  ◄── seal#1  ◄── seal#2   (prev-linked, compact snapshots)
```

## Reconciling Multi-Signer Collections

A `shared-key` DGID (the MVP scheme — see
[groups.md](../../../core/wiki/design/groups.md#group-signing-schemes)) puts the
*same* owner key on several devices, so two of the owner's own nodes can append
concurrently. The collection is then not a strict linear chain but a set of
owner-signed ops. Reconciliation is **local and CRDT-style** — no adoption round,
no voting; every node folds the same set into the same order:

1. **Witness receipts** — when an owned (trusted) device first sees an op it emits
   a small signed `(node, op, observed_at)` receipt and gossips it. These receipts
   are the ordering authority; the op's own self-asserted `timestamp` and any
   leased node's receipt are informational only.
2. **Order** — an op's effective order-time is `min(observed_at)` over owned
   receipts; ties break on lower `blake3(canonical(op))`. A late message only
   lowers a minimum — never a network reorg.
3. **Finality / GC** — order freezes once an op is witnessed by a majority of
   owned devices or is older than a configured Δ; redundant receipts are then
   dropped. This is where losing candidates are discarded — by garbage-collecting
   a settled set, not by a vote.

Folding yields the same materialized state on every node — last-writer-wins per
item, where "last" is decided by *trusted* observation, not by whoever signed the
op. This is the general rule for any collection owned by a multi-holder DGID; the
**device-group collection** (membership) is its most important instance. Full
detail, including optional sealed checkpoints, is in
[groups.md](../../../core/wiki/design/groups.md#reconciling-concurrent-appends).

## Device Groups Are Collections

A DGID's membership is itself a collection of this kind: items are **device records**
(CIRs with `kind=device`), edited by `add`/`amend`/`revoke`/`reinstate` ops, folded
by trusted-witness order. It reuses this whole machinery — append-only history,
current-state folding, multi-signer reconciliation — with no special cases. The
full model lives in [groups.md](../../../core/wiki/design/groups.md#device-membership).

---

## Open Questions

- **Suggestion format** — the signed proposal a follower sends an owner; structured edit ops (add/remove/reorder item, attach relation) the owner counter-signs
- **Op vs checkpoint propagation** — followers need "what changed since op/checkpoint N"; how often to seal checkpoints, and how a new follower bootstraps from the latest checkpoint + trailing ops
- **Current-state discovery** — how a follower finds the latest folded state: owner mesh query, gossip topic, RSS, or anchored checkpoint; and how stale-state/eclipse is mitigated
- **Resolution policy format** — per-item TOML fields vs a separate policy record vs global config
- **Collection identity** — addressed by `(DGID, name)`, or also given its own `col:<hash>`? A col hash enables dedup of identical shared collections; simpler without
- **Friend privacy** — contacts can see only collections you explicitly share; contact lists are private by default
- **Feed size limits** — a CI feed from nixpkgs could have millions of entries; need pagination or delta-sync rather than full transfer on each update
