# Gateway integration

Audience: developers adding a local client or browser integration.
For running a node, use the [operator guide](../operations.md).

The gateway exposes store operations over HTTP and serves the node's operator
interface. It is not read-only: supported writes require a bearer token.
The detailed contract is [specification section 9](../spec.md#9-the-daemon).

The HTTP listener binds to loopback. Reads are available to whoever can reach
it; a reverse proxy does not automatically make those reads private. Keep
authorization and exposure policy explicit in the integrating application.

The local socket transports the same request/response shapes using framed CBOR.
Filesystem permissions protect the socket. This lets several applications use
one daemon instead of competing to own the same store.

Named resolvers are supplied by callers. Spirit dispatches by name and returns
the result; it does not hard-code game or media schemas. Any resolver fetching
a URL must enforce its own input and network policy.

Gateway URL seeds resolve a peer's current connection information from status.
They are not interchangeable with a pinned content hash. HTTPS fetching is a
caller capability, not a built-in general HTTP client in the daemon.

Test unauthorized writes, malformed requests, unavailable resolvers, and local
socket behavior. Browser cross-origin access and trust policy need separate
tests from the handler's success path.
