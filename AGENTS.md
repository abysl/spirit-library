# Spirit implementation rules

Audience: coding assistants and contributors who have read
[Contributing](CONTRIBUTING.md) and [development](wiki/development.md).

Read [wiki/spec.md](wiki/spec.md) before using Spirit's terms in code or docs.
It defines the protocol vocabulary and built/partial/planned distinctions.
Other guides explain the spec; they do not supersede it.

## Layer boundaries

Keep consumer schemas out of Spirit. The generic record envelope supplies
replication links; applications own their game/media/package models.
Consumers write collection pointers through `spirit_core::refs`, never by
hand into the refs directory.

One process owns a store's network endpoint. Device and group keys are
separate. Trust is local and non-transitive; it gates accepted claims, not
whether correctly hashed bytes can be transferred.

The SDK re-exports the protocol crates. The application client lives in
`client/`; foreign-language bindings live in `client-ffi/`.

## Networking constraints

Additional application protocols enter through router-registration hooks.
Do not put game protocols into blob transfer or make Spirit depend on a
consumer.

Preserve the no-change/no-version-bump rule in gossip. Repeated identical
advertisements must not cause perpetual traffic.

Connection counters are cumulative per connection. Accumulate deltas;
do not sum the raw total repeatedly or equate a remembered peer with a live one.

Keep native filesystem/service features behind the native feature.
Use the shared runtime abstractions for code compiled to WebAssembly.
Do not add an application HTTP-fetch/TLS stack to the daemon; external
fetch and execution mechanisms are caller-supplied capabilities.

## Work and verification

Code carries no comment lines. Put API explanations and design constraints
in the wiki. Keep functions small and names precise.

Run `bash ci/check.sh`, treefmt, and the relevant additional integration tests.
After editing the CLI, rebuild the binary before testing it: formatting and
linting do not produce an updated executable.

Preserve existing content addresses and test compatibility readers when
changing encoding. Do not change a built/partial/planned marker without
checking the code.

Do not commit identity keys, populated stores, pairing links, credentials,
internal deployment configuration, or generated bindings/build artifacts.
