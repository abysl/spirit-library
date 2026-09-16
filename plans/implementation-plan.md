# spirit — Implementation Plan

Built incrementally, driven by what agni needs — see
`../agni/plans/hobbit-hand.md` for the active consumer plan.

## Phase 1 — Local blob store ✅ 2026-08-28

- [x] `BlobHash` — blake3, hex display/parse (iroh tree-hash migration noted
      in `core/wiki/design/addressing.md`)
- [x] `BlobStore` — put (atomic, idempotent) / get (verifies on read) / has
- [x] Tests: roundtrip, idempotence, missing, corruption, hex

## Phase 2 — First content ✅ 2026-08-28

- [x] Scryfall ingest example writing card blobs + a ciborium manifest
- [x] `refs/` files as the naming stopgap until collections exist

## Phase 3 — Records and identity ✅ 2026-09-04

- [x] Deterministic-CBOR canonical encoding (RFC 8949 §4.2 profile —
      `core/src/canonical.rs`: sorted map keys, shortest-form integers,
      definite lengths, floats refused)
- [x] Typed addresses — `CiHash` / `TdHash` / `AttHash` / `ColHash` /
      `BlobRef` with their `ci:` / `td:` / `att:` / `col:` / `blob:` prefixes
      (`core/src/address.rs`)
- [x] The four records — CIR, TDR, Attestation and Collection, each
      canonicalized then hashed (`core/src/record.rs`, `core/src/collection.rs`)
- [x] Attestations, signed. The claim is the signing scope and the proof sits
      outside it, so an unsigned phase-1 claim gains a signature without
      re-minting. The persisted node key is the DGID — a `shared-key` group of
      one (`core/src/identity.rs`); `spirit-node`'s `node_secret` now reads that
      same key, so node id and DGID are the same thing
- [x] Trust levels, local and never transitive (`core/src/trust.rs`)
- [x] CI kinds — `card`, `card-printing`, `wasm-module` — and the catalog
      (`spirit-schema`, its first code)
- [x] The index fold: `ci → attestations`, back-links, `external id → ci`
      (`spirit-index`, its first code)
- [x] Trust-ordered resolution (`spirit-routing`, its first code)
- [ ] Card ingest v2 in agni: minting card and printing CIRs from Scryfall and
      Riftcodex rather than the three hand-rolled manifests

## Phase 4 — Sync

- [x] `spirit-node serve` / `fetch` — a store travels between machines over
      iroh-blobs (2026-08-28): tickets per ref, manifest-then-blobs fetch,
      every byte re-verified into the flat store. Verified end to end: 193
      cards pulled into a fresh store, refs and bytes identical.
- [x] `BlobHash` needed no migration - BLAKE3 tree roots are what iroh uses

- [x] A generic replication envelope. `mesh::completeness` and `pull_ref` count
      and pull `{kind, refs}` — or, for a record that declares neither, every
      64-hex string it contains. The card `Manifest`/`ManifestCard` types are
      gone from `spirit-node`, so spirit replicates record kinds it was never
      taught and carries no game knowledge
- [x] Ref names have an owner. `wanted_names` follows only peers trusted at
      cache level or above, and `best_provider` skips the rest, so a stranger
      can no longer introduce a ref name. Bytes stay fetchable from anyone —
      the hash verifies them
- [ ] Deduplicate the flat store against iroh's FsStore (bytes exist twice)
- [ ] Distributed dedup and real groups — per the design docs

## Phase 5 — Collections ✅ 2026-09-04 (single-signer)

- [x] Op-set collections — `add` / `remove` / `reorder`, each an addressable
      signed record; a head record lists its ops, its records and the flat
      closure the mesh replicates
- [x] A local fold ordered by `(seq, op hash)`. The ordering key is the seam
      where witness receipts land; multi-signer reconciliation is not built
- [x] Modules as version collections — `col:modules/<name>` holding a
      `wasm-module` CIR per version, each attested to its wasm blob, resolved
      by version then trust. Publishing over a collection owned by someone else
      carries their items forward under your own key (`forked_from`), so a
      bundled build never erases a mesh install
- [x] `refs/<name>` is now written through one API (`core/src/refs.rs`)
      rather than by hand in every consumer
- [ ] Follow / fork as user-facing verbs, suggestions, sealed checkpoints
