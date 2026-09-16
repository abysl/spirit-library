# Future work

Audience: contributors evaluating proposals, not users looking for available
features. Read the [architecture](architecture.md) first.

Potential future work includes richer group permissions, alternative signing
schemes, persistent index limits, federated queries, and more complete
application/deep-link integration. These are not capabilities implied by the
current command-line tool.

A proposal should identify a real consumer, the existing behavior it cannot
use, and the smallest compatible extension. Include implications for old
content addresses, key handling, replication, and offline behavior.

Do not add a general execution runtime or application-specific schema to the
daemon merely to make a hypothetical workflow convenient. Execution
capabilities belong to callers.

The authoritative deferred list and open decisions are in
[specification sections 14 and 15](../spec.md#14-deferred).
