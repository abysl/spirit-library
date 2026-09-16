# spirit — Library and Index

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

## What the Index Is

The local index is a queryable database of everything the node knows about — CIs, attestations, collections, and which peers have which blobs. It is separate from the blob store (what bytes are on disk). A node can know about a CI without having any output blob materialized locally.

The index is the answer to "what exists and what does it map to?" The blob store is the answer to "what do I have locally?"

---

## What the Index Stores

| Record type | What it is |
|---|---|
| **CI records** | Every CIR seen from any trusted source, with metadata and tags |
| **Attestations** | Every trusted `(CI, TD) → blob` mapping, with signing group and proof type |
| **Collection records** | Subscribed collections and their current folded state |
| **Have-sets** | Which peers have which blobs — used for blob routing, not trust |
| **Feed subscriptions** | Which `(DGID, name)` feeds this node follows, and last-seen version |
| **DGID documents** | Latest known DGID documents for all groups in the trust registry |

Storage backend: SQLite. Embedded, queryable, well-supported in Rust. The schema is append-mostly — attestations and CI records are immutable once written; group documents and have-sets are updated in place.

---

## Index Gossip

When two Mesh-level peers connect, they exchange index updates since their last sync:
- New CI records
- New attestations
- New or updated collection versions

Gossip is scoped to Mesh-level groups only. Cache-level groups push their CI feeds and attestations on a pull schedule (not bidirectional gossip). Untrusted peers cannot influence the index at all.

This is how new versions propagate passively: you don't need to ask for a remaster or a new package build. When any trusted peer attests it, the record flows to you on the next gossip round.

### Local build propagation

When one of your devices builds a package:
1. It produces an output blob and creates an attestation signed with your group key
2. The attestation gossips to all Mesh peers
3. Other devices' indexes now know `ci:X → blob:Y` is available from the building node
4. The next request for `ci:X` routes to that node — no rebuild needed

Your device mesh IS your personal distributed build cache.

---

## Feed Sync

CI feeds from Cache-level groups (nixpkgs, a music label, a game publisher) are synced on a configurable schedule rather than live gossip. Each sync fetches entries newer than the last-seen version and merges them into the local index, subject to index size limits.

A node that follows many large feeds (desktop, server) builds a rich index. A node with small limits (phone) follows fewer feeds and relies on its local indexer to answer queries on its behalf — see [routing.md](../../../routing/wiki/design/routing.md).

---

## Search and Browse

The index enables local search across all known content:

```
spirit search "uncle iroh"          # find CIs matching artist/title/name
spirit search --kind package "ffm"  # scoped by kind
spirit list collections             # all known collections
spirit resolve ci:<hash>            # show all known attestations for a CI
spirit status ci:<hash>             # which output blobs are local vs remote
spirit feeds                        # list subscribed feeds and last-sync time
```

---

## Open Questions

- **Index eviction policy** — when the index hits its size limit, evict LRU records; attestations for output blobs the node has locally should be pinned (higher priority to keep than remote-only records)
- **Gossip delta protocol** — efficient sync of "what's new since timestamp T"; bloom filter exchange to avoid re-sending known records
- **Feed pagination** — large CI feeds (nixpkgs-scale) need pagination or delta-sync rather than full document transfer; delta indexed by sequence number or content hash
- **Cross-feed dedup** — if two feeds both publish a CI for the same content with different CI hashes, detect via external ID fields (ISRC, IGDB, etc.) and offer a merge prompt
