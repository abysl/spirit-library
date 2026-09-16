# Developing Spirit Library

Audience: programmers who know basic Git and terminal use but have not worked
on this project. Commands below start at the repository root after checkout.

## Requirements and checkout

Install Git, Rust 1.98.1, and a C compiler/native build tools for your platform.
If using rustup, select 1.98.1. Then:

```sh
git clone https://github.com/abysl/spirit-library.git
cd spirit-library
cargo test --locked -p spirit-core --lib
cargo build --locked -p spirit-node
```

Cargo downloads the versions in Cargo.lock. A `devenv shell` is available as
an optional development environment.

Try the [local-store tutorial](getting-started.md) before changing behavior
visible to users. The [architecture guide](design/architecture.md) maps the
workspace's crates.

## Checks

```sh
bash ci/check.sh
```

This executes the core, index, routing, schema, and SDK library tests, then
compile-checks all workspace targets, including the client and language-binding
boundary. It does not execute network integration tests or Kotlin tests.

For a broader run:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Use temporary stores in tests. Networking tests may need local sockets and
network access, and should never use your personal store or credentials.

## Browser and language bindings

The node's default `native` feature includes the filesystem-backed service
and CLI. A browser embedding disables default features and uses an in-memory
store. Changes to shared code must not accidentally require native-only APIs.

The [Kotlin guide](../kotlin/README.md) is for binding contributors and has a
separate toolchain. Passing Rust checks is not proof that Android or iOS builds.

## Changing records or protocols

Read the relevant [specification](spec.md) sections and add compatibility tests.
Keep existing addresses valid. If an optional field is absent, preserve the
existing encoding rather than filling it with a new null/default value.

## Formatting

treefmt runs the configured language formatters for this repository. With Nix
installed, the pinned environment supplies treefmt, rustfmt, taplo, and alejandra:

```sh
nix-shell ci/format.nix --run treefmt
nix-shell ci/format.nix --run 'treefmt --ci'
```

The first command applies formatting; the second fails if formatting changes
are needed. You can also install those tools yourself and run `treefmt`
directly. Rust, TOML, and Nix are covered; prose is reviewed for clarity.

## What GitHub checks

The `PR checks` workflow runs on pull requests targeting `main`, pushes to
`main`, and merge-queue events. Its `treefmt` and `fast-check` jobs feed the
single `pr-gate` result.

Rust dependency/build caches are reused; only pushes to `main` save shared
caches. A cold run still needs to fetch and compile dependencies.
The workflow uses read-only repository permissions and does not publish
packages or deploy applications.

Repository administrators must require `pr-gate` in the protection rule or
ruleset for `main` to prevent merging a failed check. A workflow file alone
does not enforce that rule.

## Common failures

If `--locked` refuses to proceed, a manifest and Cargo.lock disagree. Update
the lockfile intentionally, inspect the dependency changes, and commit it.
Do not remove `--locked` from CI to hide the mismatch.

A missing formatter means its executable is not on PATH; use the pinned Nix
environment. A native linker/pkg-config error usually means a required system
library or build tool is missing, not that a Rust test failed.

Run commands from the repository root unless a guide explicitly says otherwise.
