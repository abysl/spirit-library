# Earlier API proposal

Audience: maintainers researching API design history.

The earlier version of this page sketched an all-in-one node facade, federated
queries, collection suggestions, and multiple proof types. It was pseudocode,
not an implemented interface. It must not be used as an integration tutorial.

For current integration choices, use the [integration guide](overview.md).
For exact Rust signatures, inspect `client/`, `sdk/`, and their tests in
the revision your application uses. For record and protocol semantics, use
the [specification](../spec.md).

Any future convenience API should preserve the distinction between storing
bytes, accepting a trusted claim, following a collection, and executing a
caller-supplied capability. A short method name should not hide those decisions.
