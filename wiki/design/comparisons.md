# Evaluating Spirit's design

Audience: engineers deciding whether Spirit fits an application.
Read the [project introduction](../../README.md) first.

Evaluate the separation between bytes, identity, and signed claims. A byte
address is useful when the exact file is the thing being requested. A content
identity is useful when several artifacts represent one thing and the client
must choose among claims about them.

Spirit's current model uses local, non-transitive trust and shared-key device
groups. It is not a public consensus ledger or a system that discovers global
truth from a majority of peers.

Compare alternatives against your actual requirements: offline operation,
identity stability, artifact selection, access control, key management,
storage limits, and the maturity of client tooling. Do not infer feature
parity with a package manager, backup tool, or media library from a similar
record concept.

The current limitations and deferred features are listed in the
[specification](../spec.md). Benchmark and threat-model a concrete integration
before relying on it for production data.
