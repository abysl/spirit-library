# spirit — Prior Art and Protocol Comparisons

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

This document evaluates existing systems against spirit's design to establish where spirit is novel, what it borrows, and why specific foundational choices were made. See also the [architecture overview](architecture.md).

---

## The Core Claim

Spirit's distinctive combination is:

1. **CI / output-blob separation** — content identity (what something *is*) is separate from the output blob (the bytes that represent it)
2. **Group-signed attestations** — a signed `(CI, TD) → blob` claim from a group you trust bridges the two
3. **Transform Definition Records** — a content-addressed recipe recording exactly how an output blob was produced from its inputs
4. **Tiered local trust** — Mesh / Cache / Contact levels you set yourself; never delegated

No surveyed system has all four. The closest prior art is Nix narinfo, which independently arrived at a similar structure and validates that the problem is real.

---

## Closest Prior Art

### Nix / NixOS

Nix independently solved the `(recipe → output hash)` problem and is the most instructive comparison.

A Nix derivation describes a build: inputs, builder, arguments. The resulting store path is either input-addressed (hash of the recipe and its inputs) or, experimentally, content-addressed (hash of the actual output bytes). A `.narinfo` file maps a store path to the content-addressed NAR hash plus a download URL — functionally a `(CI, TD) → blob` mapping.

**Where it converges with spirit:**
- A derivation is functionally a Transform Definition Record
- A narinfo is functionally an attestation linking recipe to output hash
- Content-addressed derivations (experimental) separate "what the recipe is" from "what the output is"

**Where it diverges:**
- **CI is implicit.** Nix's equivalent of a CI is an input-addressed store path — a hash of the recipe and all its inputs, not a canonical semantic description of what the content *is*. Two derivations that produce identical bytes have different store paths if their recipes differ. Spirit's CI is an explicit JSON document describing what something *is*, independent of how it was produced.
- **Trust is binary and centralized.** Nix substituters are either trusted fully or not at all. `cache.nixos.org` is the default trusted cache with a single signing key. Spirit's group trust model (Mesh / Cache / Contact) has no analog.
- **Delivery is HTTP-centralized.** Nix binary caches are HTTP servers. Spirit's delivery is P2P via iroh.
- **CA derivations are stagnant.** Content-addressed derivations have been "experimental" in Nix for years. The NixOS RFC (0062) is approved but implementation has not stabilized. Spirit commits to content-addressed identity from day one.

**Verdict:** Nix narinfo is the best existing precedent for spirit's `(CI, TD) → blob` model. Spirit is a generalization: the CI is explicit and semantic (not build-system-implicit), trust is tiered and P2P (not centralized and binary), and delivery is peer-to-peer.

---

### Sigstore / in-toto / SLSA

Sigstore is a signing infrastructure for software artifacts. Cosign signs an OCI digest. in-toto attestations are signed statements linking a subject digest to a provenance predicate (what produced it, what the inputs were). SLSA (Supply chain Levels for Software Artifacts) builds a policy framework on top.

**Where it converges with spirit:**
- in-toto attestations are structurally close to spirit's attestations: a signed claim linking an output hash to its provenance
- SLSA provenance records builder identity and input digests — analogous to a locked TDR
- Sigstore's transparency log (Rekor) provides an auditable record of attestations

**Where it diverges:**
- **Centralized transparency log.** Sigstore's trust model requires Rekor, a centralized append-only log. Spirit's attestation trust is P2P and group-based — no central log required.
- **No CI concept.** Sigstore signs specific digests, not a semantic content identity. A resized image gets a new digest with no relationship to the original.
- **No P2P delivery.** Sigstore is a signing and verification layer; it doesn't address content distribution.
- **Software supply chain focus.** in-toto and SLSA are designed for CI/CD pipelines, not for music libraries or arbitrary content.

**Verdict:** in-toto is the closest analog on the attestation side. Spirit's attestation format should be studied alongside in-toto's predicate model. The key difference is group-based P2P trust vs centralized transparency logs.

---

## P2P Transport and Content Systems

### iroh

Spirit is built on iroh. This section covers what iroh provides and what spirit adds.

**What iroh provides:**
- Ed25519 NodeId identity per device
- QUIC transport with hole-punching and relay fallback
- `iroh-blobs`: BLAKE3 content-addressed blob store and transfer (Bao-encoded, 16 KiB chunk verification)
- `iroh-gossip`: epidemic broadcast trees (HyParView + PlumTree) for pub/sub
- `iroh-docs`: CRDT key-value store over blobs + gossip; range-based set reconciliation
- `iroh-tickets`: serializable tokens encoding content hash + peer dialing info

**What iroh does not provide:**
- Any concept of CI (content identity separate from output hash)
- Signed attestations mapping CI → blob
- Group trust model (Mesh / Cache / Contact)
- Transform Definition Records or reproducible computation
- Version constraint resolution
- Collections of CI references

Spirit is a semantic and trust layer above iroh's transport and blob primitives. iroh provides the pipes; spirit provides the meaning.

**Why iroh over alternatives:** See the full comparison below. Short version: modern QUIC stack, Rust-native, higher NAT traversal success rate than libp2p, BLAKE3 throughout (matches spirit's hash algorithm), approaching 1.0. The n0 team measured 10× connection overhead for equivalent throughput when building on libp2p and started over — this is the most credible benchmark available for the iroh vs libp2p question.

---

### IPFS / libp2p

IPFS is a content-addressed P2P file system. CIDs (Content Identifiers) encode a hash algorithm, a codec (data format), and the content hash — making them self-describing.

**Where it converges with spirit:**
- Content-addressed by hash
- P2P delivery
- CIDv1 encodes the codec, so `dag-cbor` and `dag-json` encodings of the same data produce different CIDs (codec awareness, though not logical identity awareness)

**Where it diverges:**
- **No logical content identity.** An mp3 and a flac of the same song are two completely unrelated CIDs. IPFS has no layer that says "these two CIDs represent the same content." Spirit's CI is that layer.
- **No trust model.** A CID proves you received the bytes you asked for. It does not prove those bytes are a trustworthy version of the content you wanted. There is no group trust registry, no signed attestation linking a content identity to an output hash.
- **No transformation model.** IPFS has no way to express that one CID was produced from another by a specific reproducible process.
- **DHT reliability.** Mainline Kademlia DHT has known reliability and latency problems for cold content. Nodes must serve data they don't care about for the DHT to function well. The 2025 IPFS performance improvements (50–95% bandwidth reduction in Bitswap) are real but don't address the DHT incentive problem.
- **NAT traversal.** libp2p caps at approximately 70% NAT traversal success. iroh's relay fallback achieves near-universal connectivity.

**IPFS interoperability:** Spirit's output blob hash is a BLAKE3 hash. IPFS CIDs can encode BLAKE3 (`blake3` multicodec). Spirit could emit CIDv1 representations of its blob hashes for read-only IPFS gateway compatibility with no architectural commitment — a future reach feature, not a dependency.

**Verdict:** IPFS validates the content-addressed P2P space. Spirit's CI / blob / attestation model fills the gaps IPFS leaves: logical identity, group trust, and reproducible provenance. libp2p is not the right transport foundation given NAT traversal limits and the iroh team's direct measurements.

---

### Hypercore Protocol / Holepunch / Pear

Hypercore is a secure append-only log identified by an Ed25519 public key (the feed key). Corestore manages multiple Hypercores. Autobase adds multi-writer collaboration via causal DAG linearization. Hyperdrive is a filesystem over Hypercores. Pear Runtime (built on Bare, not Node) is a JavaScript app runtime packaging these primitives for P2P desktop and mobile apps.

**Where it converges with spirit:**
- Ed25519 keypair as persistent identity
- Cryptographically authenticated content (append-only log = tamper-evident)
- Multi-writer collaboration (Autobase)

**Where it diverges:**
- **Append-only log, not blob lookup.** Hypercore is optimized for sequential log access. Spirit needs arbitrary blob lookup by hash. These are fundamentally different access patterns.
- **Feed-key identity, not content-addressing.** A Hypercore is identified by its public key (location-style), not by the hash of its content. The same data in two different feeds has no identity relationship.
- **No CI / output-blob separation.** No logical content identity above the byte level.
- **No attestation or trust model.** Trust is tied to key possession; there's no tiered group trust registry.
- **JavaScript-only.** Pear Runtime runs JavaScript (Bare runtime). Spirit is Rust. No native interoperability path.
- **Small ecosystem.** Keet (secure messaging) is the flagship app. Outside of Keet, Hypercore adoption is limited. The Rust implementation (datrs/hypercore) is community-maintained.

**Verdict:** Hypercore is a well-designed append-only log system for social/messaging applications. It is not a fit for spirit's arbitrary blob store + content identity needs, and the JavaScript-only runtime is a hard incompatibility.

---

## Social and Identity Protocols

### AT Protocol (Bluesky)

AT Protocol is a federated social data protocol. Users have a portable DID identity that survives moving between Personal Data Servers (PDS). Records (posts, follows, likes) are signed DAG-CBOR objects in a Merkle tree repository. Blobs (media) are content-addressed by CID (blake2b/sha256) and served via CDN.

**Where it converges with spirit:**
- Portable stable identity (DID ≈ DGID)
- Multiple apps can share the same identity
- Content-addressed blobs (CID on blobs)
- User-controlled data that survives platform migration
- Enables the same peer-to-peer social graph spirit targets

**Where it diverges:**
- **Federated, not P2P.** AT Protocol requires PDS infrastructure — a server hosting your data. Spirit is local-first and P2P. Running spirit requires no server.
- **No P2P transport.** No hole-punching, no direct device-to-device blob transfer. Content is CDN-served.
- **No logical content identity.** A resized image has a different CID. No CI concept linking multiple encodings of the same content.
- **No transformation model.** No equivalent of Transform Definition Records or reproducible computation.
- **No tiered trust.** Blobs are trusted because the PDS signed them. No Mesh / Cache / Contact distinction.
- **Social-data schema.** AT Protocol is designed for posts, follows, and likes. It has no model for software packages, music libraries, or video archives.

**Integration possibility:** Spirit's DGID (Ed25519) could be anchored to an AT Protocol DID, giving spirit users a portable handle (`@you.bsky.social` → your spirit DGID). This would let spirit apps participate in the AT Protocol social graph for discovery without adopting AT Protocol as the transport or content layer. A future integration, not a dependency.

**Verdict:** AT Protocol is the right instinct at the social layer and the wrong choice as a foundation. "If AT Protocol is decentralise Twitter, spirit is decentralise the content layer everything runs on top of." The DID integration path is worth revisiting once spirit has a stable DGID format.

---

### Nostr

Nostr is an extremely simple event-based protocol: secp256k1 keypair = identity, everything is a signed JSON event relayed via WebSocket. Blob handling (NIP-94, NIP-96, Blossom) is HTTP-based with hash verification.

**Where it converges with spirit:**
- Keypair as identity
- Signed content
- Simple, composable

**Where it diverges:**
- No P2P transport (WebSocket relays)
- No logical content identity
- No transformation model
- No group trust levels
- Blob infrastructure is bolted-on HTTP

**Verdict:** Nostr's simplicity is its strength and its limit. Too simple to provide spirit's primitives. Not a fit as a foundation; not a significant interoperability target.

---

### Secure Scuttlebutt (SSB)

SSB is an offline-first append-only log protocol with gossip replication via social follows. Each feed is identified by an Ed25519 keypair.

**Where it converges with spirit:**
- Offline-first design
- Gossip-based replication
- Keypair identity

**Where it diverges:**
- Append-only log semantics (not blob lookup)
- Social graph as trust model (not tiered groups)
- No logical content identity or transformation model
- Small community (~30k users across six social networks)
- No active growth

**Verdict:** SSB pioneered offline-first P2P social but has not grown beyond a committed small community. The append-only log model is a poor fit for spirit's arbitrary blob lookup. Not a relevant foundation or interoperability target.

---

## Build and Artifact Systems

### Bazel Remote Cache / Remote Execution API (REAPI)

Bazel's REAPI is a content-addressed action cache accessible over gRPC. An "action" (build step) is hashed by its inputs; its result (output digests) is cached and served. Clients ask "do I know the output of this action?" before building.

**Where it converges with spirit:**
- Action hash ≈ locked TDR hash
- Output digests ≈ output blobs
- Cache miss → compute locally, cache hit → fetch from cache
- Content-addressed blob store

**Where it diverges:**
- **Server-side only.** REAPI is a client-server protocol; there is no P2P model.
- **No logical content identity.** The action hash encodes the full recipe; there's no separate CI for "what the output *is*."
- **No group trust.** Trust in the REAPI cache is implicit (mTLS or API key).
- **Build-system specific.** REAPI is designed for build pipelines, not general content distribution.

**Verdict:** REAPI is essentially spirit's TD/blob model constrained to a single trusted build server. Spirit generalizes it: CI is explicit, trust is group-based and tiered, and delivery is P2P.

---

### OCI / Docker Registry

OCI image manifests describe container images as ordered sets of compressed layers, each addressed by digest (sha256). The manifest itself is content-addressed. Sigstore cosign adds signatures over OCI digests.

**Where it converges with spirit:**
- Content-addressed layers
- Manifest describing composition of layers ≈ a simple TD
- Cosign signing ≈ a simplified attestation

**Where it diverges:**
- No logical content identity (a rebuilt image layer is a different digest)
- Centralized registry (HTTP)
- No group trust levels
- No P2P delivery

**Verdict:** OCI is a mature centralized artifact system. Spirit extends the model to P2P delivery, logical content identity, and group-based trust.

---

### BitTorrent / WebTorrent

BitTorrent distributes files via torrent (sha1 or sha256 infohash of metadata). Magnet links encode the infohash. Mainline DHT enables trackerless discovery. BEP46 adds mutable torrents keyed by public key.

**Where it converges with spirit:**
- P2P content delivery
- Infohash content-addressing
- DHT discovery (iroh uses Mainline DHT for NodeId resolution)
- Large deployed infrastructure

**Where it diverges:**
- **No logical content identity.** The infohash is a hash of the torrent metadata, not of the logical content. The same file with different torrent metadata has a different infohash.
- **No trust model.** No signing, no group trust levels, no attestation that a torrent represents what it claims.
- **No transformation model.**
- **Torrent identity is fragile.** Split a torrent differently and you get a new infohash — same bytes, no relationship.

**Verdict:** BitTorrent has the most mature deployed P2P content distribution infrastructure. Spirit inherits some of this indirectly via iroh's use of Mainline DHT for NodeId discovery. BitTorrent is not a content identity system; it is a delivery system. Spirit's output blob could theoretically be served via BitTorrent for reach (infohash over a single-file torrent = the BLAKE3 hash with a wrapper), but this is a future interoperability consideration, not a design dependency.

---

## Summary

| System | Logical content identity | Encoding-independent | Group-signed attestations | Recipe / TD layer | P2P delivery |
|---|---|---|---|---|---|
| **spirit** | **Yes — canonical CIR** | **Yes** | **Yes — tiered Mesh/Cache/Contact** | **Yes — locked TDRs** | **Yes — iroh** |
| Nix narinfo | Implicit (input-addressed) | Yes (store path vs NAR hash) | Centralized, binary | Yes (derivation) | No (HTTP) |
| in-toto / Sigstore | No | No | Single identity, centralized log | Partial (SLSA provenance) | No |
| IPFS / libp2p | No | No | No | No | Yes (DHT, unreliable) |
| iroh-blobs | No | No | No | No | Yes (QUIC, reliable) |
| AT Protocol | No | No | No | No | No (federated HTTP) |
| Hypercore | No (feed-key) | No | No | No | Yes (DHT) |
| Nostr | No | No | No | No | No (WebSocket relay) |
| SSB | No | No | No | No | Yes (gossip) |
| Bazel REAPI | No (action hash) | Via action cache | Server-implicit | Yes (action) | No |
| OCI + cosign | No | No | Single identity | Partial (manifest) | No |
| BitTorrent | No | No | No | No | Yes (DHT, very mature) |

---

## Design Conclusions

**Spirit's CI / blob / attestation / group-trust combination is not replicated anywhere.** Nix narinfo is the best precedent and validates that the problem is real and worth solving. Spirit's design is a generalization: the CI is an explicit semantic document, trust is tiered and P2P, delivery is decentralized.

**iroh is the right transport foundation.** It provides BLAKE3 blob transfer (matching spirit's hash algorithm), QUIC transport with high NAT traversal success, Rust-native APIs, and pre-built gossip and CRDT layers that map directly onto spirit's index gossip and mutable collection needs. The iroh team's direct measurements against libp2p are the most credible available benchmark.

**Multi-transport support is not worth v1 complexity.** Every transport has different connection lifecycle, addressing, and reliability characteristics. A real abstraction layer is a project of its own. Spirit should define a thin transport interface with one implementation and add read-only IPFS gateway bridging as a future reach feature if ecosystem pull demands it.

**AT Protocol is worth watching for identity-layer integration.** Spirit's DGID (Ed25519) could be anchored to an AT Protocol DID, giving spirit users a portable, human-readable handle. This is a future integration possibility, not a v1 dependency.
