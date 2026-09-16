# The Spirit Library

`spirit`

> "Form changes. Identity endures."

A content-addressed store and reproducible transformation pipeline for distributed applications, built on [iroh](https://github.com/n0-computer/iroh).

---

## Why

**Content identity should survive format changes.**
An mp3 and a flac of the same song are the same song. A package built for x86 and one built for arm are the same package. Traditional systems store these as unrelated blobs and leave you to manage the relationship. Spirit separates *what something is* (its Content Identity, or CI) from *what bytes represent it* (an output **blob**, addressed by its blob hash) — so re-encodings, rebuilds, and remasters don't break identity.

**Compute once. Share everywhere.**
If your desktop already transcoded a video or built a package, no one in your network should have to do it again. Spirit makes transformations content-addressed: the input, the tool, and the parameters together determine the output hash. The result is deduplicated across every peer who would have run the same transform — verified by hash, fetched from whoever has it, trusted via attestation rather than server.

**Trust should be yours to manage.**
Package managers trust a central server. Streaming platforms trust a company. Spirit's trust model is local: you manage a registry of groups (your own devices, friends, community caches) and set each one's trust level yourself. No account required, no authority to petition, no algorithm deciding what you see.

**Content sharing should be peer-to-peer, not platform-mediated.**
Social media started as people sharing what they found interesting — playlists, links, photos, recommendations. Platforms captured that and buried it under engagement algorithms. Spirit is infrastructure for the version of that idea that doesn't require a platform: curate a playlist, a reading list, a feed of releases — share it as a link your peers can follow, fork, and build on. When a trusted peer attests a new version of something in your collection, it shows up automatically. No app store, no algorithm, no platform to get banned from.

---

## In Practice

These are illustrative sketches of how spirit-based apps would behave. The record-level operations exist today in `spirit-node` (see Try It); the package and playlist apps do not.

**Installing a package without rebuilding**

```
# Desktop builds ffmpeg and publishes an attestation
$ bumi build ffmpeg@7.1.0
→ attested: (ci:<ffmpeg-7.1.0-hash>, td:<nix-aarch64-hash>) → blob:7f3a...

# Laptop wants ffmpeg — spirit checks trusted peers first
$ bumi install ffmpeg@7.1.0
→ found attestation signed by dgid:your-devices
→ fetching blob:7f3a... from korra (your desktop) — no rebuild
```

Your laptop verifies the BLAKE3 hash against the blob hash in the attestation. Trust comes from your group signature, not a server.

**Sharing a playlist that stays current**

```
# Share a playlist of CI references — not files, not specific encodings
$ spirit share playlist/uncle-iroh-favorites
→ spirit://collection/dgid:you.../uncle-iroh-favorites

# Your friend opens it. Their node resolves each CI to the best output blob they have.
# Six months later, a trusted peer attests a remaster of Leaves from the Vine.
# Your friend's player finds it automatically — the playlist didn't change.
```

Collections reference what content *is*, not which bytes to use. When trusted peers publish new attestations, subscribers pick them up without any action from the collection author.

**Sharing tea recipes with your nephew**

```
# Iroh adds Zuko as a contact and shares his recipe collection
$ spirit contact add dgid:zuko...
$ spirit share collection/tea-recipes --with dgid:zuko...
→ spirit://collection/dgid:uncle-iroh.../tea-recipes

# Zuko forks it — he owns the fork, can add his own notes
$ spirit fork spirit://collection/dgid:uncle-iroh.../tea-recipes
→ collection/dgid:zuko.../tea-recipes (forked_from: uncle-iroh)

# Iroh publishes a new white dragon blend recipe
# Zuko's app notifies him — he reviews and accepts the update
```

Contact-level trust means Zuko can see what Iroh shares with him, but Iroh's recipes don't influence Zuko's content index. Following is how you track someone's curation and receive their updates; forking is how you take a copy under your own control.

**Transcoding once, sharing across the network**

```
# Convert a video to 480p with a specific ffmpeg version and flags
$ spirit build ci:<film-2023-hash> \
    --via ffmpeg-transcode \
    --tool ci:<ffmpeg-6.1-hash> \
    --args "-vf scale=854:480 -c:v libx264"
→ published: (ci:<film-480p-hash>, td:<locked-td-hash>) → blob:2a8f...

# Anyone who wants the same output fetches blob:2a8f... from whoever has it.
# The hash proves the bytes are exactly what that transform would have produced.
```

---

## Try It

The `spirit-node` binary exposes every primitive so you can mint records by
hand and watch them interact. `--store <dir>` or `SPIRIT_STORE` picks the
store; the default is `~/.spirit/store`. `spirit-node help` lists everything.

```
export SPIRIT_STORE=/tmp/play
spirit-node identity
CI=$(spirit-node cir mint song '{"artist":"Uncle Iroh","title":"Leaves from the Vine"}')
TD=$(spirit-node tdr mint flac-encode '{"variant":"flac"}')
BLOB=$(spirit-node blob put leaves.flac)
ATT=$(spirit-node attest content $CI $TD $BLOB)
spirit-node collection add favorites $CI --label "Leaves from the Vine"
spirit-node collection attest favorites $ATT
spirit-node collection show favorites
spirit-node resolve $CI
spirit-node record show $ATT
spirit-node index
spirit-node blob list
```

Every one of those operations is also an HTTP route, and `spirit-node mesh
--gateway 8090` serves a browser UI at `http://127.0.0.1:8090/` over them:
daemon stats and peers, every identity with its artifacts and download
links, collections, blob upload, and forms to mint records and sign
attestations. Writes need the token the daemon writes to
`<store>/gateway-token`. The deployed instance is https://spirit.rae.blue
on the tailnet.

`record show` decodes any record to JSON and verifies its signature;
`resolve` lists every trusted artifact for a CI and the resolver's pick;
`index` prints the local fold. Mint a second transform and attest a second
blob to the same CI to see one identity carrying several artifacts. Two
stores on one machine, each with its own `--store`, give you two identities
to play trust levels against with `spirit-node trust`.

## Compared to

**Nix narinfo** is the closest prior art. Nix independently arrived at a `(recipe → output hash)` mapping — a derivation describes the build, a `.narinfo` maps the store path to the content-addressed output. Spirit generalises this: the CI is an explicit semantic document describing what content *is* (not a hash of the build recipe), trust is tiered across groups you choose (not a single binary-trusted HTTP cache), and delivery is P2P.

**IPFS** addresses content by the hash of its bytes — an mp3 and a flac of the same song are two completely unrelated CIDs with no connection between them. There is no logical content identity above the byte level, no group trust model, and no transformation provenance. Spirit runs on iroh's QUIC stack rather than IPFS's libp2p, which also gives it better NAT traversal and no DHT reliability issues.

**AT Protocol (Bluesky)** has the right instincts at the social layer: portable identity (DID) that survives moving between servers, multiple apps sharing the same identity, user-controlled data. Spirit's DGID plays the same role as a DID, and the two could integrate at the identity layer — your DGID anchored to an AT Protocol handle. Where they differ: AT Protocol is a federated server architecture built for posts and follows, not a P2P content store for packages, music libraries, or reproducible computation. If AT Protocol is "decentralise Twitter," spirit is "decentralise the content layer everything runs on top of."

→ [Full protocol comparisons and design rationale](wiki/design/comparisons.md)

---

## How it Works

Spirit's model rests on three primitives and one rule.

**Content Identity (CI)** describes what something *is*, independent of encoding or build. The `ci:<hash>` for "Leaves from the Vine" is the same address whether you have an mp3, a flac, or a remaster — the CIR (Content Identity Record) describes the song; its hash is always opaque. For software, version is part of identity: the CIRs for ffmpeg 7.1.0 and 7.2.0 are different, so their `ci:<hash>` values are different.

**Output blob** is the BLAKE3-hashed bytes, addressed `blob:<hash>`. Self-verifying: `blake3(bytes) == hash`. No trust required for the bytes themselves — anyone can serve them, and you verify on receipt.

**Attestation** is the signed claim linking a CI to an output blob: `(CI, TD) → blob`. The Transform Definition (TD) records the recipe — inputs, tool, parameters. A group you trust signs it; your node accepts it and indexes the mapping.

**The rule:** the CI → blob mapping always comes from an attestation you trust. Once you have it, you can fetch the bytes from anyone — including nodes you don't trust — because BLAKE3 verifies the result.

**Groups** are the trust model. A Group is an Ed25519 keypair representing a set of devices: your own mesh, a friend's devices, or a community cache. You assign each group a trust level:

| Level | Who | What it enables |
|---|---|---|
| Mesh | Your own devices | Full bidirectional index sync; attestations signed and accepted |
| Cache | Trusted caches, build farms | Their attestations accepted; CI feeds synced into your index |
| Contact | Friends, publishers | Shared collections visible; no influence on your index |

Trust is local. You set it; it is never asserted by remote groups and never transitive.

Everything else — routing, version resolution, CI feeds, mutable collections, device roles — builds on these three primitives and this trust model.

---

## Crates

| Crate | What it owns |
|---|---|
| [`core/`](core/) | Blob store, canonical encoding, typed addresses, the four record types, collections, identity, trust, the replication envelope |
| [`schema/`](schema/) | The CI kinds: module version collections and card / printing records |
| [`index/`](index/) | The local fold over collections: `ci → attestations`, back-links, external ids |
| [`routing/`](routing/) | Trust-ordered resolution of a CI to a blob, provider ranking |
| [`node/`](node/) | iroh serve/fetch, peer registry, gossip mesh, HTTP gateway for iroh-less clients |
| [`sdk/`](sdk/) | The public surface; what apps depend on |

---

## Start Here

- [Specification](wiki/spec.md) — **the source of truth**: primitives, trust, pairing, the daemon, and the gap between this design and the code
- [Terminology](wiki/design/terminology.md) — record names, the blob-hash convention, the CID warning — read this first
- [Architecture overview](wiki/design/architecture.md) — layers, iroh ecosystem, links to all design docs
- [Addressing model](core/wiki/design/addressing.md) — CIR, TDR, output blobs, recursive attestation, version bounds
- [Groups and trust](core/wiki/design/groups.md) — DGID, trust levels, signing schemes, device pairing
- [Developer API (sketch)](wiki/api/overview.md) — the intended app/CLI surface: read & write lifecycles, follow/fork — for anyone building on spirit

Spirit is in design phase. Implementation follows the design.

---

## Related

- `andrea/projects/spirit/oasis/` — artifact manager (consumer)
- `andrea/projects/spirit/bumi/` — package manager (consumer)

## Sharing a store over iroh

`spirit-node` moves a store between machines, content-addressed end to end:

```
cargo run -p spirit-node -- serve            # on the machine with the store
cargo run -p spirit-node -- fetch <ticket>   # on the other machine
```

`serve` imports the flat store into an iroh-blobs store alongside it
(`<store>/iroh/` — BAO outboards mean the bytes exist twice locally for now),
prints the node id and the node's identity ticket (an `EndpointTicket`,
rendered as a QR — the one thing another device needs to mesh with this node),
and prints one blob ticket per `refs/*` file; subdirectories such as
`refs/modules/` are namespaced refs read by their own code paths and are
skipped at serve time rather than advertised. `fetch` pulls
the ref's manifest, then every blob it lists, re-verifying each into the local
flat store and writing the ref. Spirit's blob hashes are BLAKE3, and BLAKE3's
tree root is the same hash iroh-blobs uses — so the two stores agree on every
address with no translation layer.

## Joining a mesh

`serve` and `fetch` are a star: each node only knows whoever handed it a
ticket. `mesh` adds transitive introduction, so nodes that were introduced to
the same peer find each other and talk directly:

```
cargo run -p spirit-node -- mesh                          # rendezvous node
cargo run -p spirit-node -- mesh --seed <ticket-or-id>    # join through a peer
cargo run -p spirit-node -- mesh --seed <id> --want <set> # …and pull a set
cargo run -p spirit-node -- mesh --no-pull                # introduce, do not replicate
```

A seed takes one of four shapes, and `Mesh::seed`, `--seed`, `--seed-file`
and `<store>/seeds` all accept every one of them:

| Seed | What it names |
|---|---|
| `endpoint…` identity ticket | a node id plus its dial addresses |
| `blob…` blob ticket | the same, from a `fetch` hand-off |
| bare 64-hex node id | a node id; discovery resolves the addresses |
| `http://host[:port]` or `https://host[:port]` gateway URL | a node that runs the HTTP gateway; resolved at seed time |

A gateway URL survives the node behind it changing its key: the seed fetches
`<url>/gateway/status` when it is applied, reads the gateway's current
`ticket` (or `node_id` from an older gateway) and seeds that. Bare node ids
baked into a client go stale the moment the device key rotates, which is why
the dev gateways should be seeded by URL. `http://` URLs are fetched by a
built-in blocking HTTP/1.1 GET over plain TCP; spirit-node carries no TLS, so
an `https://` URL needs the caller to install a fetcher first —
`mesh.set_fetch(Arc::new(|url| my_client.get(url)))`, returning the body
as a `String` — and the hook, once set, serves every scheme. A fetch that
fails or a status that names no usable ticket or node id is a seed error like
any other. The node that introduces two others need not hold any blobs.
Nodes exchange the peers and refs they know over `spirit-gossip/0`, then pull
whatever they are missing from whichever peer has the most of it. If a ref is missing from every
node, the mesh can elect exactly one — the lowest reachable node id — to
backfill it through a caller-supplied hook (`Mesh::set_backfill`); spirit
ships no ingester of its own, so a bare `spirit-node mesh` simply waits for a
peer that has the content. The Scryfall ingester lives in agni
(`../agni/importers/`, the `ingest-scryfall` bin) and fills a store the mesh
then replicates. See [`wiki/design/gossip.md`](wiki/design/gossip.md).
