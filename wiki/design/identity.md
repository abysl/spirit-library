# Content identity and application schemas

Audience: application developers designing records. Read the
[vocabulary](terminology.md) and [integration guide](../api/overview.md).

Identity records contain identifying fields. Adding or changing those fields
changes the record's hash and therefore its identity. Artwork, mutable labels,
and available encodings should not be inserted into an identity just because
an interface wants to display them.

An attestation connects an identity to particular bytes and a transform
description. A corrected identity is a new record; relations can connect it
to the earlier one without pretending an immutable address changed.

Spirit owns generic kinds needed by its own protocol. Application-specific
schemas, such as a card and its printings, belong to the application.
Agni owns its card schemas; they are not part of Spirit's generic API.

Before defining a kind, decide which fields truly identify the content, which
values are external identifiers, and which links should be indexed.
A breaking schema change needs a new kind/version and a compatibility story.

The exact record and relation contracts are in
[specification section 5](../spec.md#5-records).
