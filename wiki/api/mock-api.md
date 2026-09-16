# spirit — Mock API Surface

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Pre-implementation sketch.** Rust-flavored pseudo-signatures, grouped by what
> a developer is trying to do. Types are illustrative; error handling elided
> (assume every fallible call returns `Result<_, SpiritError>`). The point is the
> *shape* of each call and where trust enters — not the final names. Naming follows
> [terminology.md](../design/terminology.md): records (CIR/TDR/AR/CR) addressed
> by typed blob-hash handles (`CiHash`, `TdHash`, …); raw output content is a
> `BlobHash`. See [overview.md](overview.md) for the two core lifecycles.

Crate ownership is noted per group so the surface maps back onto the workspace
(`core`, `index`, `routing`, `schema`, `sdk`).

---

## Node and Session — `sdk` over `core` + `index`

A `Node` is the handle an app holds for its whole session: local store, index,
identity, and network. One per process; apps share the same on-disk store.

```rust
let node = Node::open(Config {
    store_path:  "~/.local/share/spirit",
    identity:    Identity::Dgid(my_dgid),  // see Identity & Groups
    role:        DeviceRole::Node,         // leaf | node | indexer | hub
    replication: Replication::Cache,       // light | cache | archive
    limits:      Limits::preset("laptop"),
})?;

node.online()?;     // join iroh: discovery, relays, gossip topics
node.shutdown()?;
```

`role` controls query-serving; `replication` controls how much followed content
this device stores (see [routing.md](../../routing/wiki/design/routing.md)).
They are independent axes.

---

## Identity and Groups — `core`, surfaced by `sdk`

A `Dgid` is the user's stable identity; a `NodeId` is one device. A group's
membership is a [device-group collection](../../core/wiki/design/groups.md#device-membership):
a device publishes a self-signed **device record** (a CIR, `kind=device`); an owned
node admits it with an `add_device` op.

```rust
// This device authors and signs its own device record (settings + TTL).
let dev: CiHash = node.publish_device_ci(DeviceSettings {
    dgid: my_dgid, tags: ["indexer", "build-cache"], expires: None,
})?;

// An existing owned node admits it (single-signer DGID op).
node.groups().add_device(dev, Ownership::Owned)?;       // amend/revoke/reinstate too

// Trust registry — entirely local, never asserted remotely
node.trust().set(dgid_friend, TrustLevel::Contact)?;
node.trust().set(dgid_cache,  TrustLevel::Cache)?;
node.trust().level_of(some_dgid) -> Option<TrustLevel>;

// Trust policy: which proof kinds you accept, at which level
node.trust().set_policy(TrustPolicy::from_toml(...))?;
```

`TrustLevel` = `Mesh | Cache | Contact`. `Mesh` is reserved for your own DGID's
devices. See [groups.md](../../core/wiki/design/groups.md).

---

## Content Identity Records (CIR) — `core` + `schema`

A CIR says *what content is*. Its address is the hash of its **canonical encoding**
(authoring format is flexible — see terminology.md). CIRs are immutable.

```rust
// Build a CIR (schema-validated per `kind`)
let cir: Cir = Cir::builder("music-track")
    .field("artist", "Uncle Iroh")
    .field("album",  "Tales of Ba Sing Se")
    .field("title",  "Leaves from the Vine")
    .external_id("isrc", "US-XYZ-06-00001")   // optional, enables dedup
    .build()?;

let ci: CiHash = cir.address();                // ci = blake3(canonical(cir))
node.store().put_ci(&cir)?;                    // store the record itself

// Look one up
let cir: Option<Cir> = node.store().get_ci(&ci)?;

// Query by constraint (against the local index)
let matches: Vec<CiHash> = node.index().query(CiQuery {
    kind: "package", name: Some("ffmpeg"), version: Some("^1".parse()?), ..default()
})?;
```

---

## Resolution — the read path — `routing` + `index` + `core`

Turn a CI into bytes you can trust. This is the flow most apps spend their time in.

```rust
// One-shot: resolve a CI to the best output blob per policy, then fetch + verify.
let bytes: Bytes = node.resolve(&ci, ResolvePolicy {
    quality: QualityPref::Highest,     // schema-defined ordering per kind
    prefer:  Prefer::LocallyCached,    // cache | trust(most_attestors) | ...
    scope:   Scope::Cluster,           // local | cluster | network
})?;                                    // internally: pick blob, fetch, verify

// Or drive the steps yourself:
let atts: Vec<ContentAtt> = node.index().attestations_for(&ci)?;   // (ci,td)→blob
let atts = node.trust().filter(atts);                              // trusted only
let blob: BlobHash = ResolvePolicy::default().pick(&atts)?;        // choose output
let bytes          = node.blobs().fetch(&blob)?;                   // from anyone
assert_eq!(blake3(&bytes), blob);                                  // self-verify

// Miss handling is first-class: a federated query returns hits OR forwarding hints.
let resp: QueryResponse = node.routing().query(QueryRequest {
    ci: ci.clone(), scope: Scope::Network, ..default()
})?;   // Hit(att, blob_hints) | Miss(routing_hints, confidence) | Partial(..)
```

---

## Build and Attest — the write path — `routing` + `core`

Produce an output for a CI and publish a signed claim that the CI yields it.

```rust
// 1. Author an unlocked TDR: a recipe with CI/query inputs.
let tdr = Tdr::builder("nix-build")
    .input("source",  Input::Query(CiQuery::parse("package ffmpeg ^1")?))
    .input("nixpkgs", Input::Query(CiQuery::parse("nixpkgs-channel nixos-24.05")?))
    .build()?;

// 2. Lock it: pin every input CI→blob against the index. This is itself attested.
let locked: TdHash = node.routing().lock(&tdr)?;    // (ci:tdr, td:resolve) → td:locked

// 3. Execute the locked TDR to get bytes (builder/transcoder/fetcher).
let bytes: Bytes    = node.runtime().execute(&locked)?;
let blob:  BlobHash = node.blobs().add(bytes)?;     // blob = blake3(bytes), stored

// 4. Sign the content attestation with your group key and publish it.
let att: AttHash = node.attest_content(ContentClaim { ci, td: locked, blob })?;
node.index().put(&att)?;
node.gossip().publish(&att)?;        // flows to your mesh / followers
```

`node.attest_content` produces a `group-signed` proof by default; multisig,
reproducible, and zk proof kinds are selectable. See
[attestations.md](../../core/wiki/design/attestations.md).

---

## Blobs — `core` over `iroh-blobs`

Bytes are content-addressed and self-verifying. Trust never applies to blobs.

```rust
let blob:  BlobHash = node.blobs().add(bytes)?;       // store, returns blob hash
let bytes: Bytes    = node.blobs().fetch(&blob)?;     // fetch from any provider
let have:  bool     = node.blobs().has(&blob)?;
node.blobs().pin(&blob)?;                              // exempt from eviction

// Multi-file outputs are an iroh-blobs Collection (a manifest of named blobs).
let blob = node.blobs().add_tree("/path/to/package")?;
```

---

## Collections — `schema` + `sdk` <a id="collections"></a>

A collection is a named, ordered list of CI references, stored as an **append-only
op-set** ordered by trusted-witness reconciliation (see
[collections.md](../../schema/wiki/design/collections.md)). Apps surface two
verbs over someone else's collection — **follow** and **fork**.

```rust
// Create, then edit by appending ops (you are the owner).
let col: ColHash = node.collections().create("playlist", "uncle-iroh-favorites")?;
node.collections().append(col, Op::Add(Item {
    ci: ci_a, label: "Leaves from the Vine".into(), default_td: None,
}))?;
node.collections().append(col, Op::Add(Item {
    ci: ci_b, label: "Brave Soldier Boy".into(), default_td: None,
}))?;
node.collections().append(col, Op::Remove(ci_b))?;     // another op; nothing is rewritten

// Read the current folded state, or the raw op log.
let items: Vec<Item> = node.collections().state(col)?;   // folded by witness order
let ops:   Vec<Op>   = node.collections().ops(col)?;     // full append-only log

// Optional: seal a tamper-evident checkpoint of finalized ops for fast sync.
let cp: CheckpointHash = node.collections().seal(col)?;
```

### Following (the default)

```rust
node.collections().follow(col, FollowMode::default())?;  // trust owner, pull ops live

// Suggest a change back to the owner: a signed proposal they can approve.
let proposal = node.collections().suggest(col, Op::Add(ci_new))?;
node.send_to(owner_dgid, proposal)?;
// Owner side: review, approve → the approved op is appended + propagates to followers.
let inbox: Vec<Suggestion> = node.collections().pending_suggestions(col)?;
node.collections().approve(suggestion_id)?;   // owner signs + appends the op
```

### Forking

```rust
// Fork: take ownership. Re-sign the ops under your key, retaining the originals
// for provenance/attribution. v1 forks do not track their source.
let mine: ColHash = node.collections().fork(col)?;   // forked_from recorded
// (Overlay forks / merge / rebase: post-MVP.)
```

### Replication of followed content

```rust
// How much of a followed collection's blobs this device stores.
node.collections().set_replication(col, Replication::Light)?;    // metadata only
node.collections().set_replication(col, Replication::Cache)?;    // hot blobs, pruned
node.collections().set_replication(col, Replication::Archive)?;  // durable replicas
```

---

## Relations — `core` <a id="relations"></a>

Because CIRs are immutable, you never edit one — you publish a signed **relation**
between two CIs. Same mechanism handles typo corrections, cross-node dedup, and
version lineage.

```rust
// "ci_typo is replaced by ci_correct" — re-attest the same output blobs to
// ci_correct first, then publish the relation so trusting nodes prefer the fix.
node.attest_relation(Relation {
    relation: RelKind::SupersededBy,   // SameAs | SupersededBy | PreviousVersion
    from: ci_typo,
    to:   ci_correct,
})?;

// Read relations during resolution / display.
let rels: Vec<RelationAtt> = node.index().relations_for(&ci)?;
```

For an **owned** CIR (one with an `owner` field), only the owner's relation
attestations are authoritative. For ownerless CIRs, each node honors relations
from groups it trusts — convergence is local, never global. See
[attestations.md](../../core/wiki/design/attestations.md#relation-attestation).

---

## Deep Links — `sdk`

```rust
let link: Url = node.link().ci(&ci);              // spirit://ci/<hash>
let link      = node.link().collection(dgid, "uncle-iroh-favorites");
                                                  // spirit://collection/<dgid>/<name>
let target: LinkTarget = node.link().resolve("spirit://ci/7f3a...")?;
// LinkTarget::Ci(ci) routes to the app registered for that CIR's `kind`.
```

Scheme is **type-first, then authority** (`spirit://<type>/<dgid>/<name>`); a bare
`ci:` / `blob:` form is authority-less by nature. See
[sdk.md](../../sdk/wiki/design/sdk.md#deep-link-routing).
