# The HTTP gateway — a bridge for iroh-less clients

> **Superseded.** [spec.md](../spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

`spirit-node` can serve a small read-only HTTP API alongside its iroh router.
It exists for exactly one consumer today: kai's wasm build, where iroh does not
compile. A browser client fetches refs, manifests and blobs over plain HTTPS
from a native node that IS in the mesh, and so sees the live iroh world instead
of a compile-time bundle. When iroh works on wasm32 the gateway becomes
redundant and can be dropped.

## Enabling it

The gateway is off by default. Turn it on with the `--gateway <port>` flag on
`mesh`, or the `SPIRIT_GATEWAY=<port>` environment variable on `serve` or
`mesh` (the flag wins). It binds `127.0.0.1` only — exposing it beyond
localhost is a reverse proxy's job (the dev1/dev2 deployment uses
`tailscale serve`, see `orgs/andrea/infra/docs/dev-vms.md`).

    spirit-node mesh ~/.spirit/store --gateway 8090

## API

All responses carry `Access-Control-Allow-Origin: *` — everything served is
public content-addressed data, so CORS is wide open on purpose. GET only.

| Route | Returns |
|---|---|
| `GET /gateway/status` | `{node_id, ticket, dgid, peers, refs}` — the health check; `ticket` is the node's current `EndpointTicket` (id plus dial addresses) and `dgid` its device-group id, both null on a gateway without a mesh; `refs` is the same array `/gateway/refs` returns |
| `GET /gateway/refs` | `[{name, manifest, total, held, complete}]` for every local ref — card sets and `modules/<name>` module refs alike |
| `GET /gateway/ref/{name}/manifest` | a card-set ref's manifest re-encoded as JSON: `{set, cards: [{name, image}]}` where `image` is a blob hash; module manifests are not re-encoded — clients fetch them as raw CBOR via `/gateway/blob/{manifest hash}` from the refs listing |
| `GET /gateway/blob/{hash}` | raw blob bytes; content-addressed, so served with `Cache-Control: … immutable` — everything else is `no-store` |
| `GET /gateway/resolvers` | `["name", …]` — the resolvers this gateway has registered |
| `GET /gateway/resolve/{name}?…` | whatever the named resolver returns (below); 404 when no resolver of that name is registered |

Ref names with path separators or `..` are rejected before touching the
filesystem. Unknown paths 404, non-GET methods 405, OPTIONS preflight 204.

## Resolvers

The browser peer cannot fetch third-party sites (CORS), so the gateway can run
lookups server-side on its behalf. That mechanism is deliberately anonymous:
spirit knows a resolver has a *name* and takes *parameters*, and nothing else.

`Gateway.resolvers` is a `BTreeMap<String, Resolver>`, where a `Resolver` is
`Arc<dyn Fn(&ResolveRequest) -> ResolveReply + Send + Sync>`. A request to
`/gateway/resolve/{name}` looks the name up, percent-decodes the query string
into `ResolveRequest { name, params }`, and hands it over. A `ResolveReply`
carries a status, a content type and a body, all of which pass through
untouched. Unregistered names 404; the resolver decides everything else.

This follows the same inversion as `Mesh::set_backfill` and the `serve_with`
protocol closures. The `spirit-node` binary registers nothing, so a bare node
404s every resolve route.

Application knowledge lives above spirit. `agni-importers` registers the
Riftbound deck resolver under the name `deck` — see
`riftbound::gateway::deck_resolver(store_dir)` and its binary `deck-gateway`,
which serves `/gateway/resolve/deck?url=|code=|text=`. The deck-site hostname
allowlist (piltoverarchive.com, riftdecks.com, riftmana.com) moved there with
it: it used to sit in this crate as `DECK_SITE_ALLOWLIST`, which put three
Riftbound URLs inside a library whose own rules say it carries no game
knowledge. Fetch policy travels with the thing that knows what a deck is, so
off-allowlist URLs are now refused by agni with 403 before any fetch, and a
missing parameter is agni's 400.

Whole-route handling runs under `spawn_blocking`, so a slow upstream fetch in
a resolver never stalls the accept loop.

## Seed files

`mesh` also accepts `--seed-file <path>`: one seed per line (endpoint ticket,
blob ticket, or bare node id — the same shapes `--seed` takes), read once at
startup. Blank lines are skipped and unparseable lines are skipped with a
warning rather than failing the whole service, so a config file managed by
hand or by a deploy can't brick the node. This is how the dev1/dev2 systemd
services take their mesh membership: append a ticket to
`/var/lib/spirit/seeds` and restart the unit.

## Seeding by gateway URL

`/gateway/status` is also how a native client finds the mesh without baking
in node ids that rot. `Mesh::seed("https://dev1.example.net")` fetches the
status, takes its `ticket` (falling back to `node_id` for an older gateway
that has no ticket field) and seeds that, so a gateway whose device key
changed — as every one did in the device-key split — is still found by the
same URL. The seed forms are listed in the README; the fetch itself is plain
HTTP/1.1 over TCP for `http://` URLs and a caller-installed `Mesh::set_fetch`
hook for `https://`, because this crate carries no TLS.

## Trust model

The gateway serves whatever the node's store holds. A browser client cannot
verify blake3 hashes cheaply today, so it trusts the gateway the way it would
trust any origin server. The kai client hardcoded each gateway's expected
`node_id` next to its URL and compared it against `/gateway/status`; with
URL seeds the id is whatever the status says, and the URL (behind tailscale)
is the thing being trusted. That is attribution, not verification — real
verification arrives when the client can speak iroh itself.
