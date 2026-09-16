# Contributing to Spirit Library

Spirit Library is the generic content mesh substrate. It must not contain
game-specific rules, card definitions, or renderer behavior.

## Development

```text
direnv allow
fmt-check
unit-test
clippy
build
```

The main crates are `core`, `schema`, `index`, `routing`, `sdk`, `node`,
`client`, and `client-ffi`. Read the design documents under `wiki/design/`
before changing identity, canonical encoding, trust, replication, or gateway
behavior.

Content identity and serialized records are compatibility surfaces. Add
round-trip and cross-version tests for changes to their formats. Keep machine
paths, credentials, generated bindings, stores, and hydrated blobs out of Git.

## Pull requests

Use a focused branch, explain compatibility and migration impact, and run the
full workspace checks before opening a pull request.
