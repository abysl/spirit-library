# Spirit architecture

Audience: developers new to Spirit who understand ordinary files, maps, and
function calls. Start with the [local-store tutorial](../getting-started.md).

## Bytes and meaning are separate

The lowest layer stores immutable bytes. Their address is a BLAKE3 hash.
This is a **blob**.

A **record** is structured data encoded deterministically and stored as a blob.
A **content identity record** describes what a thing is. An **attestation**
is a signed claim linking that identity to particular bytes and a description
of how they were produced. A **collection** groups identities and the records
needed to use them.

This separation lets several encodings share one content identity without
claiming that their bytes are identical.

## Read and write paths

To read by identity, build the local index, find candidate attestations,
filter and rank them using local trust, then fetch and verify the chosen blob.

To publish, store the content and its records, sign an attestation, and include
the relevant records in a collection. Publishing moves a local **ref**, a name
pointing to the collection's current head. Replication starts from those refs.

The index is a derived lookup structure. It is not the authoritative copy of
the records.

## Workspace map

| Crate | Responsibility |
|---|---|
| `spirit-core` | Hashes, encoding, blobs, records, collections, identity, trust, refs |
| `spirit-index` | Derived queries over locally available records |
| `spirit-routing` | Candidate selection and transform capability interfaces |
| `spirit-schema` | Spirit-owned record kinds such as devices and modules |
| `spirit-sdk` | Re-exports of the protocol libraries |
| `spirit-node` | Network service, gossip, replication, gateway, and CLI |
| `spirit-client` | Application-facing client operations |
| `spirit-client-ffi` | Foreign-language binding boundary |

Application-specific types belong outside this workspace. Spirit does not need
to know what a card or music album is to replicate its declared records.

## Network ownership and trust

One running process owns a store's endpoint. Applications may embed that service
or use its local API. Do not have several processes open the same store as
independent network owners.

Each device has its own endpoint key. Paired devices share a group signing key.
A group identity and a device identity are not interchangeable.

Trust gates claims about content, not the correctness of a downloaded hash.
Trust is local and non-transitive. Pairing grants broad group authority rather
than limited guest access.

## Design status

The [specification](../spec.md) is the detailed authority for encoding, records,
and wire behavior. Its built/partial/planned markers distinguish implementation
from intent. Older design discussions are not a second API contract.

Continue with [integration](../api/overview.md) or
[development](../development.md).
