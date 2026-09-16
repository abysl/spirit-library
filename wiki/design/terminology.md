# spirit — Terminology and Naming

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): the record layer is implemented.** The code types in
> the table below exist in `spirit_core::address`, the records in
> `spirit_core::record` and `spirit_core::collection`, and the canonical
> encoding in `spirit_core::canonical`. Names here are no longer cheap to
> change: a rename moves every hash.

> Canonical reference for record names, addresses, and naming conventions used
> across all spirit docs and APIs. When another doc disagrees with this one, this
> one wins. Pre-implementation; names are still cheap to change here.

## Records

A **record** is a structured, canonically-serialized unit of spirit data, stored
as an immutable **blob** and referenced everywhere by its **blob hash**. We say
*record*, not *document*, on purpose: "document" is reserved for the iroh-docs
primitive, and it sidesteps the IPFS-CID collision (below).

| Record | Short | What it is | Address (prose) | Code type |
|---|---|---|---|---|
| Content Identity Record | CIR | what a piece of content *is* | `ci:<hash>` | `CiHash` |
| Transform Definition Record | TDR | how to produce an output from inputs | `td:<hash>` | `TdHash` |
| Attestation Record | AR | a signed claim (content, relation, device-op, …) | `att:<hash>` | `AttHash` |
| Collection Record | CR | an ordered / op-set of references | `col:<hash>` | `ColHash` |

- A **device record** is a CIR with `kind = "device"` — not a separate record type.
- A **checkpoint** is a Collection Record variant (a sealed, `prev`-linked snapshot).
- Full names in prose; short forms (CIR / TDR / AR / CR) only where density demands.

## Addresses: everything is a blob hash

There is one address primitive — the **blob hash** (a BLAKE3 tree root = iroh `Hash`).

- **OH is retired.** Output bytes are just a blob; address them by blob hash like
  everything else. (Old docs said `oh:<hash>`; that is now simply a blob hash.)
- **Let types carry "hash-ness."** Prefer `CiHash` / `TdHash` / `BlobHash` newtypes
  over jamming a `bh` suffix onto every identifier — the type already says it's a
  hash. Reserve a short `_bh` / `_h` suffix for untyped contexts (CLI args, raw
  JSON) where there is no type to lean on.
- **Prose and deep links** use a short type prefix on the hash: `ci:`, `td:`,
  `att:`, `col:` for the typed records, and `blob:` for **opaque content bytes**
  (a flac, a binary, a source archive — what older drafts called `oh:`/"Output
  Hash"). The prefix is a readability/validation hint; the value under any of them
  is a plain blob hash. Everything is physically a blob — the typed prefixes mark
  the spirit records; `blob:` marks raw leaf content.

### ⚠️ "CI" is NOT IPFS's "CID"

In IPFS, **CID** (Content Identifier) *is* a content hash — the address of bytes.
In spirit, **CI** (Content Identity) is a *record describing what content is*, and
its address is a blob hash written `ci:<hash>`. The IPFS concept closest to a
spirit blob hash is a blob hash, not a CIR.

**Never write the token `CID` in spirit docs or code.** It inverts the most
entrenched term in content-addressing and will mislead every new reader. Use
`ci:<hash>` for the address and "CIR" / "Content Identity Record" for the document.

## Identity is over a canonical encoding

A blob hash is over *specific bytes*, so a record's identity depends on its
serialization. To keep identity **format-independent** — the entire point of CI —
split two concerns:

- **Authoring format** — flexible. Author in TOML, JSON, whatever; tooling accepts it.
- **Canonical encoding** — exactly one, pinned. Records are **canonicalized, then
  hashed.** The canonical encoding is **deterministic CBOR** (RFC 8949 §4.2
  Core Deterministic Encoding: definite lengths, shortest-form integers,
  bytewise-lexicographic map key order) — decided 2026-08-28; rationale in
  [addressing.md](../../core/wiki/design/addressing.md#open-questions).

Same logical content → same canonical bytes → same hash, regardless of how it was
authored. This decouples *what you write* from *what gets hashed*, and keeps us
free to add or change authoring formats later without changing any identity.

## A record does NOT embed multiple formats

A CIR is **one** canonical blob with **one** hash. It does *not* carry a map like
`{ toml: hash, json: hash }` — that would leak format into identity, so two
authors using two formats would mint two different CIs for the same content,
breaking content identity. The two legitimate "multiple representations" needs
live elsewhere:

- **Multiple content forms** (a song as flac *and* mp3) → **attestations**. One CIR
  resolves to many output blobs via `(ci, td) → output`. This is the core spirit
  mechanism, not a record feature.
- **Multiple serializations of the record itself** (rare) → a **derived blob**:
  render the canonical record to TOML/JSON on demand, or store a rendering as its
  own blob related to the canonical one by a transform. Never part of the identity.

## API naming conventions

- A verb returns the address of what it creates:
  `attest(ci: CiHash, td: TdHash) -> AttHash`.
- Collections are reconciled op-sets ordered by witness receipts — **append, don't
  chain**:
  - `append(col: ColHash, op: Op)`
  - `seal(col: ColHash, prev: CheckpointHash, ops) -> CheckpointHash`  *(optional, tamper-evident checkpoint)*
- Parameters that take a blob hash are typed (`CiHash`, …), not stringly-typed
  paths — see addresses above.
