# Collections and publication

Audience: contributors implementing collection edits or replication.
Read the [vocabulary](../../../wiki/design/terminology.md) first.

A collection is identified by owner and name. Its edits are signed operations,
not in-place mutations of a published list. A head records the known operation
set and the records needed to replicate it.

The fold orders operations by sequence and hash. It ignores operations whose
signer is not the owner. Concurrent edits are kept; every replica uses the same
tie-break rather than relying on wall-clock arrival time.

Heads for the same owner/name merge by operation-set union. A superset can
be adopted without creating another equivalent head. Preserve that behavior
to avoid endless republishing.

The head's declared references define replication closure. Include records
and attestations needed by the collection rather than assuming a remote store
already has them. Move local pointers through the refs API.

A fork changes ownership and does not automatically track the source.
Suggestions, witness ordering, and sealed checkpoints are not current
collection features.

Test concurrent edits, wrong-owner operations, deterministic order, missing
records, and adoption of a known superset. See
[specification section 5.5](../../../wiki/spec.md#55-collections-built).
