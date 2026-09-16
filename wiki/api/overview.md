# Integrating Spirit into an application

Audience: application developers who know Rust but have not used Spirit.
Read the [architecture overview](../design/architecture.md) for the record
vocabulary.

## Choose an ownership model first

If several applications need one store, let one daemon own it and communicate
through the local API. If one application owns the store for its lifetime, it
can embed the node. Do not start two endpoint owners on the same directory.

The local API carries the gateway's request/response shapes over a
length-framed CBOR socket. See [specification section 9](../spec.md#9-the-daemon)
for the transport contract.

## Choose the library boundary

- `spirit-client` supplies application-oriented operations. Start with its
  public types and tests when building a client.
- `spirit-sdk` re-exports the protocol crates; it is not a high-level
  `Node::open(...)` facade.
- `spirit-node` supports embedded service ownership and protocol registration.
- `spirit-client-ffi` exposes the foreign-language boundary used by the
  experimental [Kotlin bindings](../../kotlin/README.md).

Use the source and tests in your pinned revision for exact signatures.
The old [mock API](mock-api.md) is a proposal, not compilable sample code.

## Read content by identity

Build the index from the store's followed collections. Ask for attestations
about the identity. Pass candidates through the routing policy and local
trust. Fetch the chosen blob and verify its hash before using it.

Decide how your application handles missing, untrusted, and expired claims.
Do not treat all three as the same network error, and do not execute fetched
bytes merely because their hash is valid.

## Publish content

Create identity and transform records, store the content bytes, and sign a
content attestation with the store's group identity. Include the records and
attestation in a collection, then publish through the collection/ref APIs.

A record omitted from the declared replication closure may remain available
only on the original device. Direct filesystem writes to `refs/` bypass the
supported update path.

## Supply capabilities explicitly

Spirit describes transforms but does not supply a general runtime. An
application provides fetch and execution hooks if it needs them. Similarly,
extra network protocols are registered by the embedding application rather
than hard-coded into Spirit.

Keep application-specific schemas in your application. Never place game or
media knowledge in the generic replication layer to fix a missing record.

## Verify your integration

Use temporary stores and synthetic content. Test a missing blob, an invalid
signature, an untrusted signer, and two devices making collection edits.
If embedding networking, also test clean shutdown and refusal of a second
owner of the same store.

See the [development guide](../development.md) for workspace commands and the
[specification](../spec.md) before changing an on-disk or wire format.
