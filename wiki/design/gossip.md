# Peer discovery and replication

Audience: networking contributors familiar with the
[architecture](architecture.md) and [vocabulary](terminology.md).
The [specification](../spec.md) defines the wire contract.

Gossip exchanges peer addresses and content advertisements. An advertisement
says what a peer claims to hold; it is not the content itself and does not
override local trust.

A node follows wanted refs, selects providers, fetches missing records/blobs,
and verifies bytes. Collection heads for the same owner/name merge by the
defined operation-set rules rather than by whichever reply arrived last.

View versions should change only when the node learns new information.
Refreshing an unchanged advertisement must not bump the version or a settled
network never becomes quiet. Bound concurrent dials and handle repeatedly
failing peers without starving other work.

Application presence can ride generic advertisements. Keep game-session
messages in the caller's protocol rather than adding them to blob transfer.
The caller may also supply backfill behavior; Spirit does not embed the
application's importer.

Test duplicate messages, unchanged refreshes, concurrent collection heads,
untrusted advertisements, stale presence, and failed providers. Check both
convergence and the absence of unnecessary traffic after convergence.
