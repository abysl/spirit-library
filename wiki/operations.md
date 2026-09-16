# Running a Spirit node

Audience: people operating their own Spirit store. Assumes basic terminal and
filesystem knowledge, but no knowledge of Spirit's network protocol.
Complete the [local-store tutorial](getting-started.md) first.

## Start the background service

From a built checkout:

```sh
./target/debug/spirit-node mesh ./my-store --gateway 8787
```

This starts a foreground process. It owns the store and serves an HTTP interface
on `127.0.0.1:8787`. Stop it with Ctrl-C. For a permanent installation, use
your operating system's process manager and a persistent store path.

Only one process may own a store's network endpoint at a time. Do not run two
embedded clients or daemons against the same store.

## Understand the exposure

The gateway's reads are available to anyone who can reach its listening port.
Writes require the bearer token stored in `gateway-token` inside the store.
Treat that token as a password. The local socket uses filesystem permissions.

Do not expose the gateway publicly without your own access controls. Transport
authentication, content hashes, and a write token do not make all stored
content private. Do not put sensitive files in a networked store on the
assumption that hashing encrypts them.

The daemon may use relay/discovery services to reach peers. A successful local
store operation does not prove that another network can reach your node.

## Pair devices you control

With the first device's service running:

```sh
./target/debug/spirit-node --store ./my-store pair
```

On a second device, use a separate store and the actual link the first command
printed:

```sh
./target/debug/spirit-node --store ./second-store join 'PAIRING_LINK'
./target/debug/spirit-node mesh ./second-store --gateway 8788
```

A pairing link is a short-lived, single-use invitation. Do not post it in an
issue or log. Pairing gives the new device the group's signing authority;
only pair devices you control and trust.

Members of a group follow each other's content. Pairing is not a read-only
guest invitation. Inspect membership with `members`; do not assume removing
a member erases previously shared files or secrets from that device.

## Backups and troubleshooting

Back up the complete store, including identity and collection metadata, while
it is stopped or through a consistent filesystem snapshot. Replication is not
a substitute for a backup policy. Protect backups as carefully as live keys.

If a second process refuses to open the store, find the process that owns it
before touching a lock file. If peers connect but content is missing, inspect
the refs and local trust policy: possessing a blob and following a collection
are different things.

Rebuild after editing the CLI; formatting and linting do not update the binary.
Use `spirit-node help` for the arguments supported by your checkout.
