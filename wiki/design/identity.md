# Card Identity and Attestations — From Blob Store to Rules Objects

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> Status: design. Written for the first consumer: agni's card scripts, which
> need "Lightning Bolt" to exist as a stable rules object that any number of
> printings, arts and editions point at. Naming follows
> [terminology.md](terminology.md) — what the prompt for this work called
> "assertions" are spirit's **Attestation Records**, and this doc uses the
> canonical term throughout. This is the design that
> [`plans/implementation-plan.md`](../../plans/implementation-plan.md) Phase 3
> ("Records and identity") implements.

## The problem, stated plainly

Today a card is a JPEG. The Scryfall ingester writes each image as a blob,
lists `{name, mana_cost, type_line, oracle_text, image}` in a CBOR manifest,
and points `refs/hob` at the manifest's hash. kai deals by picking rows out of
that manifest. There is no datum that *is* the card: the name is a display
string, the oracle text is a convenience copy, and two printings of the same
card are two unrelated rows. A card script cannot bind to any of this — it
would be binding to one scan of one printing from one ingest run.

Separately, a mesh where nodes will soon claim things ("this blob is the art
for that card", "this card's rules text changed") has node identity — iroh
node ids are Ed25519 keys — but nothing signs data at rest, and the node key
itself is freshly generated every process start (`Endpoint::bind` with no
persisted secret). Whoever gossips a ref first defines it.

Both gaps are the same gap. Spirit's design layer already has the answer
sketched: Content Identity Records for what things are, attestations for who
claims what. None of it is implemented. This doc pins the card-shaped slice of
that design tightly enough to build, and sequences it so the first phase is
small, unsigned, and single-authority — while every byte it mints survives the
later phases unchanged.

## What exists to build on

| Layer | State | What it gives this design |
|---|---|---|
| `BlobStore` (`core/store.rs`) | implemented | content-addressed home for every record this doc defines |
| `refs/*` files | implemented | named entry points; the replication unit the mesh already moves |
| gossip mesh (`node/mesh.rs`) | implemented | `RefAdvert { name, manifest, total, held }` — pull-based convergence for anything a ref names |
| Scryfall ingest (agni's `importers/`) | implemented | the minting pipeline; today records none of `oracle_id`, `set`, `collector_number` |
| deterministic CBOR | decided, unimplemented | the canonical encoding every record hash depends on ([terminology.md](terminology.md)) |
| CIR / AR model | designed, unimplemented | [addressing.md](../../core/wiki/design/addressing.md), [attestations.md](../../core/wiki/design/attestations.md) |
| DGID / groups | designed, unimplemented | [groups.md](../../core/wiki/design/groups.md) — the eventual trust root |

The design rule this doc follows: **card identity is not a new mechanism.** It
is the first real instantiation of the CIR + attestation model, exactly as
[future-ideas.md](future-ideas.md#card-printings-as-content-identity-from-agni-2026-08-28)
anticipated. Where the general design left options open, this doc picks; where
the general design is heavier than the first consumer needs, this doc phases
it — it does not fork it.

## Identity model

Two record kinds, both CIRs, both minted deterministically by the ingester.

### The card CIR — the rules object

```toml
[ci]
kind = "card"
game = "mtg"
name = "Lightning Bolt"

[ci.external]
scryfall_oracle_id = "4457ed35-7c10-48c8-9776-456485fdf070"

# ci:<hash> = blake3(canonical(above)) — this is what a card script binds to
```

The datum is *the card as a rules object*; the key is the CIR's blob hash over
canonical CBOR. Three candidate identity schemes were considered:

- **Scryfall `oracle_id` alone.** Exact, free, and already the industry's
  answer to "same card, many printings." But it delegates spirit's identity
  namespace to Scryfall — custom cards, other games, and any future falling
  out with Scryfall's model would need a parallel scheme.
- **`name` + rules-text hash.** Puts the rules in the identity, so every
  errata mints a new card. That inverts the requirement: identity must
  *survive* errata. Rejected.
- **`game` + `name`, with `oracle_id` carried as an external id.** The chosen
  scheme. The name (Scryfall's oracle name, front face for multi-face cards)
  is the identity humans and scripts mean; the oracle_id rides along inside
  the hashed document as a dedup anchor and a bridge back to Scryfall's API.

Because the ingester is the single minting path and the encoding is
deterministic CBOR, **every node that ingests the same card mints
byte-identical CIRs and therefore the same `ci:<hash>` — convergent identity
with zero coordination.** That property is what lets phase 1 skip signatures
entirely: there is nothing to disagree about when everyone derives the same
bytes from the same upstream. The `same_as` relation attestation (already
designed) remains the escape hatch for the rare cases where Scryfall splits or
merges an oracle_id, or a scanner mints an identity the ingester later
duplicates.

Rules text is deliberately **not** in the CIR — see the next section. Version
is also not in the CIR: unlike `ffmpeg 7.1.0` vs `7.2.0`, an erratad Lightning
Bolt is still Lightning Bolt. Rules *revisions* version underneath a stable
identity, which is the whole point.

Custom cards mint the same shape without the `external` table and with an
`owner` field (the owned-CI mechanism from
[sdk.md](../../sdk/wiki/design/sdk.md#owned-cis)): `owner = "dgid:..."` scopes
authoritative claims about that card to its author. Name collisions between
two people's custom "Lightning Dragon" are then two different CIRs — owner is
part of identity for owned cards, and that is correct.

### The printing CIR — set, number, art

```toml
[ci]
kind = "card-printing"
game = "mtg"
card = "ci:<lightning-bolt-hash>"
set  = "hob"
collector_number = "57"

[ci.external]
scryfall_id = "9a8b...-print-uuid"
```

A printing is itself an identity, not just an assertion: "Hobbit #57" exists
whether you hold a 300dpi scan, a 1200dpi rescan, or no image at all.
`(game, set, collector_number)` is the natural key; the art is *not* in the
record, for the same reason rules text is not in the card CIR — a rescan must
not mint a new printing.

"Printing P depicts identity I" is the `card` field — **structural, not
asserted.** The alternative (a standalone `depicts` attestation linking two
CIRs) was considered and rejected for the base case: the link is part of what
a printing *is*, the card CIR's hash is deterministic so embedding it keeps
the printing's hash deterministic too, and one field beats one record plus one
signature plus one index lookup. A signed `depicts`-style relation can still
be layered on later for corrections (a printing minted against the wrong
card), using the existing `superseded_by` relation attestation on the printing
— no new mechanism.

Finishes (foil, etched) are not identity either: Scryfall models finishes as a
list on one printing, and so do we — a finish selects among a printing's
attested art forms, it does not multiply printings.

Sets stay strings (`set = "hob"`) in this pass. A `card-set` CIR kind is a
natural later refinement; nothing here blocks it, and the string is what both
Scryfall and the existing refs already speak.

## Rules text and art are attested content, not identity

Here the existing design pays off completely: the **content attestation**
`(ci, td) → blob` already expresses both "identity I has rules text R" and
"printing P has art A", with provenance, without any new attestation kind.

```
(ci:<lightning-bolt>, td:<scryfall-oracle 2026-08-28>) → blob:<rules-record>
(ci:<hobbit-57>,      td:<scryfall-image  2026-08-28>) → blob:<jpeg>
```

The **rules record** is a plain CBOR blob — the fields the manifest carries
today, minus the image, plus faces for multi-face cards:

```toml
[rules]
name       = "Lightning Bolt"
mana_cost  = "{R}"
type_line  = "Instant"
oracle_text = "Lightning Bolt deals 3 damage to any target."
# multi-face cards: a faces array of the same shape
```

The TD records where the bytes came from and when — kind
(`scryfall-oracle-fetch` / `scryfall-image-fetch`), the upstream id, and the
snapshot date. **Errata is then just a newer attestation**: fresh TD (new
date), new rules blob, new content attestation against the *same* card CIR.
Nothing is edited, nothing is deleted, the identity never moves, and the
resolver prefers the newest trusted rules attestation. Old replays can pin the
exact rules blob they were played under — which agni's architecture doc
already flags as a replay-compatibility requirement.

This resolves the "what's the datum" question cleanly: the CIR is the
*identity*, the rules record is the *current rules text*, and the attestation
is the *versioned link* between them. Scripts bind to the first, interpret the
second, and trust the third.

## Attestations: format, keys, and phasing the proof

The format is the one [attestations.md](../../core/wiki/design/attestations.md)
already pins: a claim whose canonical CBOR is hashed and signed, plus a proof
block, with the signing scope `blake3(canonical(claim-fields))` **excluding
the proof wrapper**. That exclusion is the load-bearing detail for phasing:

- **Phase 1 attestations carry no proof block.** Single-authority, unsigned,
  exactly what the implementation plan's "attestations (content kind only,
  single local key)" allows for a mesh whose trust surface is already "anyone
  you QR-scanned sees everything"
  ([kai multiplayer.md](../../../../../agni/kai/wiki/design/multiplayer.md)).
  An unsigned attestation is honest about what it is: a structured statement
  in a store you already implicitly trust wholesale.
- **Phase 2 signs the same claims.** Because the signing scope never included
  the proof, a signature can be added over a phase-1 claim's existing
  canonical bytes. Nothing minted in phase 1 is re-minted, re-hashed, or
  migrated. This is the strongest argument for adopting the AR shape now
  rather than inventing a leaner interim format.

**Keys.** Attestations ride the iroh node key — `iroh::SecretKey` is
Ed25519 and iroh-base exposes sign/verify, so no second keypair, no new
crypto dependency. Two prerequisites fall out:

1. **Persist the node secret key** (e.g. `<store>/identity/key`). Today every
   process start mints a fresh node id, which silently breaks more than
   signing: kai's identity QR changes every launch and peer registries
   accumulate dead ids. Fixing this is phase 2's first task and is a win
   independent of attestations.
2. **Bootstrap DGID as the degenerate group.** Rather than adding a
   `node-signed` proof kind, the mesh owner's persisted node key *is* the DGID
   — a `shared-key` group of one, which is literally groups.md's MVP scheme.
   Verifiers check a `group-signed` proof against that pubkey. When real
   groups arrive, the same key becomes a real DGID with a membership
   collection, and no attestation ever needs re-signing.

**Storage.** Attestations are records, records are blobs — they live in the
same `BlobStore`, addressed `att:<hash>`, replicated like any blob. No
separate log: the catalog (next section) enumerates them, and immutability
plus content addressing make the attestation set a natural grow-only set —
union merge is idempotent and convergent by construction.

**Conflicts.** Two nodes attesting different rules text for the same card is
not a merge problem — both attestations are kept — it is a *resolution*
problem, answered by local policy in this order:

1. **Owner of the namespace.** For owned CIs (custom cards), only the owner
   DGID's attestations are authoritative — already the rule in
   attestations.md.
2. **Trust level.** Prefer attestations from higher-trust signers (phase 2:
   the configured authority key beats everything; groups generalize this
   later).
3. **Recency.** Among equally trusted claims, prefer the newest TD snapshot
   date — errata resolution falls out of this rule.

Last-writer-wins was rejected as the base rule: wall-clock ordering across an
untrusted mesh is exactly the attack the groups design's witness-receipt
machinery exists to avoid, and card data does not need global convergence —
"both correct for their context" (attestations.md) is acceptable here too.

## Storage, replication, and the catalog ref

Refs and set manifests stay the asset layer, untouched. The identity layer is
**one new ref per game** — `refs/cards/<game>` — pointing at a **catalog**
record:

```toml
[catalog]
kind = "card-catalog"
game = "mtg"
cards        = ["ci:...", "ci:..."]   # card CIRs
printings    = ["ci:...", "ci:..."]   # printing CIRs
attestations = ["att:...", "att:..."] # content attestations (rules + art)
```

Why a ref and not a new ALPN: the gossip design's one-protocol-per-ALPN rule
is about message shapes, not content kinds. A catalog is just a named manifest
of blob hashes — precisely what `RefAdvert { name, manifest, total, held }`
and `pull_ref` already replicate, quiet-down rules and all. A new ALPN would
buy push semantics the pull-based mesh deliberately does not have anywhere
else. If attestation volume ever outgrows periodic catalog republish (it will
not for card data), that is the moment to revisit — not before.

Two implementation consequences, both worth doing on their own merits:

- **Generalize the replication manifest.** `mesh::completeness` currently
  deserializes the card `Manifest` struct to count held blobs — replication is
  coupled to one schema. The fix is a common envelope (a `kind` field plus a
  flat list of referenced hashes) that set manifests and catalogs both
  satisfy, so the mesh replicates any future record kind without learning it.
- **Rebuild deterministically, merge by union.** The catalog is rebuilt from
  the local record set with sorted entries, so independent nodes ingesting the
  same set converge on the same catalog hash; a node holding extra records
  (custom cards) unions and republishes. Ref names still have no owner — a
  hostile peer can advertise a bogus catalog under the same name — but that is
  today's trust surface exactly (an unauthenticated advert costs a failed or
  wasted pull), and phase 2's authority rule plus the groups design are where
  it tightens.

The **index** (spirit-index's first real code) folds the catalog into memory
at startup and on ref change: `card ci → [printing ci]`,
`ci → [attestations]`, `external id → ci`. In-memory is sufficient at
hundreds-of-cards scale; SQLite when a real library forces it.

## How kai resolves, how agni binds

**kai, render time:** identity → printing → art, all local:

1. Deal produces a card *identity* (`ci:<card>`) plus a chosen printing.
2. Printing choice is **resolution policy, not data**: default is "the set
   being played"; a user preference ("always oldest frame", "always the alt
   art I own") overrides per-identity. The policy object is local and never
   gossiped — which printing you see is yours, exactly as future-ideas.md
   framed it.
3. The printing's newest trusted art attestation gives the blob hash; the
   store gives the JPEG; `watch_refs` already re-deals when a set finishes
   syncing.

This also fixes a documented multiplayer wart: hands currently travel as JPEG
bytes inside `CardFace` (hundreds of KB per deal —
multiplayer.md's known limitation). Once identities exist, the wire carries
`(card ci, printing ci)` and every peer resolves art from its own replicated
store; the mesh already guarantees the blobs are (or become) present.

**agni, script binding:** spirit stores and replicates; agni interprets.

- `agni_core::CardFace` (or its successor) gains the card's `CiHash` alongside
  the display fields. `CardId` stays what it is — a table-instance id; the
  identity says what the card *is* across every table, replay, and printing.
- The script registry lives in agni (a game crate, per agni's AGENTS.md — not
  agni-core, which stays a pure state machine, and not spirit, which never
  executes anything): a map from `ci:<card-hash>` to behaviour. Scripts bind
  to the identity, so every printing, art and edition runs the same script by
  construction.
- The rules record's blob hash joins the replay compatibility key, answering
  agni architecture's open question — a replay pins the exact rules text it
  was played under, and an errata cannot silently rewrite history.
- Whether scripts are data or wasm remains agni's decision
  ([agni architecture.md](../../../agni/wiki/design/architecture.md#3-assets-and-plugins-over-spirit));
  nothing in this design leans on it. When scripts become content, a script is
  one more blob attested to the card CIR — the mechanism is already here.

## The trust story, minimally

Today, scanning a QR grants full mesh visibility — stated plainly in kai's
multiplayer.md, deliberate, and unchanged by phase 1: unsigned records in a
fully-visible mesh add no new exposure, because the mesh already replicates
whatever anyone advertises.

Phase 2 draws the first real line: **the mesh owner's persisted node key is
the root authority.** Each node configures one authority pubkey (default:
itself). Attestations signed by it resolve; unsigned or foreign-signed
attestations are stored but inert. That single rule is enough for a personal
mesh where one person runs the ingester, and it is the honest bootstrap for
the groups design — the authority key simply *becomes* the DGID when
membership collections arrive, with contact-level trust then admitting a
friend's custom cards without letting them rewrite your rules text.

What this is not: an invitation system, per-table secrets, or any change to
who can join a table. Those stay where multiplayer.md put them — in the
groups design, later.

## Migration: the hobbit set

The existing store needs no rewriting — blobs are content-addressed and the
old manifest stays valid. Migration is a re-ingest with a wider net:

1. The ingester queries with `unique=prints` (the current default search
   collapses printings — 193 unique of 321 cards; the deferred alt-art
   variants are precisely what printings exist to represent) and starts
   recording `oracle_id`, `id`, `set`, `collector_number` per card.
2. For each card it mints the card CIR (deduped by oracle_id — one CIR, many
   printings), the printing CIR, the rules record, and the two content
   attestations. Already-held images dedup by hash on `put`; at 17MB,
   refetching what does not match is cheap.
3. It writes `refs/cards/mtg` alongside the untouched `refs/hob`. kai prefers
   the catalog when present and falls back to the manifest, so mixed-version
   meshes keep working; the manifest path retires once every node has
   re-ingested.

Re-running the ingest remains a no-op by construction: deterministic CBOR in,
identical hashes out, same catalog hash.

## Phased plan

**Phase 1 — identities.** ✅ 2026-09-04, except the ingest. Deterministic-CBOR
canonical encoding in spirit-core; card/printing CIR and catalog schemas in
spirit-schema (its first code); the generalized replication envelope in
spirit-node; the in-memory fold in spirit-index. Still to do: ingest v2
(`unique=prints`, oracle_id capture, record minting, catalog ref) in agni, and
kai resolving identity → printing → art with the manifest fallback. The
records went straight in signed — phase 2 arrived with phase 1, so nothing was
ever minted unsigned.

**Phase 2 — keys and signatures.** ✅ 2026-09-04. The node secret key was
already persisted; `node_secret` now reads it through
`spirit_core::identity`, so the node id and the DGID are one key. Attestation
and collection-op claims are signed by it as the degenerate shared-key DGID
and verified on fold; `spirit_core::trust` holds the local, never-transitive
levels and `spirit-routing` resolves by them. A second node's forged claim is
stored and never resolves. Wrapping a claim in a proof does not move the
signing scope, so an unsigned claim can still be signed later without
re-minting — there is a test for exactly that.

**Phase 3 — script binding in agni.** Identity on the dealt card and over the
wire (retiring JPEG-bytes-in-faces); the script registry keyed by card CIR in
a game crate; rules-record hash in the replay compatibility key. Explicitly
out of scope: the effect language / wasm decision — this phase delivers the
*binding*, not the interpreter. **Estimate: 4–5 days**, dominated by the wire
change and its session-protocol fallout.

Phases 1 and 2 are spirit + kai; phase 3 is agni + kai. Each is
independently shippable, and nothing in phase 1 is throwaway.

## Open questions

- **Multi-face printings and per-face art.** The rules record carries faces;
  whether art attestations need a face selector in the TD params, or the front
  face suffices until a game needs backs, is deferred to the first double-faced
  set ingested.
- **Schema versioning.** The general CIR question (addressing.md) bites here
  first: a `kind = "card"` schema change would re-hash every identity. Options
  are unchanged (embedded `schema_version` vs a registry per kind); phase 1
  should pick before minting thousands of CIRs.
- **Catalog scale.** One catalog per game is right for one library; a
  per-set sharding scheme is the obvious refinement if catalogs ever dominate
  gossip adverts.
- **Ref-name ownership.** `refs/cards/mtg` is claimable by any peer, like
  every ref today. The groups design's owned collections are the real answer;
  interim, the authority rule means a bogus catalog wastes a pull but cannot
  poison resolution.
- **oracle_id drift.** Scryfall occasionally splits or merges oracle ids;
  `same_as` / `superseded_by` relation attestations handle it, but the
  ingester's dedup-by-oracle_id needs a policy for a changed id on re-ingest.
