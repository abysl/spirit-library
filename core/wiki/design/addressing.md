# spirit — Addressing Model

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): implemented.** `spirit-core` carries the canonical
> encoding (`canonical`), the typed addresses (`address`) and the CIR / TDR /
> Attestation records (`record`). Transform *execution* — running a TDR to
> produce an output — is still design.

> Naming follows [terminology.md](../../../wiki/design/terminology.md): **records**
> (CIR/TDR/AR/CR) addressed by **blob hash**; raw output bytes are `blob:<hash>`.
> There is no `oh:`/"Output Hash" anymore — an output is just a blob.

## One Rule

```
Content Identity Record ──(attestation)──► blob:<hash>
```

A **Content Identity Record (CIR)** identifies what something *is*. A `blob:<hash>`
is the bytes of a specific form of it. The mapping between them is an attestation —
a signed claim by a trusted group. A **Transform Definition Record (TDR)** is the
third element: it says *how* to get from a CI to an output blob.

`ci:<hash>` is shorthand for `blake3(canonical(cir))` — the address of the CIR
itself. It is always a hash, never a name or slug.

Everything else is this, applied recursively.

---

## Content Identity Record (CIR)

A **CIR** is an immutable record describing what a piece of content *is* — stable
across re-encodings, rebuilds, or format changes.

`ci:<hash>` is its address: `blake3(canonical(cir))`. It never encodes a name or
title — it is always an opaque hash. The record is hashed over **one canonical
encoding** regardless of authoring format (see terminology.md).

```toml
# CIR — authored here in TOML; hashed over the canonical encoding
[ci]
kind   = "music-track"
artist = "Uncle Iroh"
album  = "Tales of Ba Sing Se"
year   = 2006
track  = 1
title  = "Leaves from the Vine"

# ci:<hash> = blake3(canonical(above)) = "ci:7f3a9e..."
# You never write the title into the hash — it's always opaque
```

```toml
# CIR
[ci]
kind    = "package"
name    = "ffmpeg"
version = "7.1.0"

# ci:<hash> = blake3(canonical(above)) = "ci:4d1e8c..."
```

A CIR is minimal: it identifies the thing, not how to get it or what form it's in.
Version is part of identity for software — `ffmpeg 1.0` and `ffmpeg 1.1` are
different CIRs with different `ci:<hash>` values. A song is the same CIR whether
it's stored as flac or mp3.

External IDs (ISRC, ISBN, IGDB, CPE) are optional but enable dedup across nodes
that independently create CIRs for the same content.

**Immutability and corrections.** A CIR never changes — a typo in the artist name
is baked into that `ci:<hash>` forever. You correct it by authoring a *new* CIR,
re-attesting the same output blobs to it (`(ci:correct, td:same) → blob:same`, so
no bytes move), and publishing a `superseded_by`
[relation attestation](attestations.md#relation-attestation). Nothing global has
to happen and nothing is deleted: the old CIR keeps resolving, and nodes that
trust the corrector prefer the new one. The same relation mechanism links
duplicate CIRs (`same_as`) and version lineage (`previous_version`). Note also
that a collection item carries its own `label`, so display text in a collection is
fixed by editing the collection — independent of the CIR's fields.

> **CI is not IPFS's CID.** See the warning in
> [terminology.md](../../../wiki/design/terminology.md): IPFS's CID is a content
> hash; spirit's CI is a *record describing what content is*, addressed by a blob
> hash written `ci:<hash>`.

---

## Transform Definition Record (TDR)

A TDR describes how to produce an output. Its inputs may be:
- **CI reference** — `{ci: "ci:<hash>"}` — the identity is known, the specific output blob still needs resolution
- **CI query** — `{query: {kind, name, version: "^1"}}` — a constraint; the resolver finds the matching CIR
- **Blob** — `{blob: "blob:<hash>"}` — fully pinned; no resolution needed

A TDR where every input is a pinned `blob:` is a **locked TDR** — self-sufficient
and reproducible. A TDR with CI inputs or queries needs one round of resolution
first.

There is no separate "locked" type. A locked TDR is just a TDR with all inputs
pinned; its blake3 hash is the `td:<hash>` committed as a lock file.

### Unlocked TDR (human-authored)

```toml
[td]
kind   = "nix-build"
system = "aarch64-darwin"
flags  = ["--enable-libx264"]

[td.inputs.source]
query = { kind = "package", name = "ffmpeg", version = "^1" }  # constraint

[td.inputs.nixpkgs]
query = { kind = "nixpkgs-channel", name = "nixos-24.05" }
```

### Locked TDR (machine-generated)

```toml
[td]
kind         = "nix-build"
system       = "aarch64-darwin"
flags        = ["--enable-libx264"]
derived_from = "blob:<hash>"   # the unlocked TDR's blob — provenance

[td.inputs.source]
ci   = "ci:<hash>"     # ffmpeg 1.1.0 CIR
blob = "blob:<hash>"   # pinned source archive

[td.inputs.nixpkgs]
ci   = "ci:<hash>"     # nixpkgs-24.05-2026-05-20 CIR
blob = "blob:<hash>"   # pinned nixpkgs snapshot
```

---

## Output Blob

The output of a transform is a **blob** — raw bytes addressed by `blob:<hash>` =
`blake3(bytes)`. Self-verifying: `blake3(bytes) == hash`. No trust required for the
bytes themselves — only the attestation that maps a CIR to this blob requires
trust.

For multi-file outputs (e.g., a package with many files), the blob is an
iroh-blobs Collection — a manifest of named child blobs, itself one hash.

---

## Recursive Resolution

Every step in the build chain uses the same `(ci:<hash>, td:<hash>) → blob:<hash>`
triple. The diagram uses short labels for readability — each label is an opaque
hash in practice:

```
ci:<source>     ──(td:<git-fetch>)──►  blob:<source-archive>
ci:<build-tool> ──(td:<fetch-bin>)──►  blob:<ffmpeg-binary>
ci:<recipe>     ──(td:<resolve>)  ──►  td:<locked>            ← locking IS attestation
ci:<output>     ──(td:<locked>)   ──►  blob:<artifact>        ← building IS attestation
```

The resolution step that converts an unlocked TDR to a locked one is itself
attested: `(ci:<unlocked-tdr>, td:<resolve>) → td:<locked-tdr>`. (The output of a
resolve is a TDR, so it's addressed `td:`; the output of a build is opaque bytes,
so it's `blob:`.)

Running `spirit lock update` re-runs this attestation with fresher index data — the
resolver finds a newer CIR matching the version constraint, pins a new output blob,
and produces a new locked TDR with a new `td:<hash>`. The lock file is just that
new `td:<hash>`.

### Versioned dependencies

Version bounds live in the unlocked TDR as CI queries. The resolver evaluates them
against the local index at lock time:

```toml
[td.inputs.ffmpeg]
query = { kind = "package", name = "ffmpeg", version = "^1" }
```

`^1` means `>=1.0.0, <2.0.0`. The resolver finds the highest matching CIR in the
index — populated via gossip from trusted groups — and pins it. Bumping to `^2`
requires editing the unlocked TDR manually.

### Attestation chain for a versioned package

```
# Resolution attestations (locking) — outputs are locked TDRs (td:)
(ci:<recipe>, td:<resolve>) → td:<locked-v1.0>
(ci:<recipe>, td:<resolve>) → td:<locked-v1.1>   # after lock update

# Build attestations — outputs are opaque bytes (blob:)
# ci:<ffmpeg-1.0> and ci:<ffmpeg-1.1> are different CIRs with different hashes
(ci:<ffmpeg-1.0>, td:<locked-v1.0>) → blob:<ffmpeg-1.0-binary>
(ci:<ffmpeg-1.1>, td:<locked-v1.1>) → blob:<ffmpeg-1.1-binary>
```

Different versions → different CIRs → different `ci:<hash>` values → different
locked TDRs → different output blobs. The same recipe traces through to both via
`derived_from`.

---

## Built-in TD Kinds (Resolver Primitives)

The resolver needs a small set of built-in TD kinds it knows how to execute:

| Kind | What it does |
|---|---|
| `git-fetch` | Fetch a git repo at a rev → source-archive blob |
| `http-fetch` | Fetch a URL → blob |
| `ci-resolve` | Pick the best output blob for a CIR given resolution policy |
| `nix-eval` | Evaluate a Nix expression → locked derivation |

All other TDs compose from these. Custom TD kinds are possible but must be declared
so the runtime knows how to execute them.

---

## Hash Algorithms

- **BLAKE3** throughout — matches iroh-blobs, fast, parallel, Bao-compatible
- CIRs and TDRs: `blake3(canonical(record))` — one canonical encoding (sorted keys, no insignificant whitespace, UTF-8) ensures hash stability across authoring formats and serializers
- Blob hash, single-file: `blake3(raw-bytes)`
- Blob hash, multi-file output: `blake3(bao-collection)`

---

## Open Questions

- **Canonical encoding spec** — **DECIDED 2026-08-28: deterministic CBOR**, per
  RFC 8949 §4.2 Core Deterministic Encoding (definite lengths, shortest-form
  integers, bytewise-lexicographic map key order, no floating point in records).
  Chosen over canonical-JSON for a bytes-native encoding (hashes stay raw, no
  base64 tax), a written determinism spec rather than a convention, and a
  natural fit with iroh's binary framing. Authoring formats stay flexible
  (TOML/JSON in, CBOR hashed).
- **CI schema versioning** — schemas must be stable or the CI hash changes; options: embed `schema_version` in the CIR, or maintain a schema registry per `kind`
- **Input field schema** — standardize `{ci}` / `{query}` / `{blob}` across all TD kinds so resolvers are interoperable
- **Resolver TD kinds** — decide the exact set of built-in kinds for v1; avoid adding kinds that can be composed from existing ones
- **Non-deterministic transforms** — some transforms (video encoding) won't produce identical output blobs across machines; options: require determinism, N-of-M quorum, or flag the TDR as non-deterministic
- **Recursion depth limit** — configurable cap on how many resolution levels the runtime follows automatically
- **External ID dedup** — resolved in principle: a `same_as` [relation attestation](attestations.md#relation-attestation) signed by a trusted group merges two CIRs for the same content. Open part: whether to *auto-propose* `same_as` when external IDs (ISRC, ISBN, …) match, or always require a manual signed claim
- **Nix substituter compatibility** — the Nix binary cache HTTP API (`/nix-cache-info`, `/<hash>.narinfo`, `/<hash>.nar`) could be implemented over spirit as a drop-in `substituters` entry
