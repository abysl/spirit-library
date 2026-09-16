# spirit — Architecture

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

## Design Philosophy

Spirit is built around a small number of coherent primitives that compose. The same mechanism that manages a package set manages a playlist. The same mechanism that introduces two of your devices introduces you to a friend's library. No special cases.

| Primitive | What it is |
|---|---|
| **Blob** | Raw bytes, addressed by a blob hash `blake3(bytes)`. Everything below is physically a blob. |
| **Record** | A canonically-serialized blob with a schema: a **CIR** (Content Identity), **TDR** (Transform Definition), **AR** (Attestation), or **CR** (Collection). Addressed by `ci:`/`td:`/`att:`/`col:`. |
| **NodeId** | An Ed25519 public key — identifies a single device |
| **DGID** | An Ed25519 public key — identifies a logical group of devices; the stable user-facing identity |
| **Attestation** | A signed record — the single trust mechanism; covers content (`(ci, td) → blob`), relation, device-membership ops, and access grants |
| **Collection** | An append-only op-set record of references, reconciled by trusted-witness order |

Names and the blob-hash convention are defined once in
[terminology.md](terminology.md). A "locked TDR" is not a separate type — it is a
TDR with all inputs pinned to `blob:<hash>`; its own blake3 hash is the `td:<hash>`
referenced in the build attestation.

---

## Terminology

The canonical reference is **[terminology.md](terminology.md)** — record names
(CIR/TDR/AR/CR), the blob-hash address convention, the `ci:`/`td:`/`att:`/`col:`/`blob:`
prefixes, the canonicalize-then-hash rule, and the hard warning that spirit's
**CI is not IPFS's CID**. The essentials, for this document:

- **Blob hash** — `blake3(bytes)`; the one address primitive. Records are blobs of canonical bytes; output content is a `blob:`.
- **CIR / TDR / AR / CR** — Content Identity, Transform Definition, Attestation, Collection records.
- **DGID** — an Ed25519 public key identifying a logical group; `dgid:<base58-pubkey>` or `dgid:web:<domain>`.
- **NodeId** — an Ed25519 public key identifying a single device; `nodeid:<base58-pubkey>`.

---

## Layers

```
8. Application        oasis (artifact manager), bumi (package manager), custom apps
7. SDK                DGID auth, deep link handling, cross-app identity
6. Schema             standard CI schemas per kind, attribution, license conventions
5. Collections        playlists, package sets, CI feeds, following/forking
4. Library / Index    local queryable DB: CIRs, attestations, collections, have-sets
3. Routing            query federation, device roles, capability tag filtering, hit-rate
2. Trust / Core       attestations (content, relation, device-op, access grant); DGID trust model
1. Transport / Blob   iroh: QUIC, hole-punching, relay; iroh-blobs: BLAKE3, Bao
```

Spirit implements layers 2–7. Layer 1 is iroh and iroh-blobs. Layer 8 is application code built on spirit.

---

## Crates

| Crate | Path | What it owns |
|---|---|---|
| `spirit-core` | [`core/`](../../core/) | Blob store (blake3, verify-on-read), deterministic-CBOR canonical encoding, typed addresses, the four record types, op-set collections and their fold, identity (the store's Ed25519 key is its DGID), trust levels, the `refs` API, the replication envelope |
| `spirit-schema` | [`schema/`](../../schema/) | The CI kinds: module version collections and card / printing records |
| `spirit-index` | [`index/`](../../index/) | The local fold over collections: `ci → attestations`, CIR back-links, `external id → ci` |
| `spirit-routing` | [`routing/`](../../routing/) | Trust-ordered resolution of a CI to a blob, and provider ranking |
| `spirit-node` | [`node/`](../../node/) | iroh serve/fetch, the peer registry, the gossip mesh, and the read-only HTTP gateway for iroh-less clients; compiles to `wasm32` without the `native` feature |
| `spirit-sdk` | [`sdk/`](../../sdk/) | The public surface; downstream crates depend on this one |

Query federation, device roles, TTL eviction, DGID auth and deep links are
designed in the docs below but not yet built.

---

## iroh Ecosystem

| Spirit layer | Build from scratch | Use from iroh ecosystem |
|---|---|---|
| Transport | — | `iroh` — QUIC, hole-punching, relay fallback, NodeId identity |
| Blob store + transfer | — | `iroh-blobs` — BLAKE3, Bao encoding, content-addressed store |
| Index gossip | — | `iroh-gossip` — topic-based epidemic broadcast (HyParView + PlumTree) |
| Mutable collections | — | `iroh-docs` — CRDT key-value store over blobs + gossip; range-based set reconciliation |
| Query protocol RPC | — | `irpc` — lightweight RPC framework; unary, streaming, bidi |
| Deep link / ticket sharing | — | `iroh-tickets` — content ID + dialing info tokens; QR-friendly |
| LAN discovery | — | `iroh` mDNS feature flag |
| DHT discovery | — | `iroh` mainline DHT feature flag |
| BLE device pairing | — | `iroh-ble-transport` — BLE transport (experimental, AGPL) |
| CIR / TDR / attestation model | **spirit-core** | — |
| All attestation kinds | **spirit-core** | — |
| Routing rules + capability filtering | **spirit-routing** | — |
| Version constraint resolver | **spirit-core / spirit-index** | — |
| Standard CI schemas | **spirit-schema** | — |

`iroh-docs` maps onto mutable collections closely — a collection is a document, items are entries, the owner's DGID keypair gates writes — but spirit overrides its author-timestamp conflict resolution with trusted-witness ordering (see [collections.md](../../schema/wiki/design/collections.md#reconciling-multi-signer-collections)); whether to build on `iroh-docs` or roll our own over blobs+gossip is an implementation decision. `iroh-gossip` handles index gossip delivery; spirit decides what messages contain and which peers are trusted. `irpc` covers the `QueryRequest / QueryResponse` protocol boilerplate.

`iroh-live` and `iroh-roq` implement Media over QUIC (MoQ) for future scalable streaming support — see [future-ideas.md](future-ideas.md).

---

## Design Documents

| Doc | Crate | What it covers |
|---|---|---|
| [wiki/spec.md](spec.md) | — | **The specification — source of truth.** Everything below is superseded by it |
| [wiki/design/terminology.md](terminology.md) | — | Canonical names, blob-hash convention, CID warning, canonicalize-then-hash |
| [core/wiki/design/addressing.md](../../core/wiki/design/addressing.md) | core | CIRs, TDRs, output blobs; recursive attestation; version bounds |
| [core/wiki/design/attestations.md](../../core/wiki/design/attestations.md) | core | All attestation kinds: content, relation, device-membership, access grant; expiry; trust policy |
| [core/wiki/design/groups.md](../../core/wiki/design/groups.md) | core | DGID; trust levels; device-group membership; owned/leased; reconciliation; pairing |
| [index/wiki/design/index.md](../../index/wiki/design/index.md) | index | Local index; gossip; feed sync; TTL eviction |
| [routing/wiki/design/routing.md](../../routing/wiki/design/routing.md) | routing | Device roles; replication classes; capability filtering; query federation |
| [schema/wiki/design/collections.md](../../schema/wiki/design/collections.md) | schema | Collections; CI-sharing rationale; following/forking; reconciliation; CI feeds |
| [sdk/wiki/design/sdk.md](../../sdk/wiki/design/sdk.md) | sdk | Cross-app identity; DGID auth; deep links; app ecosystem conventions |
| [wiki/design/comparisons.md](comparisons.md) | — | Prior art survey; IPFS, AT Protocol, Nix, iroh vs libp2p, and more |
| [wiki/design/future-ideas.md](future-ideas.md) | — | Blue-sky possibilities not targeted in the near term |

---

## Plans

- [plans/implementation-plan.md](../../plans/implementation-plan.md) — phased build plan (stub; to be filled in after design scoping)
