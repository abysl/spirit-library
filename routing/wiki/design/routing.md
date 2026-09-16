# spirit — Routing and Resource Management

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

## Overview

No node is expected to have complete data. Routing is designed around this: queries return hits, misses with forwarding hints, or partial results. Every node participates at the level its resources allow. The system degrades gracefully when any tier is unavailable.

---

## Device Roles

Nodes declare a role that describes their resource profile and routing priority. Role is included in the node's membership attestation tags so peers can make routing decisions.

| Role | Index | Blob cache | Serves queries for |
|---|---|---|---|
| `leaf` | small | minimal | itself only |
| `node` | medium | moderate | itself + mesh on request |
| `indexer` | large | large | its cluster; proxies feed subscriptions |
| `hub` | full | large | always-on; relay + indexer |

Presets map onto roles:

| Preset | Role | Typical device |
|---|---|---|
| `phone` | leaf | mobile |
| `laptop` | node | personal computer |
| `desktop` | indexer | home machine |
| `server` | hub | always-on homelab or VPS |
| `custom` | — | user-defined limits |

```toml
[device]
preset = "desktop"      # applies role + default limits
# or override individually:
role = "indexer"

[device.limits]
index_max_entries    = 5_000_000
blob_cache_max_gb    = 64
serve_bandwidth_mbps = 100
index_gossip_batch   = 5000     # max entries per gossip round
```

---

## Resource Limits and Eviction

Every resource pool has a configurable cap. When a cap is hit, LRU eviction runs. Pinned records are exempt.

```toml
[device.limits]
index_max_entries    = 100_000   # phone default
blob_cache_max_gb    = 4
serve_bandwidth_mbps = 10
index_gossip_batch   = 200
```

**Eviction priorities (index):** attestations for locally-stored output blobs are pinned; attestations for remote ones evict first. CIRs referenced by subscribed collections are pinned. Expired attestations are evicted immediately on the next TTL sweep regardless of LRU order.

**Eviction priorities (blob cache):** user-pinned blobs never evict; recently-accessed blobs evict last; blobs with no local attestation evict first.

---

## Peer Selection and Attestation Checks

Before sending a query to a peer, the routing layer validates:

1. **Membership attestation exists** — the target node has a valid, non-expired membership attestation in the relevant DGID
2. **Capability tags match** — the node's membership tags include the service required for this query
3. **Access grant exists** (if applicable) — if the requester is using a delegated grant, it is valid and non-expired
4. **Attestation not expired** — all relevant attestations pass the expiry check (with a configurable clock-skew grace window)

A node that fails any check is skipped without sending a request. If a request is sent to a node that is not attested for the requested service, the node drops it — the requester can diagnose the mismatch by inspecting local membership attestations.

This enables fine-grained service topology: a DGID with separate subsets of nodes attested for `cat-pics` and `dog-pics` routes queries to the correct subset without the requester needing to know individual NodeIds.

---

## Query Protocol

Queries are structured requests with content metadata. Responses are typed — not just yes/no.

```
QueryRequest {
  ci: "ci:<hash>",
  constraints: {              // optional — resolver hints
    kind, version_range,
    system, tags, ...
  }
  scope: local | cluster | network
}

QueryResponse {
  Hit(attestation, blob_hints: [NodeId, ...])
  Miss(routing_hints: [NodeId, ...], confidence: float)
  Partial(have: attestation | blob, missing: blob | attestation)
}
```

`Miss` with routing hints is first-class. A node that doesn't have an answer returns suggestions ranked by confidence. The client follows hints without re-broadcasting.

`Partial` covers the case where a node has the attestation but not the blob (or vice versa). The client fetches the missing half from elsewhere.

---

## Tiered Query Federation

Queries flow up a local hierarchy before going to the broader network:

```
phone (leaf)
  └─► desktop (local-indexer, role=indexer)
        ├── local index hit           → return immediately
        ├── subscribed feeds          → query upstream → return
        └── mesh peers (have-sets)   → broadcast → return

if desktop is unreachable:
  phone queries trusted DGIDs directly   (degraded, more hops, still works)
```

`local-indexer` is a runtime token — resolved to the highest-role node currently reachable in the local mesh. No hardcoded NodeId. The topology self-describes via membership attestation tags.

The indexer proxies feed subscriptions for its cluster: a phone doesn't need to subscribe to a large nixpkgs feed directly — it asks its desktop, which has the full feed cached locally.

---

## Content-Routing Rules

A priority-ordered list of rules mapping content metadata to query targets. Rules are evaluated in order; first match wins.

```toml
[[routing.rules]]
when = { ci_kind = "image", tags = ["cats"] }
ask  = ["dgid:cat-indexer", "local-indexer", "mesh"]

[[routing.rules]]
when = { ci_kind = "package" }
ask  = ["dgid:nixpkgs", "local-indexer", "mesh"]

[[routing.rules]]
when = { ci_kind = "music-track" }
ask  = ["dgid:my-music-cache", "local-indexer", "mesh"]

[[routing.rules]]
when = "*"    # default
ask  = ["local-indexer", "mesh", "trusted-caches"]
```

**Target tokens:**

| Token | Resolves to |
|---|---|
| `"dgid:<id>"` | Members of a specific DGID attested for the required service tags |
| `"local-indexer"` | Highest-role reachable node in local mesh |
| `"mesh"` | All mesh peers — have-set broadcast |
| `"trusted-caches"` | All Cache-trust DGIDs |

The rule defines the eligible set. Within that set, candidates are filtered by capability tag match and expiry, then reordered by hit-rate score.

---

## Hit-Rate Optimization

Per `(source-dgid, ci-kind)` pair, track:
- Query count
- Hit count
- Average response latency

At runtime, reorder a rule's `ask` list by weighted score (hit rate × latency penalty). High-hit-rate sources float to the top automatically.

```
dgid:cat-indexer | image  : 1000q  870h  87ms  → score: high
local-index      | image  :  200q  160h   4ms  → score: high (low latency)
mesh             | image  :  500q  120h  210ms  → score: low
```

Scores decay over time so stale performance data doesn't permanently suppress a source that has improved.

---

## Replication Classes and the Self-Optimizing CDN

`role` (above) governs how a node answers *queries*. **Replication class**
governs how much of the content it *follows* it actually stores. The two axes are
independent — a phone is a `leaf`/`light` pair, an always-on server is often
`hub`/`archive`.

| Class | Stores | Pruning / sharding |
|---|---|---|
| `light` | CIRs + attestations only; fetches blobs on demand | nothing to prune — blobs aren't retained |
| `cache` | replicas of blobs it serves or accesses | hot-biased: evicts cold blobs; shards so popular content has the most replicas |
| `archive` | durable replicas across the followed set | retention-biased: keeps cold and rare content; shards for coverage, not just popularity |

Cache and archive are the same machinery with **different pruning/sharding
policies** — cache optimizes for hit-rate, archive for durability and coverage.

### Cooperative replication within a DGID

A user's cache and archive nodes form a group under their own DGID and coordinate
so the group as a whole maximizes replicas and retrievability. Membership tags
already advertise role and class; the group treats its members as one logical
store and places replicas across them rather than each device deciding alone.

### Locality-driven placement

Replica placement follows access. Per `(blob, requesting-region/node)` the group
tracks where content is actually fetched from and pushes replicas **toward the
nodes where that content is most frequently accessed**. Frequently-accessed blobs
converge on the fastest-responding nodes near demand (which, by
[hit-rate optimization](#hit-rate-optimization), then receive even more of those
requests); rarely-accessed blobs are pushed out to the edges — the individual
"mini-CDN" of whoever keeps that content in their library.

The result is a self-optimizing, personalized CDN. The same mechanism scales from
one person's laptop + phone + desktop balancing their own library, up to a
multi-site CDN serving popular content to millions: a small operator's mini-CDN
connects to a larger one for the content they share, and the two optimize replica
placement together — popular content replicated wide and close to demand, long-tail
content held at the edges by whoever cares about it.

- **No global consistency** — two nodes can have different views of what's available; both are correct for their local context
- **Miss-with-hints** — no query dead-ends; every miss returns a forwarding suggestion
- **Blob routing is independent of index routing** — finding the attestation and fetching the blob are separate steps; a node can serve attestations without storing blobs and vice versa
- **Expired attestations are predictable** — routing skips expired nodes before sending; clients and nodes agree on validity without coordination
- **Graceful degradation** — if the local indexer is down, leaves query trusted DGIDs directly; slower, more hops, still functional

---

## Open Questions

- **Role advertisement** — role is declared in membership attestation tags; define canonical tag names for `indexer`, `hub`, etc. and how the routing layer discovers them
- **Multiple indexers** — if two devices both declare `role=indexer`, ask both (fastest answer wins) or define a priority tag
- **Feed proxy scope** — should the local indexer automatically proxy all feed subscriptions for cluster members, or only on explicit request?
- **Routing rule language** — TOML `when`/`ask` is simple but limited; a small expression language handles OR conditions and numeric comparisons without becoming a full DSL
- **Hit-rate score decay** — exponential decay vs sliding window; tune to avoid penalizing temporarily-offline sources
- **Scope escalation** — if `scope=local` misses, auto-escalate to `scope=cluster` then `scope=network`, or require explicit scope in the request?
- **Clock-skew grace window** — how long after expiry does a node continue honouring an attestation? Recommendation: 5 minutes, configurable
- **Cache vs archive policies** — the concrete pruning/sharding algorithms that distinguish the two classes; replication factor targets, eviction signals, shard assignment
- **Replica placement signals** — what locality/access metrics drive "push toward where it's accessed"; how to avoid thrashing replicas on bursty access
- **Cross-CDN coordination** — how one operator's mini-CDN negotiates joint placement with a larger CDN for shared content without ceding control of its own storage
