# spirit — Records, Collections and Transforms

> The spirit half of retiring the hand-rolled machinery agni and kai grew while
> phase 3 stayed on paper. The agni/kai half is
> [`../../agni/plans/port-to-spirit-primitives.md`](../../agni/plans/port-to-spirit-primitives.md);
> the card-identity slice is already designed in
> [`../wiki/design/identity.md`](../wiki/design/identity.md) and this plan
> implements it rather than restating it.

## Why now

Three primitives are designed and unimplemented — records, attestations,
collections — and in their absence the consumers built substitutes:

| Substitute | Where | Primitive it stands in for |
|---|---|---|
| `refs/<name>` single-hash files | `core/store`, written by hand in agni | Collection Record |
| three near-identical card manifests | `agni-importers` ×3, plus a fourth copy in `spirit-node` | Content Identity Records + catalog |
| `<store>/<game>-images` journals | `agni-importers/art.rs` | content attestations + index |
| `image_url` fields inside manifests | all three manifests | Transform Definition Record |
| `<store>/modules-seeded` + `seed_one` precedence | kai `engine/modules.rs` | collection ops with signer provenance |
| `blob:` prefix parse / hex helpers | `agni-sim/pins.rs`, `agni-engine-host`, kai | typed addresses in `spirit-core` |
| "most-held peer wins" ref replacement | `node/mesh.rs` `best_provider` | trust-gated resolution |

None of it is wasted work — each substitute is a working sketch of the
primitive that replaces it — but it is duplicated per game, unsigned, and
schema-coupled: `mesh::completeness` try-decodes a *card* manifest, so spirit
cannot replicate a record kind it has not been taught.

Two constraints shape every phase below:

- **spirit stays game-free and dependency-thin.** `spirit-node` must keep
  linking on android with no HTTP client and no TLS. Anything that fetches or
  executes arrives through a caller-supplied capability, the way
  `Mesh::set_backfill` already does.
- **Nothing minted is re-minted.** Deterministic CBOR plus a signing scope that
  excludes the proof wrapper means unsigned phase-1 records gain signatures
  later without changing a hash.

One prerequisite the design docs list as missing is already done:
`node_secret` persists the Ed25519 node key at `<store>/identity/key` and
`serve_mesh` binds with it. The degenerate DGID is free.

> **S0–S4 landed 2026-09-04**, together with the two leaks and the module
> version collections (A4). What remains is S5 (transforms), S6 (real groups),
> and the agni-side card work (A2, A3) and trust UI (A5). Checkboxes below are
> updated; `implementation-plan.md` carries the same record.

## Phase S0 — canonical encoding and typed addresses ✅

`spirit-core` gains the two things every later record depends on.

- [x] `canonical::{to_vec, from_slice}` — RFC 8949 §4.2 core deterministic
      encoding: definite lengths, shortest-form integers, bytewise-lexicographic
      map keys, floats rejected. `ciborium` does not emit this; wrap it with a
      canonicalizing writer or vendor a small encoder. Tests: key reordering and
      integer width both round-trip to identical bytes; a float fails to encode.
- [x] Typed addresses: `CiHash`, `TdHash`, `AttHash`, `ColHash` newtypes over
      `BlobHash`, each with `Display`/`FromStr` carrying its `ci:` / `td:` /
      `att:` / `col:` prefix, plus `BlobRef` for `blob:`. `BlobHash` keeps its
      bare-hex form.
- [x] Re-export through `spirit-sdk`.

Done when: agni's `pins.rs` hex parsing and `blob_ref`/`hash_hex` can be deleted
in favour of these (done in the port plan's A1). **1–2 days.**

## Phase S1 — the record types ✅

`spirit-schema` gets its first code: the four records as Rust types over the
canonical encoding, and the CI kinds the first consumers need.

- [x] `Cir`, `Tdr`, `AttestationRecord`, `CollectionRecord` — construct, encode,
      decode, and address (`blake3(canonical(record))`).
- [x] CI kinds: `card`, `card-printing`, `wasm-module`. Rules records stay
      plain content blobs, not CIRs (per identity.md). The `device` kind waits
      for S6 — nothing consumes it until device-group membership exists.
- [x] `AttestationRecord` with an optional proof block, signing scope
      `blake3(canonical(claim fields))` — proof excluded, so S4b adds signatures
      over S4a's bytes.
- [x] A `ModuleManifest` shim: `core/modules.rs`'s record becomes a
      `kind = "wasm-module"` CIR (`name`, `role`, `version`, `abi_version`) plus
      a content attestation to the wasm blob. Keep the old reader for one
      release so existing stores load.

Done when: every record type round-trips, and two processes minting the same
logical record produce identical hashes. **2 days.**

## Phase S2 — a replication envelope spirit understands ✅

Removes agni's schema from spirit and unblocks every later record kind.

- [x] Define the envelope: any manifest/catalog/collection blob carries
      `{kind, refs: [BlobHash]}` (canonical CBOR) alongside its own fields.
- [x] `mesh::completeness` and `pull_ref` count and pull `refs` generically;
      `decode_ref_manifest`'s card-then-module try-decode goes away.
- [x] **Delete `Manifest` / `ManifestCard` from `node/src/lib.rs`** — card
      knowledge in spirit, in violation of its own constitution. `fetch()`
      returns the envelope.
- [x] Legacy path: an old card manifest still replicates, via a compat decoder
      that projects it into the envelope, until agni re-ingests.

Done when: a store containing only module refs and a store containing card refs
both replicate with no card-shaped type in spirit. **1 day.**

## Phase S3 — collections replace refs ✅ (single-signer)

The op-set model from
[`collections.md`](../schema/wiki/design/collections.md), at the smallest size
that serves modules and card sets.

- [x] `CollectionRecord` ops: `add`, `remove`, `reorder`, each an addressable
      blob; a collection is `(owner, name)` plus its op set.
- [x] Local fold to materialized state. Single-signer for now: witness receipts
      and sealed checkpoints are deferred — write the fold so the ordering key
      is pluggable, and order by `(op hash)` until receipts exist.
- [x] `refs/<name>` becomes a pointer to a collection head rather than to a
      manifest blob; the file format does not change, so the mesh needs no wire
      change.
- [x] Follow-set: `Mesh::wanted_names` currently accepts **every name any peer
      advertises**. Gate it on collections you follow, plus explicit `--want`.
- [ ] `spirit-node` CLI: `collection list|show|append`.

Done when: a module collection carries three versions, folds identically on two
nodes, and an unfollowed peer's new ref name is no longer pulled. **3 days.**

## Phase S4 — attestations and the index ✅

Implements identity.md phases 1 and 2 with the record layer now in place.

**S4a — unsigned, single-authority.**
- [x] Content attestations `(ci, td) → blob` minted by the importer path;
      stored as blobs, enumerated by the catalog collection.
- [x] `spirit-index` first code: an in-memory fold over the catalog —
      `ci → [attestations]`, `card ci → [printing ci]`, `external id → ci`.
      Rebuilt at startup and on ref change. SQLite only when a real library
      forces it.
- [x] Resolution: `resolve(ci, policy) -> BlobHash` — owner, then trust, then
      newest TD snapshot.

**S4b — signed.**
- [x] `group-signed` proofs over the S4a claim bytes using the persisted node
      key as a `shared-key` DGID of one.
- [x] A trust registry with mesh / cache / contact levels — one `<store>/trust`
      file of `dgid level` lines, own key always mesh, edited with
      `spirit-node trust <node-id|dgid> <level>`.
- [x] Verify on fold: foreign-signed and unsigned attestations are stored but
      inert.

Done when: a second node's forged rules attestation is stored and never
resolves, and S4a's attestations verify after re-wrapping with proofs, with no
re-minting. **3 days.**

## Phase S5 — transforms

The piece with no design doc yet: making a TDR *executable*, so an importer is a
document rather than a binary. Design decision, to be written into
`wiki/design/transforms.md` in the same change:

**spirit owns the record, the lock, and the attestation. It executes nothing.**
Runtimes arrive as caller-supplied capabilities, exactly like `set_backfill` —
which is what keeps `spirit-node` free of HTTP and TLS on android, and what
keeps "spirit never executes anything" true.

Two TD kinds cover every importer:

```toml
# impure, network, rate-limited — the ONLY step that touches the outside world
[td]
kind     = "http-get"
url      = "https://api.scryfall.com/cards/search?q=set%3Ahob&unique=prints"
snapshot = "2026-09-04T00:00:00Z"
rate_ms  = 100
```

```toml
# pure, deterministic, gas-metered, zero-import wasm
[td]
kind   = "wasm-transform"
module = "blob:<scryfall-cards.wasm>"
args   = "blob:<canonical cbor args>"

[td.inputs]
pages = ["blob:<page-1-json>", "blob:<page-2-json>"]   # pinned = locked
```

- [ ] `Fetcher` capability trait (`fn get(&self, url, rate) -> Result<Vec<u8>>`)
      and `TransformRunner` trait (`fn run(&self, module, request) -> reply`),
      both supplied by the caller. agni supplies `ureq` and its existing wasmi
      host; the browser build supplies `fetch`.
- [ ] **The fetch-plan loop.** A transform's second wave of URLs depends on its
      first output (card list → image URLs), so the runner drives a fixpoint:
      invoke the module, it returns either `Output(bytes)` or
      `NeedInputs([url…])`; the host acquires those, pins them into the TDR, and
      re-invokes. Bounded rounds. This needs **no new sandbox capability** — the
      existing `alloc` + `call(ptr,len) -> packed u64` ABI already carries it,
      so transform guests harden and run exactly like game plugins.
- [ ] `lock(tdr) -> TdHash`: replace every `url` input with the `blob:` it
      fetched. A locked TDR re-runs offline and byte-identically — which is what
      makes transforms testable against golden fixtures in CI.
- [ ] On completion, mint `(ci:<output>, td:<locked>) → blob:<output>` and
      append the outputs to the target collection.
- [ ] `spirit-node run <td:hash>` and `spirit-node lock <file>`.

Done when: `spirit-node run` against a locked TDR reproduces a byte-identical
catalog offline, and re-running the unlocked one produces a new locked TDR with
a fresh snapshot date and the same output when upstream has not changed.
**3–4 days.**

## Phase S6 — groups

Only after a consumer needs a second identity. Device-group membership as a
collection, `dgid:web:` resolution, contact-level trust admitting a friend's
custom cards without letting them rewrite your rules text. Witness receipts and
sealed checkpoints land here, filling the ordering seam left in S3. Not
estimated; unblocked by S3 + S4b.

## Sequencing

S0 → S1 → S2 gates everything and is a week of unglamorous plumbing that
deletes code on both sides. S3 and S4 are independent of each other and can be
built in either order; S5 depends on S1 (records) and S4a (attestations) only.
S6 is optional until a second person joins a mesh.

The consumer port interleaves — see the agni plan. Nothing here changes the
gossip wire format, the ALPN set, or the table protocol.
