# spirit — Groups and Trust

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): the degenerate group is implemented.** A store's
> persisted Ed25519 key is its DGID (`spirit_core::identity`), which is also
> the iroh node key, and trust levels are local and never transitive
> (`spirit_core::trust`). Device-membership collections, `dgid:web:` resolution
> and witness receipts are still design.

## DGID — The Stable Identity

A **DGID** (Device Group ID) is an Ed25519 public key identifying a logical entity — a person's device mesh, a company's server cluster, a community cache, a paid CDN service. The label is agnostic to what the group represents; at its core it is a public key whose membership is a **collection of device-CIs** describing which nodes belong and what they are trusted to do.

```
DGID = ed25519-pubkey  →  a device-group collection (append-only, DGID-signed, of device-CIs)
```

A DGID can be written two ways:

| Form | Example | How it resolves |
|---|---|---|
| `dgid:<base58-pubkey>` | `dgid:abc123...` | The public key directly |
| `dgid:web:<domain>` | `dgid:web:jasmine-dragon.com` | HTTPS: `https://<domain>/.well-known/spirit.json` returns `{ "dgid": "<base58-pubkey>" }` |

The DNS form gives groups a human-readable address without a central registry. Trust is still anchored to the Ed25519 public key — the well-known document is a pointer, not the trust root.

---

## Trust Levels

Every DGID in your registry has a trust level you set locally. Trust is never asserted by the remote group and never transitive.

| Level | Who | What it enables |
|---|---|---|
| **Mesh** | Your own devices | Full bidirectional index gossip; content attestations signed and accepted; access grants honoured |
| **Cache** | Trusted upstream caches, build farms | Content attestations accepted; CI feeds synced into your index |
| **Contact** | Friends, publishers, services | Shared collections visible; access grants from this DGID honoured for the services specified |

A Contact can be manually promoted to Cache. The promotion should be explicit and reversible in any UI.

---

## Device Membership

A device group's membership is itself a **collection** (see [collections.md](../../../schema/wiki/design/collections.md)) — an append-only, DGID-signed log whose items are **device-CIs**. Admitting, amending, revoking, or reinstating a device is an op appended to this log; the current membership is the log folded in [timestamp order](#reconciling-concurrent-appends). There is no separate "membership" object — only device-CIs and the ops that admit them. Same Blob + Statement kernel, same append-only chain, same follow/fork machinery as any other collection.

### The device-CI

A device-CI is the immutable, content-addressed document a device publishes about itself: its public key plus a **settings object signed by that device's own key** — the capabilities it offers a specific group, with a TTL.

```toml
[ci]
kind   = "device"
pubkey = "nodeid:xyz..."

[ci.settings]                            # signed by the device key
dgid        = "dgid:abc123..."           # the group these capabilities are offered to
tags        = ["image-cache", "cat-pics"]# capabilities this node provides within the group
role        = "indexer"
replication = "archive"
expires     = "2026-09-01T00:00:00Z"     # TTL on this signed settings block
device_sig  = "base64:..."               # nodeid key signs blake3(canonical(settings without device_sig))

# ci:<hash> = blake3(canonical(whole document)) — immutable
```

**The device's signature is its consent.** A group cannot fabricate a device-CI for a node it does not control, because it cannot produce that node's signature over the settings. This is the device half of what used to be a doubly-signed membership attestation — now carried by the CI itself, which lets the group's admitting op stay single-signer. Because device-CIs are immutable, changing a device's settings means publishing a *new* device-signed device-CI and pointing the group at it via `amend_device`.

### Membership ops

Each op is a Statement appended to the device-group collection, signed by the DGID key. Under the MVP `shared-key` scheme, any of the group's own nodes can append one.

| Op | Effect | References |
|---|---|---|
| `add_device` | admit a device | a device-CI |
| `amend_device` | replace a device's current settings | the new device-CI (matched by `pubkey`) |
| `revoke_device` | drop a device immediately | the device `pubkey` |
| `reinstate_device` | re-admit a previously revoked device | an existing device-CI |

```toml
[attestation]
kind      = "amend_device"
dgid      = "dgid:abc123..."
device_ci = "ci:9b2c..."             # the new device-signed settings
ownership = "owned"                  # set by the admitting owned node: owned | leased
timestamp = "2026-06-07T18:30:00Z"   # originating node's local clock — informational only
prev      = "blake3:..."             # advisory link to the prior head

[proof]
kind = "group-signed"
dgid = "dgid:abc123..."
sig  = "base64:..."                  # DGID key over blake3(canonical(claim-fields))
```

**Revoke / reinstate.** A device can be dropped at any time with `revoke_device` and brought back with `reinstate_device`. Reinstatement re-uses the device's existing signed settings block, so it only works while that block's `expires` TTL is still in the future. Once the TTL lapses there is nothing valid to reinstate — the device must publish a fresh signed settings block (a new device-CI with a new TTL) and be added again. The TTL is therefore the bound on how long a group can revive a device on its own signature alone. (Note the corollary: a device cannot *guarantee* it has left until its last-signed settings block expires — fine within a personal mesh you fully control, but see open questions for non-personal groups.)

### Owned and leased members

Not every member is your hardware. The owned node that admits a device tags its entry as one of:

- **owned** — your own device. Holds the group signing key (under `shared-key`), can append ops, and its clock is **authoritative** for reconciliation.
- **leased** — third-party capacity (a hosting provider, a community server). Serves the capabilities in its device-CI but does **not** hold the group key, cannot append ops, carries lower trust, and its self-asserted timestamps are **informational only**.

A leased node's signed settings TTL is the machine-readable form of its contract: "paid for one month" → a 30-day TTL. When the term ends the provider can simply stop responding, or return a `banned` / `revoked` response, signalling the operator (or an automated owned node) to `revoke_device` it. Because a leased node can never sign group ops or set ordering, a wrong or hostile provider clock cannot corrupt your membership history. Run two or more providers for redundancy and speed; lean on owned devices for anything trust-sensitive.

### Reconciling concurrent appends

Reconciliation is **local and CRDT-style** — there is no adoption round, no voting, nothing to "win." Every node keeps the set of ops plus their witness receipts and independently folds them into the same ordered state. A late message only ever lowers a minimum; it never triggers a network-wide reorg.

**Witness receipt.** When an owned (trusted) device first sees an op, it emits a small signed receipt — `(node, op, observed_at)` signed by its NodeId — and gossips it alongside the op. This receipt is the unit of ordering authority. A leased node's receipt, and the op's own originating `timestamp`, are informational only and never count.

**Order.** An op's effective order-time is `min(observed_at)` over receipts from owned devices; ties break on lower `blake3(canonical(op))`. Fold all ops by that key → last-writer-wins per item, identical on every owned node. Until an owned device has witnessed an op its order is *pending* — harmless, since it cannot affect your view of the group before you have seen it.

For a mesh of owned `a`, `b`, `c` plus leased `z`: `z` may emit an op (and a receipt) stamped with its own clock, kept for reference only; the receipts from `a`/`b`/`c` decide the order, and the earliest owned observation wins.

**Finality and GC.** An op's order freezes once it has been witnessed by a majority of owned devices **or** is older than a configured Δ, whichever comes first — after which no earlier receipt can legitimately appear, so the redundant receipts are dropped and only the winning one is kept. *This* is where losing candidates are discarded: not by a vote, but by garbage-collecting a settled set. Exact quorum size and Δ are config/implementation; "how long until a revoke is certain" is the only user-visible knob.

**Sealed checkpoints (optional).** Periodically an owned device may seal a run of finalized ops into a `prev`-linked block — a compact, tamper-evident snapshot for fast sync and history. A block is a checkpoint, not a vote: it commits to an order the fold already settled.

This is the general rule for any multi-signer collection — see [collections.md](../../../schema/wiki/design/collections.md#reconciling-multi-signer-collections). Hardening against a *trusted* device with a wrong clock (strict validation, etc.) is implementation, not pinned here — see open questions.

### Capability tags

Tags live in the device-CI's signed `settings` object. They are free-form strings scoped to the group. Standard tags are defined in `spirit-schema`; custom tags are allowed. Examples:

```
"indexer"       — serves index queries for this group's content
"build-cache"   — serves built artifacts
"image-cache"   — serves image blobs
"cat-pics"      — serves cat-pic content specifically
"video-stream"  — serves video streaming
"cdn"           — general blob serving
```

The routing layer uses tags to select which members to contact for a given query — see [routing.md](../../../routing/wiki/design/routing.md). A client querying for cat pics skips nodes not tagged for that service before sending a request. If a node receives a query for a service it is not attested for, it drops it — and the client can diagnose the mismatch by inspecting the membership attestations.

### One node, multiple groups

A NodeId can belong to any number of DGIDs simultaneously. It publishes a separate device-CI per group — each scoped to that group's `dgid` with its own tags and TTL — and each group's collection admits it independently. Revoking it from one group does not affect the others.

```
nodeid:korra
  ├── device-CI → dgid:your-devices    tags: ["indexer", "build-cache"]  (no TTL set)
  └── device-CI → dgid:cat-collective  tags: ["image-cache", "cat-pics"] expires: 2026-09-01
```

---

## Access Grants

A DGID can delegate query access for specific services on a specific node to contacts or subscribers — without making that node a member of the grantee's DGID.

```toml
[access-grant]
grantor  = "dgid:abc123..."
grantee  = "dgid:contact-alice..."   # or a whole contact group
node     = "nodeid:cf-edge-7..."
services = ["image-cache"]
expires  = "2026-09-01T00:00:00Z"   # must not exceed the node's membership expiry

[proof]
dgid_sig = "base64:..."   # grantor's DGID key — only the grantor signs this
```

The node receiving a query checks: valid membership attestation for this node AND valid access grant covering this requester and service? If either is expired or missing, the request is dropped. The requester checks the same attestations locally before routing — they already know whether the route is valid.

**Example: paid CDN service**

```
1. User pays for a spirit gateway service, 30-day term
2. The service's edge node publishes a device-CI: settings signed by the edge node, image-cache tag, 30-day TTL
3. Service appends add_device referencing that device-CI → edge node is a member
4. User publishes access grants to their contact list for image-cache service
5. Contacts route image requests to the edge node via the user's DGID
6. Day 30: the device-CI's settings TTL lapses, node drops out of routing automatically
7. User renews → edge node publishes a fresh device-CI, service amend_devices it → service resumes
```

---

## Expiry and Renewal

A device-CI's signed `settings` block and every access grant carry an optional `expires` field (RFC 3339). When it lapses:

- The index evicts it on the next TTL sweep
- The routing layer stops selecting that node
- Contacts holding a derived access grant get no valid route

Renewal is a new device-signed device-CI with a fresh TTL, admitted via `amend_device` and propagated through the device-group collection. Non-renewal is the primary lapse mechanism — no broadcast needed. Unlike the old set-of-attestations model, **emergency revocation is now first-class**: a `revoke_device` op drops a compromised device immediately in the next folded state, no separate revocation-document format required.

---

## Group Signing Schemes

How a DGID exercises its private key is declared in the DGID document. The signature on membership attestations and access grants is always a standard Ed25519 signature verifiable against the DGID public key — the internal scheme is invisible to verifiers.

### `shared-key` *(MVP)*

Every member device holds the full DGID private key. Any member can sign unilaterally. Suitable for personal meshes with a low external threat model.

### `frost`

Threshold Schnorr signatures (FROST, RFC 9591). The DGID private key is never held by any single device. M-of-N devices must participate in a signing round. Output is a standard Ed25519 signature. Key refresh allows redistributing shares to a new M-of-N without changing the DGID public key — added devices get new shares; removed devices' shares are fully invalidated.

```toml
signing_scheme = "frost"
threshold = 2   # minimum signers required
total = 3       # total shares distributed
```

### `multisig`

M-of-N individual node signatures. Each NodeId signs; verifiers confirm M current members signed. More verbose than FROST but individually auditable. No shared secret.

---

## Device Pairing (QR / NFC)

Adding a device to your Mesh group:

1. New device generates its Ed25519 keypair and a **device-CI** — a `settings` block (group `dgid`, tags, role, TTL) signed with its own key — and displays `ci:<hash>` (or the document) as a QR code
2. Trusted device scans → authenticates over iroh → appends an `add_device` op referencing that device-CI, signed with the DGID key
3. (Under `shared-key`) the new device also securely receives the DGID private key so it can sign as the group — see open questions
4. All other Mesh members fold the new op via gossip → the device is trusted with the capabilities its settings declare

The device's consent is already in its self-signed device-CI, so there is no partial-attestation countersigning round-trip — the group simply admits a CI the device already signed.

---

## Sharing via QR / Deep Link

```
spirit://ci/<blake3-hash>
spirit://collection/<dgid>/<name>
spirit://collection/checkpoint/<blake3-hash>   # sealed checkpoint snapshot
spirit://group/<dgid>                          # add this group as a contact
spirit://group/web/<domain>                    # DNS-resolvable form
```

Opening `spirit://group/<dgid>` prompts "Add [name] as a contact?" with a trust level selector.

---

## Open Questions

- **Tag schema governance** — standard tags defined in `spirit-schema`; custom tags namespaced by DGID (`dgid:abc.../my-tag`); exact governance TBD
- **Device-CI delivery** — how the device's self-signed device-CI reaches the group node that appends `add_device`. Simpler than the old partial-attestation dance (no countersign round-trip): QR/NFC for proximity, iroh channel for remote
- **Membership / head discovery** — when a contact adds your DGID, how do they fetch the current head of your device-group collection? Shares the [collection head-discovery question](../../../schema/wiki/design/collections.md); options: HTTPS well-known, iroh ticket in the DGID document, gossip topic
- **DGID document format** — largely subsumed: the device-group collection *is* the membership record. Open part: where group metadata (name, description, relay hints, signing scheme) lives — a reserved metadata entry in the collection, or a small separate signed doc
- **`shared-key` key distribution** — how a new device securely receives the DGID private key: QR-encoded on a trusted device, encrypted blob over iroh, derived from a master seed
- **Admin key custody** — losing the DGID private key means losing the ability to append ops; recovery options: seed phrase, secondary admin NodeId, FROST threshold
- **Finality knobs** — ordering and receipts are settled (local CRDT fold, signed witness receipts, finalize on owned-majority or age Δ). Remaining: the default quorum size and Δ, and whether they are fixed or per-group config — an implementation/tuning choice, not a schema one
- **Bad clock on a *trusted* node** — trusted-tier ordering removes the untrusted-clock attack, but an owned device with a wrong clock can still skew order; hardening (strict block validation, dropping ops too old to have reached a quorum of owned nodes) is treated as implementation, not design — confirm none of it forces a schema change
- **Guaranteed departure before TTL** — a group can `reinstate_device` on an old signed block until its TTL lapses, so a device cannot unilaterally bind itself as "left" early; decide whether non-personal groups need a device-signed `leave` that the group must honour
