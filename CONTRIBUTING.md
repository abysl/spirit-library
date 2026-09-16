# Contributing to Spirit Library

Audience: programmers familiar with Git and basic programming, new to Spirit.
You do not need prior peer-to-peer networking experience for a local-store fix.

## Start small

Follow the [local-store tutorial](wiki/getting-started.md), then the
[development guide](wiki/development.md). Read the
[architecture overview](wiki/design/architecture.md) to choose a crate.

Create a branch, add a regression test, and make one focused change. Open a
GitHub pull request with the problem, approach, commands run, and any data or
protocol compatibility effects.

## Keep the layers separate

The core data structures do not need a network. The network service exchanges
records without understanding every application's content. Game, media, and
package-specific schemas belong in applications.

A valid hash verifies bytes; local trust decides which identity-to-file claims
to accept. Do not combine these checks or make trust transitive.

Use the public ref APIs to update collection pointers. Do not write files
directly into a store's `refs/` directory.

## Protocol changes need a specification change

The [specification](wiki/spec.md) defines record names, encoding, addresses,
identity, trust, and replication. Read the relevant sections before changing
these interfaces. For a change that crosses sections, read the full spec.

Existing content addresses must remain valid. New optional fields, compatibility
readers, and format tests are preferable to silently rewriting old data.
Describe migrations explicitly.

## Verification and contribution hygiene

Run treefmt and the [fast checks](wiki/development.md#checks). Also run the
affected crate's tests; networking changes need the relevant integration tests.
A passing local unit test is not evidence that pairing, reconnect, or browser
behavior works.

Never commit identity keys, populated stores, pairing links, credentials, or
internal deployment configuration. Use temporary stores and synthetic content
in tests.

Code uses small functions and precise names without comment lines. Put usage
and design explanations in the wiki. [AGENTS.md](AGENTS.md) lists additional
implementation constraints.
