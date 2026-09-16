# Addressing and deterministic encoding

Audience: maintainers changing serialization. Read the
[vocabulary](../../../wiki/design/terminology.md) first.

All blob addresses are BLAKE3 hashes of exact bytes. Typed prefixes distinguish
record roles without changing the underlying hash primitive.

Records use the deterministic CBOR profile implemented by `spirit_core::canonical`.
Field order, integer encoding, omitted optional fields, and rejected value
types affect compatibility. Do not substitute an ordinary serializer because
its output happens to decode to the same map.

A record address covers the entire encoded record. Signing scopes are defined
separately for each claim type. Do not hash a display JSON representation or
include an external proof wrapper in a scope that excludes it.

Test canonical ordering, round trips, malformed input, and old fixtures.
An encoding cleanup that changes previously minted hashes is a breaking
change. The exact contract is in [specification sections 3–5](../../../wiki/spec.md).
