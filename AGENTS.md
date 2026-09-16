# spirit — Agent Rules

Content-addressed store and reproducible transformation pipeline over iroh.
`wiki/spec.md` is the source of truth for the design; every other design doc
is superseded by it and kept for rationale. Read it before using any spirit
term in code or docs — the terms are load-bearing and used precisely. Its
section 13 is the ordered list of changes that take the code to the spec;
work from that list and update the spec's markers in the same change.

## Layout

| Crate | Path | Role |
|---|---|---|
| spirit-core | `core/` | the primitives: blob store (blake3, verify-on-read), deterministic-CBOR `canonical`, typed `address`es (`ci:`/`td:`/`att:`/`col:`/`blob:`), the four `record` types, op-set `collection`s and their fold, `identity` (the store's Ed25519 key is its DGID), `trust` levels, the `refs` API every consumer writes named pointers through, and the replication `envelope` the mesh counts blobs with. `modules.rs` is the retired single-pointer module ref, kept only so old stores still read |
| spirit-schema | `schema/` | the CI kinds: `modules` (a module version collection — a `wasm-module` CIR per version, each attested to its wasm blob, resolved by version then trust) and `cards` (card / printing CIRs, rules records, the catalog collection) |
| spirit-index | `index/` | the local fold over collections: `ci → attestations`, CIR back-links, `external id → ci` |
| spirit-routing | `routing/` | trust-ordered resolution of a CI to a blob, and provider ranking |
| spirit-node | `node/` | iroh serve/fetch, the peer registry (`peers.rs`), the gossip mesh (`gossip.rs`, `mesh.rs`), the `assets` index (`assets.rs`: one `assets` ref per node — a canonical-CBOR `{kind, refs, entries: key → blob}` map that advertises like any ref; `Mesh::find_asset`/`missing_asset_indexes` answer a key from the indexes peers advertise and `mesh::fetch_blob` pulls one blob from a named provider on demand, so a consumer asks the mesh before any URL), and the read-only HTTP gateway for iroh-less clients (`gateway.rs`, see `wiki/design/gateway.md`). `SPIRIT_QUIC_PORT` pins the native endpoint's UDP port (`bound_builder`) so a router can forward it and peers dial the node directly by id. Compiles to `wasm32` with `default-features = false`: the fs store, gateway, CLI and converge loop sit behind the default `native` feature, while `serve_in_memory_with` runs endpoint + router + gossip + mesh on `n0-future` spawns with an in-memory blob store — timers and spawns in shared code must come from `n0_future`, never `tokio` directly |
| spirit-sdk | `sdk/` | the public surface; downstream crates depend on this one |

The Scryfall ingester moved out: it is `agni-importers`
(`../agni/importers/`), because a card importer is game knowledge and spirit
carries none. The table-session transport moved with it, to `agni-net`
(`../agni/net/`).

Consumers write named pointers through `spirit_core::refs`, never by hand into
`<store>/refs/` — that hand-rolled write is what agni did while the collection
primitive was missing, and it is why spirit briefly carried a card schema.

`spirit-sdk` is the intended dependency for consumers. `agni`
(`../agni/`, the card game framework) depends on it by Cargo path
dependency, so a breaking change to the sdk surface breaks agni's build with no
version pin to soften it — CI builds both.

## Dev Environment

devenv provides the toolchain; there is no system-wide cargo.

```
direnv allow          # or: devenv shell
build | unit-test | clippy | fmt | fmt-check
```

These five scripts are defined in `devenv.nix` and are the same commands CI
runs. Change them there, not in `.woodpecker/spirit.yml`.

## Rules

- The test script is `unit-test`, never `test` — bash resolves `test` to its
  builtin before any devenv script, so a script named `test` is unreachable.
  The same trap bites `fmt` in any shell where coreutils precedes the devenv
  scripts on PATH: coreutils `fmt` waits on stdin forever. Check `type fmt`
  before chaining it, and fall back to `fmt-check` or `treefmt`.
- `fmt` and `fmt-check` format the WHOLE monorepo, not just spirit. That is
  deliberate: one root `treefmt.toml` keeps style identical across orgs. Run
  them freely — the repo is already formatted, so they are a no-op unless you
  changed something.
- `Cargo.lock` is committed. It is what makes the nix build cacheable.
- Design docs in `wiki/design/` are living documents — update them in the same
  change that alters behaviour, not afterwards.
- `Connection::stats()` is cumulative *per connection*, not per peer. The peer
  registry accumulates deltas against a per-connection last-seen value; summing
  the raw numbers double-counts on every sample, and one peer can hold several
  connections at once.
- iroh 1.1 exposes neither an enumeration of known remotes nor any liveness
  probe. `Endpoint::remote_info` needs an id you already track and only covers
  recently-used remotes. Do not add an online/offline signal on top of it — see
  kai's `wiki/design/peers.md` for what is actually derivable.
- spirit carries no schema of its consumers'. The mesh counts and pulls what a
  record's `{kind, refs}` envelope declares, and falls back to scanning any
  record for 64-hex blob hashes — that is how a legacy card manifest still
  replicates without spirit knowing what a card is. Never re-add a typed
  manifest here to make replication work.
- Ref names have an owner: `wanted_names` follows only peers trusted at cache
  level or above, and `best_provider` skips the rest. Seeded peers are granted
  cache trust; discovered ones start unknown. Bytes remain fetchable from
  anyone, because the hash verifies them — trust gates the *mapping*, never the
  bytes.
- Gossip has its own ALPN, `spirit-gossip/0`. Do not add membership or
  discovery messages to iroh-blobs' ALPN; read `wiki/design/gossip.md` before
  changing the wire format or the convergence rules — the quiet-down behaviour
  depends on the view version, and it is easy to reintroduce endless chatter.
  Table adverts ride the gossip `View` and obey the same rule: a refresh of an
  unchanged advert must NOT bump the version, or the mesh never goes quiet.
- spirit registers exactly two ALPNs itself: iroh-blobs and
  `spirit-gossip/0`. Every additional protocol comes from the CALLER through
  the `serve_with`/`serve_mesh_with`/`serve_in_memory_with` register
  closures, which receive the `RouterBuilder` before it spawns — that is how
  kai puts agni-net's `spirit-table/1` on the router. One endpoint, one
  identity (`Serving.ticket` is the `EndpointTicket`), all protocols. Do not
  add game-aware protocols back into this crate; spirit must never depend on
  agni.
- `spirit-node` must stay free of HTTP-client and TLS deps: kai links it on
  android. `Mesh::set_backfill` stays a callback hook for exactly this
  reason — the mesh owns *when* to backfill, a caller may supply the
  mechanism, and no ingester is linked here (the Scryfall one lives in
  `agni-importers`). Check with `cargo tree` if you touch those
  dependencies. `gateway.rs` respects this by hand-rolling its HTTP/1.1
  responses over plain tokio TCP — do not "clean it up" by introducing hyper,
  axum, or any TLS stack; localhost-only plus a reverse proxy is the design.
- After changing the CLI, rebuild the binary before testing. `clippy` and
  `fmt` do not emit one, so `target/debug/spirit-node` silently stays stale and
  you will be testing the old argument parser.
