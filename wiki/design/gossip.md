# Gossip — How Nodes Find Each Other

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

> **Status (2026-09-04): ref names now have an owner.** `wanted_names` folds in
> a peer's advertised names only when that peer is trusted at cache level or
> above, and `best_provider` skips untrusted providers, so a stranger can no
> longer introduce a ref name into your store. Seeding a peer grants it cache
> trust; everything else starts unknown (`spirit-node trust <id> <level>`).
> Blob *bytes* stay fetchable from anyone — the hash verifies them.

A blob ticket is a one-way introduction. Scan A's QR and you know A; A's other
peers stay invisible to you, and you stay invisible to them. Every node ends up
pointing at whoever handed it a ticket, and nothing else. That is a star, and a
star has a centre that everything depends on.

Gossip replaces it with a mesh. When two nodes talk, they exchange the peers
they know and what each of them holds. A node that hears about a peer it has
never met can dial that peer directly — iroh does the traversal; the only thing
missing was the introduction.

## The protocol

Its own ALPN, `spirit-gossip/0`. Deliberately not layered onto `iroh-blobs`'
ALPN: bulk transfer and membership have different message shapes, different
sizes and different failure modes, and a peer that speaks one need not speak
the other. Keeping them separate means either can change without the other
noticing.

One CBOR round trip on a bidirectional stream, matching spirit's existing
encoding choice everywhere else:

```
dialer  --> View { addr, peers, refs }
dialer  <-- View { addr, peers, refs }
```

`View` carries the sender's own `EndpointAddr`, every `EndpointAddr` it knows,
a `RefAdvert { name, manifest, total, held }` per ref, optionally the
sender's own open `TableAdvert { name }`, and `heard_tables`, the open tables
it has heard from other hosts as `HeardTable { host, table, heard_at }`.
Both sides merge what they receive.
There is no separate request and response type because the message is
symmetric — each side is telling the other the same kind of thing.

Two ref shapes ride the same advert. A card-set ref (`hob`) names a manifest
of cards and counts its images; a module ref (`modules/<name>`, stored as
`refs/modules/<name>`) names a `spirit_core::modules::ModuleManifest` and
counts exactly one blob — the hardened wasm module — so `total`/`held` and
the converge rules apply unchanged. Ref names arriving from peers pass
`safe_ref_name` (a flat name or a single `modules/` segment, no separators or
traversal) before anything is written under `refs/`. The converge loop also
services `Mesh::request_blob` wants — single blobs requested by hash (a
joiner fetching a genesis-pinned module), pulled from a hinted provider,
then complete `modules/*` advertisers, then any known peer — and re-imports
blobs that appeared in the local store since startup so a freshly published
module is servable without a restart.

## Table adverts: presence over gossip, not another handshake

A node hosting a game table advertises it in `table`, and only the host ever
fills that field: hearing a `table` means the sender itself said it over an
authenticated connection. Every view exchange doubles as the heartbeat:
setting or clearing the advert bumps the view version so the change spreads
within a round or two, while an unchanged advert refreshes its timestamp
without bumping anything — the quiet-down rule holds. Receivers expire a
first-hand advert not refreshed for 150 seconds, two missed idle-recheck
periods, which only matters when a host dies without clearing; a clean close
propagates in seconds.

### Relay: a table travels more than one hop

Originally adverts stopped there, and a seat saw a host's table only once it
had gossiped with the host directly. Gateways introduce the host's address
but the advert itself never crossed them, so a client behind one gateway and
a host behind another sat in the same mesh and never saw each other's table.
A view now also carries `heard_tables`: every open table the sender knows
about, first-hand or relayed, as `HeardTable { host, table, heard_at }` with
the host's node id and the wall-clock second at which the host itself was
last heard saying it. That timestamp is set when the advert is heard
first-hand and carried unchanged through every relay — a relayer never
re-stamps hearsay — so a dead host's table cannot be kept alive by nodes
echoing it to each other. The field is `#[serde(default)]` and omitted when
empty, so a node from before it parses views with it and sends views without
it.

`tables.rs` keeps the books, and every rule in it is a pure function over
the two maps so it can be unit-tested with a fake clock:

- **First-hand beats hearsay.** A host's own `table` replaces every relayed
  entry for that host, and while a first-hand entry is live, relayed entries
  for the same host are ignored. Only once the host has gone silent to us for
  150 seconds does hearsay about it count again — the case where we lost the
  host but a peer still has it.
- **Never self-echo.** Relayed entries naming our own node id are dropped
  (our table goes out in `table`, never in `heard_tables`), and so are
  entries naming the sender — the sender's own table arrives in its `table`
  field, and a relay of it is at best stale.
- **Only known hosts.** A relayed entry whose host is not in our peer
  registry is dropped: a table we cannot dial is noise, and a pruned peer
  stays pruned against hearsay, the same rule membership follows.
- **Dedupe by (host, table name).** Re-hearing a table from several relays
  keeps the newest `heard_at` and lists it once.
- **Expiry.** A relayed entry is dropped once its `heard_at` is five minutes
  old, or five minutes after we learned it, whichever comes first; the second
  bound means a peer with a wrong clock cannot pin an advert on us forever.
- **A clean close outranks stale hearsay.** When a host withdraws its table
  first-hand, the withdrawal time is remembered for five minutes, and relayed
  entries for that host with an older `heard_at` are refused. A table the
  host reopens afterwards carries a newer stamp and passes.
- **Quiet-down holds.** `hear` and `learn` report a change only when the set
  of visible `(host, name)` pairs changed; refreshed timestamps never bump
  the view version.

`Mesh::open_tables` is the read side: first-hand and relayed tables together,
each `OpenTable { host, name, relayed }` saying which it is, never the node's
own table. Anyone in the mesh can see and dial what it returns — the trust
surface is documented in kai's `wiki/design/multiplayer.md`.

`EndpointAddr` is the currency because it already derives `Serialize` in
iroh-base and it is exactly what `Endpoint::connect` accepts. A node that
learns one can dial it with no further lookup. Bare node ids would have worked
too — discovery resolves them — but carrying addresses means the first dial
does not depend on DNS or pkarr being current.

## Membership converges, then goes quiet

Naive gossip chatters forever. Three things stop it:

- **Dedupe by node id.** Peers live in a map keyed by id, so re-hearing about a
  peer is a no-op rather than a new entry.
- **A view version.** A counter bumps only when a merge actually taught us
  something. A peer is re-contacted when our version has moved since we last
  talked to it, or when an idle interval has elapsed. Once every node agrees,
  no version moves and gossip drops to a heartbeat.
- **A fan-out cap.** At most four peers per round, regardless of how many are
  known.

Repeated dial failures park a peer rather than retrying it forever, and the
failure count and last error are recorded so a peer behind a NAT iroh cannot
traverse shows up as a visible failure instead of silence.

Peers do get forgotten. A peer silent for an hour is pruned from the registry
and from the mesh's table, unless it holds a live connection or was seeded on
the command line. Silence is measured strictly: activity means the peer itself
took part in an exchange — a connection, bytes moved, a completed gossip round.
A failed dial, a secondhand mention in someone else's table or a pull error
edit the record without counting as contact. And a pruned peer stays forgotten
against hearsay: other peers keep listing it for a while, and those
introductions are ignored. It comes back only by speaking to us directly, which
an alive node does within a round because it learns our address from the same
gossip. Re-seeding clears the mark. Without the second rule every node prunes
and re-learns the same dead id in turn and the mesh never converges on dropping
it.

## Data converges too

Membership alone would leave every node knowing about a set it does not have.
Each ref advert says how many blobs that peer holds out of the manifest total,
so a node can see it is behind. It then pulls the missing blobs from the peer
holding the most, preferring one it already has a path to.

The source does not matter, and that is the point of content addressing: every
blob is verified against its hash on write, so the same hash from any peer is
the same bytes. There is no need to trust the peer you pulled from — a
corrupted or hostile response fails the hash check and is discarded.

A node pulls any ref it hears about, so the mesh fully replicates by default.
`--no-pull` turns that off for a node that should introduce peers without
storing 16 MB of card art — a rendezvous node with no interest in the content.

## Scryfall is the last resort, and only one node uses it

If a ref is missing from every node in the mesh, someone has to go get it from
upstream. If everyone does, Scryfall gets hammered by the whole mesh at once
for the same set.

The node with the **lowest id among the peers it can currently reach** is the
only one that acts. Node ids are stable public keys, so every node computes the
same winner from the same membership without any coordination. Unreachable
peers are excluded from the comparison, so a dead node holding the lowest id
cannot wedge the mesh into never backfilling.

Two more guards: a settle window means nothing is ingested in the first seconds
after startup, before gossip has had a chance to reveal that a peer already
holds it; and a per-set cooldown stops a failing backfill retrying in a loop.

The ingester itself lives outside spirit — `agni-importers`, in the card
engine that owns game knowledge — and the mesh reaches an ingester only
through the `Mesh::set_backfill` callback, so `spirit-node` carries no HTTP
or TLS dependency and no importer at all. kai never backfills and never
links one. The mesh owns the policy of *when* to backfill; the caller
supplies the *mechanism*, and today no shipped caller wires one — a mesh
with a ref nobody holds waits until a peer that ran the importer appears.

## What this does not do

- **No transitive reachability guarantee.** Learning an address is not the same
  as being able to use it. Two peers behind symmetric NATs may both be visible
  in gossip and still fail to connect. That failure is surfaced, not retried
  into oblivion.
- **No partial-ref merging across peers.** A node pulls a ref from the single
  best provider rather than striping across several. Simpler, and adequate at
  this size.
- **No authentication of adverts.** A peer can claim to hold a ref it does not.
  The lie costs a failed download and nothing else, because the blobs are
  content-verified — but a malicious peer could still waste a round. Group
  trust from `core/wiki/design/groups.md` is where that gets solved, not here.
- **No persistence of what was forgotten.** The forgotten set lives in memory.
  A restarted node accepts hearsay about a dead peer again, keeps it for an
  hour, and prunes it once more.
