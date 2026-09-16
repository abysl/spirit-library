# The local index

Audience: contributors adding queries or debugging missing results.
Read [architecture](../../../wiki/design/architecture.md) first.

The index is derived from followed collection heads and available records.
It accelerates lookups; it is not the authoritative storage format.

Queries connect identities to attestations, external identifiers, record
kinds, and structural links. A missing result can mean an unfollowed ref,
a missing record, or a record not yet included in the local fold—not only
a failed network request.

Do not write authoritative state only into an index. Publishing must store the
records and update the collection/ref path so another device can rebuild the
same information.

Test rebuilding from an empty index, a ref change, incomplete closures,
duplicates, and unknown application kinds. Keep indexing separate from the
routing policy that decides which candidate to use.

See [specification section 7](../../../wiki/spec.md#7-naming-index-and-resolution).
