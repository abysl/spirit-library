# Store and retrieve your first file

Audience: developers and technical users comfortable with a terminal, new to
Spirit. This tutorial uses only your machine; it does not start a network service.

## Build the tool

From the repository root, with Rust and Git installed:

```sh
cargo build --locked -p spirit-node
./target/debug/spirit-node help
```

The executable is `target/debug/spirit-node`. Run the commands below from the
repository root. Building may download Rust dependencies.

## Use a separate test store

A store is a directory containing content, metadata, and identity keys. Use a
new directory so the tutorial does not change an existing library:

```sh
export SPIRIT_STORE="$(mktemp -d)"
./target/debug/spirit-node identity
./target/debug/spirit-node blob put README.md
```

The last command prints the hash of the stored bytes. Copy that value, then
replace `HASH` below with it:

```sh
./target/debug/spirit-node blob has HASH
./target/debug/spirit-node blob get HASH --out retrieved-readme.md
cmp README.md retrieved-readme.md
```

A successful `cmp` exits without output. You have stored and retrieved a
**blob**: immutable bytes addressed by their hash.

Changing README.md later creates different bytes and therefore a different
hash. It does not update the blob you already stored.

## What this does not do

Putting a blob does not automatically publish a collection or make a claim
about the file's meaning. Applications add records and signed attestations
for those purposes; see the [integration guide](api/overview.md).

The temporary store persists until you remove it. Keep its identity keys
private. Close any process using it before moving or deleting it.

Next, read the [operator guide](operations.md) if you want to run a service or
pair devices. Read [development](development.md) if you want to change the code.
