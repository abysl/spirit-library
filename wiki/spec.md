# spirit — Specification

> **This document is the source of truth.** Where any other spirit document,
> the README, AGENTS.md, a crate wiki or a plan disagrees with this one, this
> one wins and the other is to be corrected. The older design docs are kept
> for their rationale and prior-art discussion; they no longer define
> behaviour. Revision: 2026-09-08.

Every section carries one of three markers so the spec doubles as the gap
list between design and code:

| Marker | Meaning |
|---|---|
| **[built]** | the code does this today |
| **[partial]** | the code does some of this; the rest of the section is the target |
| **[planned]** | design only; nothing in the tree does this yet |

Section 13 lists the concrete changes that take the code from its current
state to this spec, in the order they should land.

---

## 1. Purpose and scope

Spirit is a content-addressed blob store daemon with a record layer on top,
built on iroh. Version 1 targets exactly two things:

1. **A blob store daemon.** One long-running process per store that owns the
   bytes, serves them to peers over iroh, and offers a local API to the
   applications on the same machine.
2. **Insular device clusters.** A set of devices one person controls, paired
   by scanning a code, that replicate each other's content fully and
   automatically. Every device in the cluster is trusted at the same level;
   nothing outside it is trusted by default.

Above those two sit the primitives that give content a stable identity across
formats and builds: records, attestations and collections. They are in scope
for version 1 because the daemon is useless without a way to name what it
holds.

Out of scope for version 1, and specified only far enough that version 1 does
not preclude them: transform execution, groups spanning more than one person,
query federation across trust boundaries, device roles and replication
classes, sealed checkpoints, follow-and-suggest curation, and non-Ed25519
proof kinds. Section 14 lists them.

### 1.1 Vocabulary bridge

The design was written in one vocabulary and the daemon is discussed in
another. They map one-to-one:

| Everyday term | Spec term | Section |
|---|---|---|
| blob | blob | 4 |
| record | record | 5 |
| identity manifest, blob manifest | Content Identity Record (CIR) | 5.2 |
| artifact, build, copy, encoding | an attested blob: one content attestation `(ci, td) → blob` | 5.4 |
| artifacts map | the artifact view: every trusted content attestation for a CI, keyed by transform | 7.4 |
| playlist, collection, package set | collection | 5.5 |
| device cluster, mesh | a device group; a shared-key DGID | 6 |
| pairing | the pairing handshake | 6.4 |

---

## 2. Principles

These are the rules every later section obeys. A change that breaks one of
them is a change to this section first.

1. **Identity survives form.** An mp3 and a flac of the same song, an arm64
   and an amd64 build of the same program, share one Content Identity. The
   bytes are attested to the identity; they are never part of it.
   Consequently a CIR never lists its artifacts: adding an encoding must not
   mint a new identity.
2. **Trust gates the mapping, never the bytes.** The claim that a CI resolves
   to a blob is accepted only from a signer you trust. The bytes themselves
   are fetched from anyone, because the hash verifies them on receipt.
3. **Trust is local and never transitive.** You set every trust level
   yourself. No remote party asserts one, and trusting A never implies
   trusting whoever A trusts.
4. **Canonicalize, then hash.** Every record is one deterministic CBOR
   encoding, and its address is the BLAKE3 hash of those bytes. Authoring
   formats are free; the hashed bytes are not.
5. **Nothing minted is re-minted.** Signing scopes exclude proof wrappers,
   new fields are optional and omitted when empty, and migrations add records
   rather than rewriting them. A hash minted under this spec stays valid.
6. **Spirit executes nothing.** A Transform Definition is a description. Any
   runtime that fetches, builds or transcodes is supplied by the caller
   through a capability hook. The daemon links no HTTP client and no TLS
   stack.
7. **Spirit carries no consumer schema.** The daemon replicates any record
   whose `refs` it can read. Kinds it does not know are stored, replicated
   and indexed by their generic fields. Game, media and package knowledge
   live in the applications.
8. **One store, one process, one identity.** A store directory is owned by
   exactly one running daemon, which holds the device key and speaks every
   protocol on one iroh endpoint.

---

## 3. Terminology and addresses **[built]**

### 3.1 Hashes and keys

There is one address primitive: the **blob hash**, a 32-byte BLAKE3 digest.
It is the same value iroh-blobs uses as its `Hash`, so spirit and iroh agree
on every address with no translation.

All hashes, public keys and signatures are written as lowercase hex, in prose,
in records, on the wire and in the CLI. There is no base58 form. A 32-byte
value is 64 hex characters; an Ed25519 signature is 128.

### 3.2 Prefixes

A prefix on a hash says what kind of thing it names. The value under every
prefix is a plain blob hash; the prefix is a readability and validation hint.

| Prefix | Names | Code type |
|---|---|---|
| `blob:` | opaque content bytes: a flac, a binary, an image, a rules text | `BlobRef` |
| `ci:` | a Content Identity Record | `CiHash` |
| `td:` | a Transform Definition Record | `TdHash` |
| `att:` | an Attestation Record | `AttHash` |
| `col:` | a Collection head record | `ColHash` |
| `dgid:` | a group public key | `Dgid` |
| `nodeid:` | a device public key | iroh `EndpointId` |

A bare 64-hex string with no prefix is a blob hash whose type is given by
context, for example inside a collection head's `refs` list.

### 3.3 Names

- **Record** — a canonically encoded, immutable, hash-addressed unit of
  spirit data. Never "document": that word belongs to iroh-docs.
- **CIR, TDR, AR, CR** — Content Identity, Transform Definition, Attestation,
  Collection records. Full names in prose, short forms where density demands.
- **DGID** — Device Group ID: an Ed25519 public key identifying a group of
  devices. The stable, user-facing identity.
- **NodeId** — an Ed25519 public key identifying one device; its iroh
  endpoint id.
- **The token `CID` is banned** from spirit code and docs. In IPFS a CID is a
  hash of bytes; in spirit a CI is a record describing content. Using the
  IPFS word inverts the meaning.

---

## 4. Blobs and the store

### 4.1 What a blob is **[built]**

A blob is bytes plus their BLAKE3 hash. Nothing else. Blobs are immutable,
self-verifying, and fetchable from any peer: a blob that does not hash to its
address is discarded on receipt. Every record in sections 5 and 6 is
physically a blob; the record layer is a way of reading some blobs, not a
different kind of storage.

Large outputs with many files are one blob at the spirit level: an iroh-blobs
`HashSeq` collection whose root hash is the blob hash. **[planned]**

### 4.2 One store **[built]**

Every blob's bytes exist once on disk, as the file `<store>/<hex>` that
`spirit-core`'s `BlobStore` reads and writes. iroh-blobs keeps its index and
outboard hashes under `<store>/blobs/` and **references** those files in
place rather than copying them: on start and whenever a blob appears, the
daemon imports each file by reference, which also re-hashes it and removes a
file whose name lies. A blob pulled from a peer is downloaded by iroh and
then **moved** out into the flat store by a reference export, so a pull
writes the bytes once too. Blobs under sixteen kilobytes are the exception:
iroh inlines those in its database, so a record or a small file exists in
both places.

A store from before 2026-09-08 kept a full copy of every blob under
`<store>/iroh/`. The first start after the change deletes that directory and
rebuilds the index by reference, which reads every blob once.

`spirit-core` defines a `Blobs` trait with `put`, `get` and `has`,
implemented by the file store and by `MemBlobs` for tests, so nothing in
core depends on iroh. The record, collection and index code still take the
file store concretely; making them generic over the trait waits for a
second implementation that needs it. The `wasm32` build uses iroh's
in-memory store and holds no persistent blobs.

### 4.3 Store layout on disk

| Path | Holds | Status |
|---|---|---|
| `<store>/<64 hex>` | every blob, one file each, the only copy of its bytes | [built] |
| `<store>/blobs/` | iroh-blobs' index and outboards, referencing the files above | [built] |
| `<store>/identity/key` | the group secret key, hex, mode 0600 | [built] as the single key |
| `<store>/identity/node` | the device secret key, hex, mode 0600 | [built] |
| `<store>/trust` | one `dgid level` line per explicit trust entry | [built] |
| `<store>/refs/<name>` | the hex hash of the current head of a followed collection | [built] |
| `<store>/peers` | the persisted peer registry: every known address, the forgotten set, and per-peer activity, rewritten every round | [built] |
| `<store>/lock` | the running process's pid, touched every round; a second process refuses to start while it is fresh and the pid is alive, and takes over a lock left by a dead pid (`/proc/<pid>` on Linux; elsewhere only the two-minute freshness applies) | [built] |
| `<store>/api.sock` | the local API socket, mode 0600; a store whose path is too long for a socket gets one under the runtime directory instead | [built] |
| `<store>/seeds` | endpoint tickets written by pairing, read on every start; any seed form (ticket, node id, gateway URL) is accepted | [built] |
| `<store>/gateway-token`, `<store>/gateway-port` | the gateway's write token and bound port | [built] |

Ref names are one to three path segments of `[A-Za-z0-9._-]`, no segment
starting with a dot. **[built]**

---

## 5. Records

### 5.1 Encoding and shape **[built]**

A record is a CBOR map encoded under the RFC 8949 §4.2 core deterministic
profile: definite lengths only, shortest-form integers, map keys sorted
bytewise, no floating point, no tags. `spirit_core::canonical` is the only
encoder and decoder; it refuses floats and produces the same bytes for the
same logical map however it was authored.

Hashes, keys and signatures inside records are hex **text**, with their
prefix where section 3.2 gives one. This is a deliberate choice over raw byte
strings: records stay greppable, the gateway can render them as JSON without
a schema, and the size cost on a 32-byte value is irrelevant.

Every record is a map with a `record` field naming its type. Where a type has
sub-schemas, a `kind` field names the sub-schema. Optional fields are omitted
when absent, never encoded as null, so adding an optional field never moves
an existing hash.

| `record` | Address | Defined in |
|---|---|---|
| `cir` | `ci:` | 5.2 |
| `tdr` | `td:` | 5.3 |
| `attestation` | `att:` | 5.4 |
| `collection-op` | bare blob hash | 5.5 |
| `collection` | `col:` | 5.5 |
| `receipt` | bare blob hash | 14, deferred |

A record's address is `blake3(canonical(record))`, the whole map including
any proof block. Signing scopes are narrower and are given per type.

### 5.2 Content Identity Record **[built]**

A CIR says what a piece of content *is*. It is immutable, and its address is
its identity.

```
record = "cir"
kind   = "<schema name>"
body   = { ... }
```

`body` is a CBOR map whose fields are fixed by `kind`. Every field in the
body is part of the identity, so the body carries **identifying fields
only**. A song's artist, album and title identify it; its cover art, its
lyrics text and its current tags do not, and they are attached as attested
content (5.4) rather than written into the CIR.

Conventions across all kinds:

| Body field | Type | Meaning |
|---|---|---|
| `name` | text | the human name; recommended for every kind, required by the generic kind |
| `external` | map of text to text | ids in other namespaces (ISRC, Scryfall oracle id, CPE); lets independent minters converge and lets the index answer "which CI has external id X" |
| `owner` | `dgid:` | present only on owned CIs; only the owner's attestations about this CI are authoritative |
| any `ci:` value | text | a structural link to another CI, indexed as a back-link |

Version is part of identity for software: `ffmpeg 7.1.0` and `ffmpeg 7.2.0`
are two CIRs. Version is not part of identity for a song or a card, whose
text and art revise underneath a stable identity.

**Corrections.** A CIR is never edited. A typo is fixed by minting a new CIR,
re-attesting the same blobs to it, and publishing a `superseded-by` relation
(5.4) from old to new. The old CI keeps resolving; nodes that trust the
corrector prefer the new one.

**Schema versioning.** A kind's field set is frozen once a CI has been
minted under it, because changing it would move every hash. A breaking
change is a new kind name, `card/2` beside `card`, and CIs minted under both
are bridged with `same-as` relations (5.4). Adding an optional field is not a
breaking change: absent fields are omitted from the encoding, so existing
hashes stand.

**Kinds owned by spirit** live in `spirit-schema`; everything else lives in
the application that understands it (section 12).

### 5.3 Transform Definition Record **[partial]**

> `variant` and `snapshot` are read by the resolver today; `inputs`,
> `derived_from` and locking arrive with transforms (section 10).

A TDR says how an artifact was, or can be, produced from inputs. It is the
key that distinguishes one artifact of a CI from another: the flac and the
mp3 of one song are `(ci, td:flac-encode) → blob` and
`(ci, td:mp3-encode) → blob`.

```
record = "tdr"
kind   = "<transform kind>"
body   = { ... }
```

Standard optional body fields, honoured by the resolver and the UI whatever
the kind:

| Field | Type | Meaning |
|---|---|---|
| `variant` | text | a short human label for the artifact this transform yields: `flac`, `1080p`, `aarch64-linux`. This is the key the artifact view groups by |
| `snapshot` | RFC 3339 text | when the inputs were taken from the outside world; recency ordering in resolution |
| `inputs` | map of name to input | what the transform consumed |
| `derived_from` | `td:` | the unlocked TDR this locked one was resolved from |

An **input** is one of `{ci}`, `{ci, blob}`, `{blob}`, `{url}` or
`{query}`. A TDR whose every input is pinned to a `blob:` is **locked**: it
can be re-run offline and byte-identically. There is no separate locked type;
locking replaces each input with its pinned form and mints a new `td:`.

Today a TDR is an opaque `{record, kind, body}` written by importers to
record provenance. The standard fields, inputs and locking are the target;
the record shape does not change. Transform *execution* is section 10.

### 5.4 Attestation Record **[built]**

An attestation is a signed claim. The claim is the signing scope; the proof
sits outside it, so an unsigned claim gains a signature without re-minting.

```
record = "attestation"
claim  = { kind, ci, td?, blob?, other?, expires? }
proof  = { dgid, sig }
```

The signing scope is `blake3(canonical(claim))`, signed with Ed25519 by the
group key named in `proof.dgid`. `signer()` returns the DGID only if the
signature verifies; an attestation with no proof or a bad proof is stored,
replicated and never resolved.

| `claim.kind` | Fields | Meaning |
|---|---|---|
| `content` | `ci`, `td`, `blob` | this CI, produced by this transform, is these bytes |
| `same-as` | `ci`, `other` | the two CIs denote the same content; resolvers union their artifacts |
| `superseded-by` | `ci`, `other` | `other` corrects `ci`; prefer `other` for display, keep `ci` resolvable |
| `previous-version` | `ci`, `other` | `other` is the version before `ci` in a lineage |

`expires` is optional RFC 3339 text. An expired attestation is excluded from
resolution and shown as expired in the artifact view; no revocation message
is needed. The index still holds it, so a correction that re-attests the
same blob keeps its history.

Only the `group-signed` proof exists. Multisig, reproducible and zero-knowledge
proofs are deferred (section 14) and would be added as new `proof.kind`
values without touching the claim.

### 5.5 Collections **[built]**

A collection is a named, ordered list of CI references owned by one DGID.
Items reference identities, never blobs, so a shared playlist keeps working
when a better encoding appears. The same type is a playlist, a package set,
a module's version history and a device group's membership.

A collection is identified by `(owner, name)`. Its state is an
**append-only set of signed ops** folded deterministically; a **head** record
is a published snapshot of that set, addressed `col:`.

**Op.** Each edit is its own record, signed by the owner key:

```
record = "collection-op"
claim  = { kind, collection, owner, seq, item? | target? | order? }
proof  = { dgid, sig }
```

| `claim.kind` | Carries | Effect in the fold |
|---|---|---|
| `add` | `item = { ci, label?, default_td? }` | insert, or replace the item with the same `ci` |
| `remove` | `target = ci` | drop the item |
| `reorder` | `order = [ci, ...]` | move the listed items to the front in that order; unlisted items follow in their previous order |

An op's `seq` is one more than the highest `seq` its author had seen for that
collection. Ops are ordered by `(seq, op hash)`; ops whose signer is not the
collection's owner are ignored by the fold. Two devices of one shared-key
group can append concurrently: they mint two ops with the same `seq`, both
are kept, and the hash breaks the tie identically everywhere. This is a
grow-only set with a total order, so heads merge by **union of ops** and
every node folds the same items with no clocks and no coordination.

A pull of a head whose `(owner, name)` matches a local head merges: if one
side's closure contains the other's, the larger head is adopted as is;
otherwise a union head is minted locally. Two nodes that each merge the
other's head converge, because after one round each holds a superset of the
other and a superset is adopted rather than re-minted.

**Head.** A published snapshot:

```
record       = "collection"
kind         = "<collection kind>"
name         = "<name>"
owner        = "dgid:..."
forked_from  = "dgid:..."   optional
ops          = [hash, ...]
attestations = [hash, ...]
records      = [hash, ...]
refs         = [hash, ...]
```

`ops` is every op the publisher knows; `attestations` and `records` are the
attestation and CIR/TDR/blob hashes the collection wants replicated alongside
its items; `refs` is the sorted union of all three, the closure the mesh
counts and pulls (section 8). A head minted before 2026-09-07 has no
`record` field and `kind = "collection"`; it still decodes, and the next
publish over it mints a modern head.

**Fork.** Publishing a head over a collection whose current head has a
different owner carries every item forward under the publisher's key and
records `forked_from`. A fork is a clean break: it does not track its source.

**Kinds spirit defines**: `device-group` (6.3), `modules` (12). A consumer
may use any other kind string; the fold does not care.

---

## 6. Identity, groups and trust

### 6.1 Two keys per device **[built]**

A device holds two Ed25519 keys.

- The **device key** is its iroh endpoint identity. It authenticates
  connections and signs the device's own records, such as the settings in
  its device CIR. It is never shared.
- The **group key** is the DGID of the device group the device belongs to.
  Under the `shared-key` scheme every device in the group holds the same
  group secret and can sign as the group: attestations, collection ops,
  membership ops.

The group key lives at `identity/key` and the device key at
`identity/node`; a store from before 2026-09-07 had only the first, used as
both, and keeps it as the group key so every attestation and op already
signed stays valid. The device key is generated on first start after the
split, which changes the node id once; peer registries and seed files that
named the old id re-learn or are re-seeded.

Every gossip view carries the sender's DGID and a **vouch**: the group key's
signature over the sender's node id. A receiver that verifies the vouch
records the device's group, and until device groups (6.3) exist that vouch
is the membership proof: a device and the group it proves membership of
share the higher of their two trust levels (6.2).

A device with no group yet is a group of one: its first start generates both
keys and publishes a `device-group` collection containing itself. Pairing
(6.4) is how a second device joins that group instead of forming its own.

**One store is one group.** Every device in the group holds the group secret
and attests on behalf of the whole group; there are no per-device
permissions inside a group. A device that wants to be in a second group runs
a second store, with its own device key and its own DGID, and the two
networks never touch: different DGIDs, different peers, different content.
Threshold signing and per-group permissions for one device are deferred
(section 14).

### 6.2 Trust levels **[built]**

| Level | Who | Grants |
|---|---|---|
| `mesh` | devices in your own group | everything: peers exchanged, every ref followed, attestations accepted, membership ops signed |
| `cache` | a group whose attestations you accept | their attestations resolve; their advertised refs are followed |
| `contact` | a group you exchange collections with | collections you explicitly follow are pulled; nothing influences your index otherwise |
| `unknown` | everyone else | bytes only; a hash is a hash |

The registry is `<store>/trust`, one `dgid level` line per entry, edited with
`spirit trust <dgid> <level>`. Your own group is always `mesh` and is not
written to the file.

**Trust of a node id** **[built]** is derived. A device in the folded
membership of your own group (6.3) is `mesh`, whatever the file says. For
every other device the gossip vouch (6.1) proves its group, and the
derivation runs both ways: seeding a device at `cache` makes its vouched
group `cache`, and trusting a group makes every device that proves
membership trusted at the group's level. An explicit entry for a bare node
id remains honoured as an override.

Seeding a peer from the command line grants it `cache`. Pairing grants
`mesh`. Nothing else grants anything.

### 6.3 Device groups **[built]**

A group's membership is a collection of kind `device-group`, named
`device-group` and owned by the group's DGID. Its items are **device CIRs**:

```
record = "cir"
kind   = "device"
body   = { pubkey, settings = { dgid, tags?, role?, expires? }, device_sig }
```

`device_sig` is the device key's signature over
`blake3(canonical(settings))`. It is the device's consent: a group cannot
admit a device it does not control, because it cannot produce that
signature. Changing a device's settings publishes a new device CIR.

Membership ops are ordinary collection ops signed by the group key: `add`
admits a device CIR, `remove` revokes the device named by its `pubkey`,
`add` again reinstates it. The `amend` and `reinstate` verbs of the earlier
design collapse into `add` with a newer device CIR. Admitting a device whose
`pubkey` is already present removes the older record in the same publish, so
the fold holds one record per device.

A device that starts with no membership admits itself: `node_secret`
enrols the device in the store's own group on every start, which is a no-op
once it is listed. `spirit-node members` prints the fold and `revoke <node>`
drops a device; the `device-group` ref is replicated like any other, so a
revocation reaches every member by gossip.

The folded membership is the source of truth for section 6.2's derived trust
and for who may hold the group secret. The daemon re-folds it every round
and after every admission.

Signing schemes other than `shared-key` are deferred (section 14). The scheme
is a property of the group and is invisible to verifiers: a proof is always
one Ed25519 signature against the DGID.

### 6.4 Pairing **[built]**

Pairing adds a device to a group. It is the only way to reach `mesh` trust,
and it is symmetric by construction: the joiner trusts the group and every
member trusts the joiner, in one round.

The admitter is any device already in the group. The joiner is a device with
a device key and no group, or one that chooses to leave its group of one.

1. **Offer.** The admitter runs `spirit pair`. It mints a random 32-byte
   token, valid for ten minutes and for one use, and displays
   `spirit://pair?ticket=<endpoint ticket>&dgid=<dgid>&token=<hex>` as a QR
   code and as text.
2. **Consent.** The joiner scans it, builds a device CIR for `dgid` with the
   settings it offers, signs the settings with its device key, and dials the
   ticket on ALPN `spirit-pair/0`.
3. **Prove.** Over the iroh connection, which is already mutually
   authenticated by device keys, the joiner sends `{token, device_cir}`. The
   admitter checks the token in constant time, consumes it, and checks that
   the CIR's `pubkey` is the connection's remote id and that `device_sig`
   verifies.
4. **Admit.** The admitter appends an `add` op for the device CIR to the
   `device-group` collection, publishes the new head, and replies
   `{group_secret, head, members: [endpoint addr, ...]}`.
5. **Join.** The joiner writes the group secret to `identity/key`, sets the
   group to `mesh`, seeds every listed member, and pulls the `device-group`
   collection. Its existing group-of-one collection is abandoned.
6. **Converge.** Every other member learns the new op by gossip, folds the
   membership, and by section 6.2 trusts the joiner at `mesh`.

A failed token, a mismatched `pubkey` or a bad signature closes the
connection with no state change. The token never travels over anything but
the QR code and the authenticated stream.

The admitter is always the running daemon: it holds the offer and answers
on `spirit-pair/0`. `spirit-node pair` and the UI's pairing panel ask it for
a code through the gateway. The joiner is either the running daemon, which
takes the link through the gateway and reloads its identity in place, or
`spirit-node join` with no daemon running, which binds the device key
itself, joins, writes the members to `<store>/seeds`, and leaves the
daemon to replicate when it next starts. The daemon reads `<store>/seeds`
on every start beside any `--seed-file`.

Seeding an endpoint ticket by hand still works and still grants only
`cache`; it is how a device follows a group it does not belong to. A seed
may also be a gateway URL (`http(s)://host[:port]`): the node fetches
`<url>/gateway/status` when the seed is applied and seeds the `ticket` it
finds there, so a client that names the dev gateways by URL survives their
device keys changing. `http://` is fetched by a built-in blocking GET over
plain TCP; `https://` needs the caller's `Mesh::set_fetch` hook, since the
daemon links no TLS.

---

## 7. Naming, index and resolution

### 7.1 Refs **[built]**

A ref is a local name for the current head of a collection this store
follows: `<store>/refs/<name>` holds the head's hex hash. Refs are the unit
of replication and the entry points the index rebuilds from. Consumers write
them only through `spirit_core::refs`.

A ref name is local. The collection it follows is identified by the
`(owner, name)` in its head, and a peer's advertised ref is matched on both:
once a local ref has an owner, only adverts with that owner, or legacy
adverts with none, are followed for it.

### 7.2 Index **[built]**

The index is derived state: a fold over every ref's head and its `records`,
rebuilt from the store at start-up and on ref change, never authoritative.
It answers:

| Query | From |
|---|---|
| `kind_of(ci)` | the CIR |
| `attestations_for(ci)` | every attestation record naming `ci` |
| `linked_to(ci)` | every CIR whose body contains `ci:` |
| `by_external(key, value)` | every CIR's `external` map |
| `owner_of(collection name)` | the head |

It is in memory. Persistence, size limits and eviction arrive only when a
real library forces them; the fold stays the definition.

### 7.3 Resolution **[built]**

`resolve(ci, policy)` picks one artifact for a CI. Candidates are the
content attestations for `ci` whose signer verifies, is trusted at or above
`policy.minimum` (default `cache`), and which have not expired. They are
ranked, best first, by:

1. the transform in `policy.prefer_td`, which a caller sets from a
   collection item's `default_td`;
2. held locally, when `policy.prefer_held`;
3. the signer's trust level, `mesh` over `cache`;
4. the TDR's `snapshot`, newest first;
5. the blob hash, as a deterministic tie-break.

The winner's blob is fetched from any provider (section 8) and verified.

For an owned CI (5.2) only the owner's attestations are candidates.

### 7.4 The artifact view **[built]**

The artifact view is the materialized "manifest" for a CI: the identity plus
everything trusted that resolves to bytes, grouped by variant.

```
ArtifactView {
  ci, cir,
  artifacts: [ { variant, td, tdr, blob, signer, level, held, snapshot, expires } ],
  relations: [ { kind, other, signer } ],
}
```

`routing::artifacts(store, index, trust, ci, policy)` builds it from the
same candidate set as 7.3, ranked the same way, with every candidate marked
eligible, untrusted or expired rather than dropped. It is what a UI shows when it lists "available as flac,
mp3, remaster" or "built for arm64, amd64", and it is the answer to every
question that would otherwise tempt someone to write the artifacts into the
CIR.

---

## 8. Replication and gossip

### 8.1 Protocols **[built]**

The daemon registers exactly two ALPNs itself: iroh-blobs for bytes, and
`spirit-gossip/0` for membership and adverts. Pairing adds `spirit-pair/0`
(6.4). Every other protocol comes from the caller through the register hook
(section 9.3). One endpoint, one device key, all protocols.

### 8.2 Gossip **[built]**

One CBOR round trip on a bidirectional stream; both sides send a `View` and
merge the other's:

```
View { addr, peers: [addr, ...], refs: [RefAdvert, ...], table?, heard_tables: [HeardTable, ...] }
RefAdvert { name, manifest, total, held, owner? }
HeardTable { host, table, heard_at }
```

`manifest` is the head hash, named for the manifests it carried before
collections existed. `owner` is the head's DGID and is optional on the wire
so nodes from before it interoperate. `table` is a caller-supplied presence
advert that only its host ever sends. `heard_tables` relays every open table
the sender knows of, first-hand or relayed, keyed by host node id and table
name, with `heard_at` the second at which the host itself was last heard —
stamped first-hand and carried unchanged through relays. A receiver drops
relayed entries naming itself or the sender, hosts it does not know, stamps
older than five minutes, and hosts whose first-hand advert is live or was
withdrawn after the stamp; the rules are in `wiki/design/gossip.md`. The
field defaults to empty so older nodes interoperate.

Convergence rules, all built:

- peers are keyed by node id, so hearing about one twice is a no-op;
- a **view version** bumps only when a merge taught the node something, and
  a peer is re-contacted only when the version moved since the last exchange
  or an idle interval passed, so a settled mesh drops to a heartbeat;
- at most four peers per round;
- repeated dial failures park a peer; an hour of silence prunes it, and a
  pruned peer is ignored in hearsay until it speaks for itself again.

A view is accepted from any peer: introductions cost nothing. What is
*followed* from a peer is gated by trust, below.

### 8.3 What a node replicates **[built]**

The wanted set is: every ref the node already holds, every ref passed with
`--want`, and every ref advertised by a peer trusted at `cache` or above.
For each wanted ref the node picks the best provider among trusted peers
advertising it, pulls the head, and pulls every hash in the head's `refs`
that it lacks. Every byte is verified on write.

Providers are matched on `(owner, name)` and ranked: the owner's own
devices first, then the highest trust level, then the most held. A peer is
a provider for a ref when it holds more of it than we do, or when it
advertises a complete head of our own owner's collection that differs from
ours and has not been merged yet. A pulled head merges with the local one
(5.5) rather than replacing it, and a head is merged at most once per
process, so two devices with differing heads converge in one round each and
then go quiet.

A caller may also request single blobs by hash; the converge loop pulls them
from a hinted provider, then from any peer advertising a complete ref that
should contain them, then from any known peer.

Inside a device group this means full replication: every device follows
every other's refs at `mesh`, so the cluster converges on one content set
with no configuration.

### 8.4 Backfill hook **[built]**

If a wanted ref is missing from every reachable peer, the node with the
lowest id among reachable peers may call a caller-supplied backfill closure
for it, after a settle window and with a per-ref cooldown. The daemon ships
no backfill mechanism; this is how an importer in an application gets
invoked exactly once per cluster rather than once per device.

---

## 9. The daemon

### 9.1 Process model **[built]**

`spirit-node mesh <store>` is the daemon: it opens the store, takes
`<store>/lock`, binds the endpoint with the device key, registers the
protocols, serves the local API socket, runs the gossip and converge loops,
prunes and persists peers, and optionally serves the gateway. A second
process on the same store fails fast instead of binding a second endpoint
under the same key. The lock is a pid file the daemon touches every round;
a lock older than two minutes is stale and yields, so a killed daemon
blocks a successor for at most that long. SIGINT and SIGTERM both shut the
daemon down cleanly, removing the socket and the lock. A daemon whose
endpoint cannot come online within ten seconds starts anyway and serves the
store locally until the network returns.

The same code runs **embedded**: an application may call `serve_mesh_with`
in-process instead of talking to a daemon, and kai's desktop build does. The
rule is one process per store, not one binary per store.

### 9.2 Local API **[built]**

Applications on the same machine talk to the daemon over a Unix domain
socket at `<store>/api.sock`. Each frame is a four-byte big-endian length
followed by CBOR: a request `{method, path, query, body}` naming one of the
gateway's routes (9.4), and a reply `{status, content_type, body}`. The
socket is mode 0600 and needs no token; filesystem permission is the
authorisation. Every operation the CLI and the UI run goes through the same
dispatcher whichever transport carried it, and `spirit-node pair` and
`join` use the socket first, the gateway second, and bind an endpoint of
their own only when no daemon answers. This is the boundary that lets a
music player, a package manager and a game share one store without each
linking iroh. The `wasm32` browser client uses the gateway instead.

### 9.3 Register hook **[built]**

`serve_with`, `serve_mesh_with` and `serve_in_memory_with` hand the caller the
`RouterBuilder` before it spawns, so the caller adds its own ALPNs to the
daemon's endpoint. Game protocols, table sessions and anything else with
consumer knowledge arrive this way and never live in spirit.

### 9.4 HTTP gateway **[built]**

`--gateway <port>` serves an HTTP/1.1 API on `127.0.0.1` for clients that
cannot speak iroh, and a browser UI at `/` for the daemon's operator. It is
hand-rolled over tokio TCP by design; introducing hyper, axum or a TLS stack
is a regression. Exposure beyond localhost is a reverse proxy's job, and the
proxy decides who can reach it: spirit.rae.blue is tailnet-only.

Reads are open to whoever can reach the port: status and stats, the ref
list, the blob list, any blob by hash (with `?name=` for a download
filename), any record decoded to JSON with its signature verified, the
index, collections and their ops, the artifact view for a CI, the trust
table, and caller-registered named resolvers.

Writes need a bearer token. The daemon reads `<store>/gateway-token` and
mints one if the file is missing; the operator copies it out of the store
directory and pastes it into the UI once. With the token a client can store
a blob, mint a CIR or TDR, sign a content or relation attestation, edit a
collection (`add`, `remove`, `attest`, `record`), set trust, offer a
pairing code, join a group by its link, and revoke a member. The daemon
records its port in `<store>/gateway-port` so the CLI can find it. Every write
goes through the same operations the CLI uses, so anything the UI does the
CLI can reproduce, and both are the seed of the local API of 9.2.

### 9.5 CLI

Every command takes `--store <dir>`, or `SPIRIT_STORE`, defaulting to
`~/.spirit/store`. `spirit-node help` prints the full list.

| Command | Does | Status |
|---|---|---|
| `identity` | the store's DGID, node id and key path | [built] |
| `refs` | every ref name and the record it points at | [built] |
| `blob put | get | has | list` | raw blob operations; `list` labels each blob by record type | [built] |
| `record show <hash>` | decode any record to JSON and verify its signature | [built] |
| `cir mint <kind> <json>` / `tdr mint <kind> <json>` | mint records from a JSON body | [built] |
| `attest content <ci> <td> <blob>` / `attest relation <kind> <ci> <other>` | sign claims with the store key | [built] |
| `collection list | show | ops | add | remove | attest | record` | build and inspect collections; each write publishes a head and moves the ref | [built] |
| `index` | the local fold: collections, kinds, links, externals, attestations with trust | [built] |
| `resolve <ci> [--min <level>] [--td <td>]` | every candidate artifact with its variant, trust and expiry, and the pick | [built] |
| `trust [<id> <level>]` | list or set trust | [built] |
| `mesh`, `serve`, `fetch` | the daemon and one-shot sync; `--gateway` adds the HTTP API and browser UI of 9.4 | [built] |
| `pair` / `join <link>` | the pairing handshake (6.4): offer a code through the running daemon; join through it or standalone | [built] |
| `members` / `revoke <node>` | the folded device-group membership (6.3) | [built] |
| `peers` | the peer registry without a running daemon | [planned]: kai renders it today |
| `lock <td\|@file>` / `run <td> --ci <ci>` | transforms, section 10; the CLI has no fetcher or runner, so it pins what the index can answer and names the missing runtime otherwise | [built] |

Rebuild the binary after changing the CLI; `clippy` and `fmt` do not.

---

## 10. Transforms **[built]**

Spirit owns the record, the lock and the attestation. It runs nothing.

Two capability traits, supplied by the caller: a `Fetcher` that turns a
`{url}` input into bytes, and a `TransformRunner` that runs a
`wasm-transform` module against pinned inputs. An application registers the
ones it can provide; the daemon binary and the CLI register none, so
`spirit-node lock` pins identities, queries and blobs and refuses a url,
and `spirit-node run` reports which runtime is missing.

A TDR's `inputs` map names each input as `{ci}`, `{query: {kind, name}}`,
`{url}` or, once pinned, `{blob}` with the `ci` or `url` it came from kept
beside it. `lock(tdr)` resolves every `{query}` to a CI against the index by
kind and `name`, every `{ci}` to a blob through 7.3, and every `{url}` to a
blob through the fetcher, producing a locked TDR with `derived_from` set to
the unlocked record and a `snapshot` stamped if the author left it out.
Locking a locked TDR pins nothing and fetches nothing.

`run(td, ci)` executes a locked TDR and attests its output to `ci` as
`(ci, td) → blob`, signed by the store's group key. An `http-get` is the one
impure step: its output is what the fetcher returns for its `url`. A
`wasm-transform` names its `module` and optional `args` blobs and is handed
its pinned inputs' bytes; a runner may answer `NeedInputs([url, ...])`
instead of output, in which case the host fetches, pins those urls into a
new locked TDR, and re-invokes, for at most sixteen rounds. The attestation
names the final TDR, so the output's provenance lists every input the
transform actually consumed.

Descriptive kinds such as `flac-encode` with no registered runtime are
valid TDRs that record provenance and cannot be run; that is what importers
write today.

---

## 11. SDK surface

`spirit-sdk` is what applications depend on. It re-exports the protocol
crates and adds nothing today. The intended v1 surface, in the order the two
lifecycles use it:

**Read path**: `Index::attestations_for(ci)` → `routing::resolve(...)` →
fetch the blob → verify. **[built]**, minus the ranking refinements in 7.3.

**Write path**: `Cir::new(kind, body)` → `Tdr::new(kind, body)` → put bytes →
`Attestation::sign(Claim::content(ci, td, blob), identity)` →
`collection::Builder` add, attest, record, publish → the mesh replicates.
**[built]**, with publish becoming append (5.5).

**Trust and identity**: `identity::load_or_create`, `Trust::load`, `set`,
`level`, `trusts`. **[built]**

**Artifact view**: `routing::artifacts(...)`, behind `spirit-node resolve`
and the gateway's artifacts route. **[built]**

**Deep links** **[planned]**, type first, then authority:

```
spirit://ci/<hash>
spirit://blob/<hash>
spirit://collection/<dgid>/<name>
spirit://group/<dgid>
spirit://pair?ticket=…&dgid=…&token=…
```

Cross-app identity, app registration and attribution fields are deferred
(section 14).

---

## 12. Schema: what spirit defines

`spirit-schema` holds the kinds spirit itself needs and the body conventions
of 5.2. Nothing game-specific.

| Kind | Record | Status |
|---|---|---|
| `device` CIR and the `device-group` collection | 6.3 | [built] |
| `wasm-module` CIR: `{abi_version, name, role, version}` | a versioned software artifact; one CIR per version, each attested to its wasm blob | [built] |
| `modules` collection, one per module name: `modules/<name>` | the version history; `resolve(name, abi)` picks the newest held, trusted version | [built] |
| `item` CIR: `{name, external?, owner?, ...}` | the generic kind for anything with a name and identifying metadata and no better schema; extra identifying fields ride beside the named ones | [built] |

The `card` and `card-printing` kinds and the catalog collection were game
knowledge and moved to agni on 2026-09-08 (`agni-importers::cards`). They
use nothing spirit-specific beyond the conventions above.

---

## 13. From the code to this spec

The concrete deltas, in landing order. Each is small; together they are
the gap the markers above describe.

| # | Change | Sections |
|---|---|---|
| 1 | ✅ 2026-09-07 — Collection head: discriminator `kind: "collection"` becomes `record: "collection"`; `kind` names the collection kind. Envelope reads `refs` and treats `record`/`kind` as informational. Heads are republished; no CIR or attestation hash moves | 5.1, 5.5 |
| 2 | ✅ 2026-09-07 — Ops append: `Builder::publish` mints ops only for edits since the loaded head and lists old plus new; heads with the same `(owner, name)` merge by union of ops | 5.5 |
| 3 | ✅ 2026-09-07 — `RefAdvert` gains optional `owner`; follow and provider choice match on `(owner, name)` | 7.1, 8.2, 8.3 |
| 4 | ✅ 2026-09-07 — Split keys: `identity/key` stays the group key, `identity/node` is generated; `node_secret` reads the device key; the gossip view vouches for the node id | 6.1 |
| 5 | ✅ 2026-09-07 — `device` CIR, the `device-group` collection, and trust derived from folded membership | 6.2, 6.3 |
| 6 | ✅ 2026-09-07 — `spirit-pair/0`, `spirit pair`, `spirit join` | 6.4 |
| 7 | ✅ 2026-09-08 — `Blobs` trait in core; the node references the flat files from iroh and exports pulls by moving them, so bytes exist once; the old copied `iroh/` index is rebuilt on first start | 4.2 |
| 8 | ⏸ blocked — Delete the legacy readers: `core/modules.rs`, the envelope's 64-hex scan, `schema::modules::legacy_versions`. The deployed `hob` and `riftbound` manifests on dev1 and dev2 declare no `refs`, and their modules are legacy refs, so deleting these today stops that content replicating and kai.rae.blue's modules resolving. Unblocked once agni's importers write `refs` into every manifest, the fleet re-ingests, and modules are republished as collections | 5.1 |
| 9 | ✅ 2026-09-08 — `card`, `card-printing`, catalog moved to `agni-importers::cards`; `item` kind added | 12 |
| 10 | ✅ 2026-09-08 — TDR `variant` and `snapshot` read by the resolver; ranking by preferred transform, snapshot and expiry | 5.3, 7.3 |
| 11 | ✅ 2026-09-08 — `routing::artifacts` behind `spirit-node resolve --td` and the gateway's artifacts route | 7.4, 9.5 |
| 12 | ✅ 2026-09-08 — Store lock, persisted peer registry, local API socket | 4.3, 9.1, 9.2 |
| 13 | ✅ 2026-09-08 — Transforms: `Fetcher` and `TransformRunner` traits, `lock`, `run` with the fetch-plan loop, `spirit-node lock` and `run` | 10 |
| 14 | ✅ 2026-09-12 — Assets over the mesh: every node publishes an `assets` ref (`node/src/assets.rs`, a `{kind: "assets", refs, entries}` map from asset key to blob) that gossips as an ordinary ref advert; `Mesh::find_asset` reads the peer indexes a node holds, `missing_asset_indexes` names the ones to pull, and `mesh::fetch_blob` pulls a single blob from a named provider on demand, so a consumer (kai's art worker, the deck gateway's `asset` resolver) fetches from a URL only when no known node holds the blob. `SPIRIT_QUIC_PORT` pins the native endpoint's port for router forwarding | 7.1, 8.2 |

Items 1 through 3 change no wire ALPN and interoperate with current nodes
through optional fields. Item 4 changes every node id once. Item 7 changes
the on-disk layout and is the only step that needs a migration on first
start; it deviates from the first draft of 4.2, which retired the flat files
in favour of iroh's own data directory, because keeping the flat files as
the one copy left every consumer's synchronous store API untouched.

---

## 14. Deferred

Designed in the older documents, consistent with this spec, and not part of
version 1. Each is listed so version 1 does not paint over it.

- **Witness receipts and sealed checkpoints.** The `(seq, hash)` order is
  sufficient for a shared-key group. Receipts would replace the ordering key
  if a group ever needs ordering by trusted observation time; checkpoints
  would compact long op histories.
- **Follow and suggest.** A follower sending a signed proposed op for the
  owner to approve. Today following is implicit in the wanted set.
- **Groups beyond shared-key**: FROST and multisig threshold signing; one
  device in several groups with different permissions in each; owned versus
  leased members; access grants; `dgid:web:<domain>` resolution.
- **Proof kinds**: `group-multisig`, `reproducible`, `zk`.
- **Query federation**: device roles, `Hit`/`Miss`/`Partial` responses,
  routing rules, hit-rate ordering, replication classes and cooperative
  placement.
- **Index persistence and eviction**, feed sync from `cache` groups on a
  schedule, feed pagination.
- **SDK conventions**: cross-app identity, app registration, attribution and
  license fields, deep-link routing to apps.
- **Blue sky**: scalable streaming over Bao ranges, delta encoding, content
  defined chunking, transitive trust webs. See `future-ideas.md`.

---

## 15. Open decisions

Questions this spec leaves to its owner. Each has a default so work can
proceed.

1. **Where do a group's name and relay hints live?** Default: a small
   `group` CIR owned by the DGID, referenced from the `device-group` head's
   `records`, never in the head itself.
2. **Contact-level following.** Whether a `contact` group's advertised refs
   are listed for the user to pick from, or invisible until named. Default:
   listed, never auto-followed.

Decided 2026-09-07 and folded into the body: one store is one group and
every device signs for the group (6.1); a breaking schema change is a new
kind name bridged by `same-as` (5.2). Decided 2026-09-08: the local API is
length-framed CBOR over a Unix socket carrying the gateway's request shapes
(9.2); irpc is revisited only if a second in-process embedding needs a
shared definition.
