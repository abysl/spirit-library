# spirit — Future Ideas

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

Blue-sky possibilities. Not near-term, not planned. Captured here so they don't get lost.

---

## Deduplicated Downscaling / Scalable Streaming

Store one high-quality base blob. Serve lower-quality variants without storing them separately.

The naive version: run a transformer on demand (e.g., `ffmpeg -vf scale=480:-1`) when a lower-quality request arrives, cache the result as a derived output blob. Lazy materialization of TD→blob.

The ambitious version: use **scalable video coding** (H.265 SVC, AV1 scalable layers, or Bao-style tree slicing). The base layer is independently decodable at low quality. Enhancement layers add detail. A peer can serve only the base layer to a bandwidth-constrained client — no re-encoding, no storing multiple copies. The enhancement layers are additive blobs; a client requesting 1080p fetches base + all enhancement layers; a client requesting 480p fetches only the base.

This maps naturally onto iroh-blobs' Bao encoding: Bao's Merkle tree already lets you verify and stream a sub-range of a blob. If the encoding is designed so that the first N bytes of the blob are a decodable low-quality version (like a progressive JPEG), you get this for free at the blob layer.

---

## Zero-Knowledge Proof Implementation

The attestation record already has a `zk` proof kind slot (see attestations.md). The open question is implementation:

- **zkVM choice** — RISC Zero and SP1 are the leading candidates; both can prove execution of arbitrary programs (Rust, C) over a RISC-V target
- **Program scope** — proving a full Nix build inside a zkVM is expensive; a more practical first target is proving a hash computation or a simple transform (e.g., "this source archive, when extracted and hashed file-by-file, produces this manifest blob")
- **Proof size and verification cost** — STARK proofs are large (~200KB+); SNARKs are smaller but need a trusted setup; both verify in milliseconds, which is fine for attestation checking
- **Practical stepping stone** — `reproducible` attestations (N independent builders, same output blob) are already in the design and require no zkVM; ship those first, add `zk` proofs when tooling matures

---

## Delta Encoding / Binary Diffs Between Versions

Two versions of the same package or file are often 90%+ identical. Store the delta (bsdiff, xdelta, or zstd-dict-based) and reconstruct on demand.

Modeled as a special TD: `{base: blob:<v1-hash>, delta: blob:<delta-hash>, algorithm: "zstd-dict"} → blob:<v2-hash>`. The delta blob is tiny; you only need the base + delta to reconstruct v2.

Most useful for large media libraries where albums get re-tagged, or packages where minor version bumps change only a few files.

---

## Untrusted Node Participation

Allow nodes outside your trust ring to serve blobs (acting as a CDN), without letting them influence your CI/TD→blob index.

Constraint: content blobs are self-verifying (BLAKE3). You can accept a blob from anyone as long as the hash matches. Only the *mapping claim* requires trust. So untrusted nodes can be pure content servers — you look up the output blob from a trusted index, then fetch the bytes from whoever has them.

This unlocks community seeding for popular content: anyone can host blobs without being trusted for correctness, as long as you resolve the output blob from a trusted source first.

---

## Content-Aware Deduplication

Two different CIs whose output blobs share large common chunks (e.g., two packages that vendor the same library). Store shared chunks once.

Requires chunking blobs at content-defined boundaries (Rabin or FastCDC fingerprinting) so that shared regions produce identical chunk hashes regardless of position. iroh-blobs' Bao tree would need to be built over fixed-size chunks rather than a whole-file hash for this to work — a significant protocol change.

---

## Federated Trust Networks

Today: you explicitly list trusted NodeIds. Future: trust webs where you trust a CA-like node that endorses others. You trust Alice; Alice endorses Bob's cache; you transitively trust Bob's TD→blob mappings up to a policy limit.

Modeled as signed endorsement records: `{endorser: nodeid-alice, endorsee: nodeid-bob, scope: "td→blob mappings", expires: ...}` with a chain-length limit to prevent unbounded transitive trust.

---

## CI-Addressed Remaster / Mod Discovery

When a CI is shared, any trusted peer who later publishes a new TD→blob for that CI makes it available to everyone who has that CI in a collection — without the collection author doing anything. This could be made explicit:

- "Remaster" TDs that reference an original CI and a difference process
- A collection preference: `accept_new_tds = true` (default) vs `pin_td = "td:<hash>"` (locked)
- A notification system: "content in your playlist has a new version available, want to switch?"

The mechanism already exists in the architecture; this is about surfacing it cleanly in UX.

## Partial Blob Requests / Range Fetching

Bao encoding already supports this at the protocol level. Surface it as a first-class API: `spirit fetch blob:<hash> --range 0..1MB`. Useful for streaming large video files without downloading the full blob, or for resumable downloads.

## Card printings as content identity (from agni, 2026-08-28)

Trading-card alt arts and reprints are the CIR model in miniature: the card
(name + rules text) is one Content Identity; every printing's art is an output
blob attested to it; `same_as` merges duplicates minted by different sources
(Scryfall, a scanner, another player); and "which printing do I see" is a
per-user resolution policy. agni's card loading is the intended first consumer
— see `agni/plans/hobbit-hand.md`.
