# SDK boundary

Audience: Rust application developers choosing dependencies.
Read the [integration guide](../../../wiki/api/overview.md) first.

`spirit-sdk` currently re-exports the protocol crates. It does not provide
the earlier proposed all-in-one node facade or automatically execute content.

Use `spirit-client` for application-oriented operations. Use the re-exported
core/index/routing/schema APIs when you need direct control of records and
selection. Use the node's embedding hooks only when your process owns the store.

An API wrapper must preserve explicit decisions about trust, publication,
network ownership, and execution. Convenience is not a reason to open the
same store twice or silently trust a discovered signer.

Test consumer examples against their pinned revision. Exact signatures belong
to that revision's source and tests; historical mock signatures are not an API
stability promise.
