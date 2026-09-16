# spirit — Attestations

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): the content and relation kinds are implemented.**
> `spirit_core::record::{Claim, Attestation, Proof}` — the claim is the signing
> scope and the proof sits outside it, so an unsigned claim gains a signature
> without re-minting. Only the `group-signed` proof kind exists, over a
> single-key DGID. Device-membership ops and access grants are still design.

## Overview

An attestation is a signed claim. Spirit uses four kinds:

| Kind | What it claims | Who signs |
|---|---|---|
| **Content attestation** | `(ci:<hash>, td:<hash>) → blob:<hash>` — this CI, via this recipe, yields these bytes | DGID (group-signed) |
| **Relation attestation** | One CI relates to another — `same_as`, `superseded_by`, or `previous_version` | DGID (group-signed) |
| **Device-membership op** | `add` / `amend` / `revoke` / `reinstate` a device in a group's membership collection | DGID (group-signed); the referenced device-CI carries the device's own signature |
| **Access grant** | A DGID grants a grantee query access to a specific node for specific services | DGID key only |

They all share the same structural principle: a typed claim record whose canonical encoding is signed, plus a typed proof block. The signing scope is always `blake3(canonical(claim-fields))` — the proof wrapper is excluded so the same claim can be re-wrapped without invalidating existing signatures.

All attestations carry an optional `expires` field (RFC 3339). Expired attestations are evicted from the index and excluded from routing without any revocation broadcast.

---

## Content Attestation

Asserts that a CIR, transformed by a specific locked TDR, produces a specific output blob.

```toml
[attestation]
kind    = "content"
ci      = "ci:7f3a..."    # blake3(canonical(cir))
td      = "td:4d1e..."    # blake3(canonical(locked-tdr))
blob    = "blob:2a8f..."  # blake3(output-bytes)
expires = "2027-01-01T00:00:00Z"   # optional

[proof]
kind         = "group-signed"   # or group-multisig, reproducible, zk
dgid         = "dgid:abc123..."
sig          = "base64:..."     # Ed25519 over blake3(canonical({ci, td, blob}))
signer_node  = "nodeid:..."     # audit trail only — not the trust anchor
```

### Proof types

**`group-signed`** — single signature verifiable against the DGID public key. How the DGID key is exercised internally (shared-key, FROST, multisig) is invisible to verifiers — see [groups.md](groups.md).

**`group-multisig`** — M-of-N individual NodeId signatures from group members. Each signature verifies against its NodeId; verifier confirms M signers are current group members. Individually auditable.

```toml
[proof]
kind       = "group-multisig"
dgid       = "dgid:abc123..."
threshold  = 3
signatures = [
  { node = "nodeid:...", sig = "base64:..." },
  { node = "nodeid:...", sig = "base64:..." },
  { node = "nodeid:...", sig = "base64:..." },
]
```

**`reproducible`** — N independent DGIDs each produced the same output blob from the same locked TDR. Emerges for free when multiple builders independently attest the same output. A practical stepping stone before ZK proofs.

```toml
[proof]
kind         = "reproducible"
threshold    = 3
attestations = [
  { dgid = "dgid:...", sig = "base64:..." },
  { dgid = "dgid:...", sig = "base64:..." },
  { dgid = "dgid:...", sig = "base64:..." },
]
```

**`zk`** — a zero-knowledge proof that the transformation was executed correctly. Trustless: math is the trust anchor. The proof asserts that program `program_hash` given inputs matching the locked TDR produces the output blob. DGID is attribution only — not required for acceptance.

```toml
[proof]
kind         = "zk"
system       = "risc0"         # or "sp1", etc.
program_hash = "blob:..."      # hash of the zkVM program proven
proof        = "base64:..."
dgid         = "dgid:..."      # attribution — optional
```

See [future-ideas.md](../../../wiki/design/future-ideas.md) for ZK implementation notes.

---

## Relation Attestation

CIRs are immutable — you never edit one. A **relation attestation**
records that one CI relates to another, signed by a group you trust. The same
mechanism handles three cases that all reduce to "this CI and that CI are
connected": metadata corrections, cross-node dedup, and version lineage.

```toml
[attestation]
kind     = "relation"
relation = "superseded_by"        # same_as | superseded_by | previous_version
from     = "ci:7f3a..."           # subject
to       = "ci:9b2c..."           # object
expires  = "2028-01-01T00:00:00Z" # optional

[proof]
kind = "group-signed"             # same proof types as content attestations
dgid = "dgid:abc123..."
sig  = "base64:..."               # Ed25519 over blake3(canonical({relation, from, to, expires?}))
```

### Relation kinds (v1)

| `relation` | Meaning | Direction | Resolver / UI effect |
|---|---|---|---|
| `same_as` | `from` and `to` denote the same content identity (e.g. two nodes minted CIRs for the same track under different external IDs) | symmetric | Union the output-blob sets of both CIRs |
| `superseded_by` | `from` is corrected/replaced by `to`; `to` carries the fixed metadata | directional | Prefer `to`'s document; `from` stays resolvable |
| `previous_version` | `to` is the immediate predecessor of `from` in a version lineage (e.g. ffmpeg 7.2.0 `previous_version` 7.1.0) | directional | Offer upgrade paths without conflating identity |

A correction does **not** move any bytes. The corrected CIR re-attests the same
output blobs (`(ci:correct, td:same) → blob:same`), so `from` keeps resolving; the relation
only tells trusting nodes which document to prefer. Collections referencing the
old CI update their reference lazily, via the normal follow/suggest flow — see
[collections.md](../../../schema/wiki/design/collections.md).

### Authority

- **Owned CIs** (a CI with an `owner` field, see [sdk.md](../../../sdk/wiki/design/sdk.md#owned-cis)) — only the owner DGID's relation attestations are authoritative.
- **Ownerless CIs** — each node honors relations from groups it trusts at the configured level. Convergence is local, never global: two nodes may follow different correctors, and both are correct for their context.

---

## Device-Membership Ops

Membership is not a standalone dual-signed record — it is a **collection** of device-CIs, mutated by single-signer ops (`add_device` / `amend_device` / `revoke_device` / `reinstate_device`) appended to the device-group log and folded by trusted-witness order (a local CRDT fold over signed witness receipts — see [groups.md](groups.md#reconciling-concurrent-appends)). The two signatures that used to be required are now split across two objects:

- the **device-CI** carries the device's own signature over its `settings` (consent + key control), and
- the **op** carries the DGID signature (the group admitting it).

```toml
[attestation]
kind      = "add_device"
dgid      = "dgid:abc123..."
device_ci = "ci:9b2c..."             # device-signed settings: tags, role, TTL
timestamp = "2026-06-07T18:30:00Z"   # reconciliation ordering
prev      = "blake3:..."             # advisory link to prior head

[proof]
kind = "group-signed"
dgid = "dgid:abc123..."
sig  = "base64:..."                  # DGID key over blake3(canonical(claim-fields))
```

`revoke_device` references the device `pubkey` instead of a `device_ci`. Reinstatement is bounded by the referenced settings block's TTL. The full model — device-CI format, reconciliation, expiry, and pairing — lives in [groups.md](groups.md#device-membership).

---

## Access Grant

A DGID-signed record delegating query access for specific services on a specific node to a grantee. The grantee presents this grant alongside a query; the node validates it against the underlying membership attestation.

```toml
[attestation]
kind     = "access-grant"
grantor  = "dgid:abc123..."
grantee  = "dgid:contact-alice..."
node     = "nodeid:cf-edge-7..."
services = ["image-cache"]
expires  = "2026-09-01T00:00:00Z"   # must not exceed the node's membership expiry

[proof]
dgid_sig = "base64:..."   # grantor's DGID key signs over blake3(canonical(claim-fields))
```

Only the grantor signs. The grantee accepts it by receiving it via gossip or direct delivery.

---

## Trust Policy

Your trust policy maps proof kinds and DGID trust levels to acceptance criteria. It is local — you set it; it is never asserted by remote groups.

```toml
[trust_policy]
group-signed   = { require_dgid_level = "cache" }
group-multisig = { threshold = 3, require_dgid_level = "cache" }
reproducible   = { threshold = 3, require_dgid_level = "contact" }
zk             = { accept_from = "anyone" }   # math is the trust
```

A ZK proof from an unknown DGID is accepted if math validates. A `group-signed` proof is accepted only if the DGID is in your trust registry at the required level.

---

## Trust Model

```
trust_registry: [(DGID, TrustLevel), ...]
```

Effective trusted NodeIds = all devices present in the folded membership collection of a DGID at Cache or Mesh trust level, whose device-CI settings have not expired. The device-group collection (each op DGID-signed) is the authoritative source — see [groups.md](groups.md#device-membership).

Rules:
- Accept a content attestation if its proof kind + signer satisfies your trust policy and the attestation has not expired
- Accept a device-membership op only if it is DGID-signed and, for `add`/`amend`/`reinstate`, its referenced device-CI carries a valid device signature and an unexpired settings TTL
- Accept an access grant if the grantor DGID is at Contact level or above and the grant has not expired
- Accept index gossip only from Mesh-level DGIDs
- Accept content blobs from any NodeId — BLAKE3 verifies them regardless of source
- A blob whose hash doesn't match the attested blob hash is silently rejected

Any node can serve blobs as a CDN — untrusted nodes included — without influencing your index. Resolve the output blob from a trusted content attestation, then fetch bytes from whoever has them fastest.

---

## Storage

Attestations are documents stored in the content-addressed store. The index maintains:
- `(ci, td) → [content-attestation-hashes]` — look up all attestations for a given pair
- `dgid → [attestation-hashes]` — audit all attestations by a given group
- `blob → [content-attestation-hashes]` — find all claims pointing to a given output blob
- `ci → [relation-attestation-hashes]` — relations where this CI is `from` or `to`
- `(dgid, node) → folded-membership-entry` — current device-CI + settings for a node, materialized from the device-group collection
- `(grantor, grantee, node) → access-grant` — active grants

---

## Open Questions

- **Trust policy format** — TOML covers common cases; a small expression language handles "require ZK from strangers, group-signed from my cache" without combinatorial explosion
- **`zk` system support** — which zkVM systems to support in v1; RISC Zero and SP1 are the frontrunners; verification must be fast (milliseconds) per attestation fetch
- **Partial attestation propagation** — how does the `dgid_sig`-only membership invite reach the node securely? See groups.md open questions
- **Revocation document format** — emergency path for invalidating before expiry; exact format TBD
- **Expiry clock skew** — nodes may have slightly different clocks; define a grace window (e.g. 5 minutes) before treating an attestation as expired
- **Relation kind set** — `same_as` / `superseded_by` / `previous_version` cover the known cases; confirm naming and whether `previous_version` should generalize to a typed lineage edge before locking the format
- **Relation cycles and chains** — `superseded_by` and `previous_version` form chains; define how far a resolver follows them and how it detects/handles cycles
