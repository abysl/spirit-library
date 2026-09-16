# Spirit Library

Spirit Library is a Rust toolkit and background service for storing and
sharing content between devices.

Files are addressed by a hash of their bytes instead of by a server location.
Spirit can fetch a file from another device and verify that the received bytes
match the requested hash. Signed records add information about what the content
is and who claims it belongs to a particular identity.

## What is it for?

An application can use Spirit to keep content on several devices without making
one server the permanent home of every file. The project provides:

- A local file store and structured records.
- Signed claims connecting content identities to files.
- Collections of content and local rules for which signers to trust.
- A background service for exchanging content with peers.
- Rust client libraries and experimental Kotlin bindings.

Spirit is under active development. It is not a finished backup product,
media player, package manager, or general-purpose build executor.

## Try the command-line tool

Install a current Rust toolchain and Git, then build the tool:

```sh
git clone https://github.com/abysl/spirit-library.git
cd spirit-library
cargo build --locked -p spirit-node
./target/debug/spirit-node help
```

Use the [local-store tutorial](wiki/getting-started.md) to add and read a file
without joining a network. The [operator guide](wiki/operations.md) explains
running the background service and pairing devices.

No hosted account is required for a local store. Networking has additional
trust and exposure considerations explained in the operator guide.

## A few important distinctions

A hash proves that bytes match an address; it does not prove that the file is
safe, correctly labeled, or lawfully redistributable. Those are separate
questions.

A content identity describes a thing independently of a particular file
format. A signed claim links that identity to particular bytes. Spirit only
uses claims from signers accepted by the local trust policy.

Spirit can describe transformations and accept a runtime supplied by an
application. The command-line service does not include a general build or
transcoding runtime.

## For developers

Start with [Contributing](CONTRIBUTING.md) and the
[development guide](wiki/development.md). The [integration guide](wiki/api/overview.md)
explains the available library boundaries.
The [documentation index](wiki/README.md) separates tutorials from protocol
specifications and historical proposals.

Project code is licensed under [GNU GPL version 3](LICENSE). Stored files and
third-party dependencies retain their own licenses.
