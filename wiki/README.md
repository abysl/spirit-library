# Documentation guide

Audience: readers choosing a starting point and contributors editing documentation.

Start with the [project introduction](../README.md). You should not need to
know the project's other libraries, development history, or maintainer's
environment to understand it.

## Choose a document

| Document | Intended reader | What it provides |
|---|---|---|
| [Spirit implementation rules](../AGENTS.md) | Coding assistants and implementation contributors | Constraints to preserve while editing |
| [Contributing to Spirit Library](../CONTRIBUTING.md) | New contributors with basic programming knowledge | Prepare, test, and submit a change |
| [Spirit Library](../README.md) | First-time visitors | What the project does, limitations, and where to start |
| [Addressing and deterministic encoding](../core/wiki/design/addressing.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Signed claims](../core/wiki/design/attestations.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Devices, groups, and trust](../core/wiki/design/groups.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [The local index](../index/wiki/design/index.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Kotlin bindings and demo](../kotlin/README.md) | Kotlin/JVM and Android binding contributors | Native library, generator, and platform build constraints |
| [Resolution and transform capabilities](../routing/wiki/design/routing.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Collections and publication](../schema/wiki/design/collections.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [SDK boundary](../sdk/wiki/design/sdk.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Earlier API proposal](api/mock-api.md) | API maintainers | Understand the status of the earlier proposal |
| [Integrating Spirit into an application](api/overview.md) | Rust application developers | Choose an integration boundary and follow read/write paths |
| [Spirit architecture](design/architecture.md) | Developers new to the codebase | Responsibilities, vocabulary, and code navigation |
| [Evaluating Spirit's design](design/comparisons.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Future work](design/future-ideas.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Gateway integration](design/gateway.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Peer discovery and replication](design/gossip.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Content identity and application schemas](design/identity.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Spirit vocabulary](design/terminology.md) | Contributors to this subsystem | Read the stated prerequisites, then use as a focused reference |
| [Developing Spirit Library](development.md) | Programmers new to the project | Install tools, build, test, and troubleshoot |
| [Store and retrieve your first file](getting-started.md) | Terminal users new to Spirit | Complete a local store/retrieve example |
| [Running a Spirit node](operations.md) | People running their own store | Service ownership, pairing, exposure, and backups |
| [spirit — Specification](spec.md) | Protocol implementers and compatibility reviewers | Normative contract; read the overview and vocabulary first |

## Writing for the reader

Introductions explain the problem, capabilities, limitations, and next step.
They expand project-specific names before using them and do not double as
reference manuals.

Contributor guides assume basic programming and Git, not knowledge of this
codebase. Setup instructions name prerequisites, the working directory, the
command, the expected result, and common failures.

Subsystem references can assume the linked introductory material, but should
state that prerequisite. Explain why a boundary exists before listing internal
symbols. Distinguish implemented behavior from a proposal.

Specifications serve compatibility work: preserve precise contracts and
explicit status markers. A prose rewrite must not silently change a protocol.

Machine-readable fixtures are data even when their extension is Markdown.
Do not paraphrase or reflow data files as part of a documentation edit.
Keep credentials, personal paths, and internal deployment details out of
public examples. Use local or example addresses when showing configuration.
