# spirit — SDK

> **Superseded.** [spec.md](../../../wiki/spec.md) is the source of truth for spirit's design; where this document disagrees with it, the spec wins. This page is kept for its rationale and prior-art discussion.

## What the SDK Is

`spirit-sdk` is the app-facing layer. It provides the conventions and utilities that let multiple independent apps — a music player, a manga reader, a package manager, a filesystem browser — share the same underlying spirit store and interoperate seamlessly.

The protocol layers (core, index, routing, schema) are intentionally unopinionated. The SDK layer is where opinions live: how apps identify themselves, how deep links route to the right app, how a user's DGID serves as their identity across every app they run.

---

## Cross-App Identity

A user's DGID is their identity across the entire spirit ecosystem. Apps do not manage separate accounts. Signing into a music app and a manga app with the same DGID means:

- Both apps share the same device mesh and contact list
- Collections from either app are visible to the other (subject to per-collection sharing settings)
- Adding a contact in one app makes them a contact in all apps

The SDK provides DGID authentication flows so apps don't implement this independently.

---

## App Registration

Apps declare which CI kinds they handle. This lets the ecosystem route `spirit://` deep links to the right app, and lets the filesystem browser know which app to suggest for a given CI.

```toml
[app]
id       = "dgid:..."     # the app's own DGID — stable across versions
name     = "Resonance"
handles  = ["music-track", "album", "playlist"]
```

An app is itself a DGID — it has a stable Ed25519 keypair, publishes a DGID document, and can be added as a contact. "Install this app" and "add this group" are the same operation.

---

## Deep Link Routing

Any spirit-addressable thing can be a deep link:

```
spirit://ci/<blake3-hash>
spirit://collection/<dgid>/<name>
spirit://collection/checkpoint/<blake3-hash>   # sealed checkpoint snapshot
spirit://group/<dgid>                          # add this group / install this app
```

The SDK resolves `spirit://ci/...` to the correct app by looking up the CI's `kind` field and matching it against registered app declarations. If multiple apps handle the same kind, the user's preferred app wins.

---

## Attribution and License Conventions

The SDK defines standard fields that all spirit apps understand, enabling cross-app display and filtering:

```toml
[ci]
kind = "music-track"
# ... content-specific fields ...

[ci.attribution]
creators = [
  { name = "Uncle Iroh", role = "artist", dgid = "dgid:uncle-iroh-official..." },
]
derived_from = "ci:..."    # ci:<hash> of the original, if this is a remix or adaptation

[ci.license]
spdx = "CC-BY-4.0"         # or "All Rights Reserved", etc.
url  = "https://..."       # full license text
```

These fields are defined by the SDK layer, not by spirit-core. The protocol doesn't care what license a CI has. The ecosystem does.

---

## Owned CIs

The SDK formalizes the owned CI concept: a CI with an `owner` field is only attested by that DGID. The trust layer in spirit-core enforces this; the SDK provides the UX and tooling around it.

```toml
[ci]
kind  = "music-track"
owner = "dgid:uncle-iroh-official..."   # only this DGID's attestations are accepted
```

Remixes and adaptations create new CIRs with their own owner and a `derived_from` reference — attribution without identity conflation.

---

## Open Questions

- **App discovery** — how does a user find spirit-compatible apps? Options: a well-known DGID that publishes an app registry CI feed; manual DGID sharing; platform app stores
- **Permission model** — what can one app see of another app's collections? Default: only explicitly shared collections; opt-in to broader visibility
- **License enforcement** — the SDK defines license fields but cannot enforce them; enforcement is a UX and legal concern, not a protocol concern
- **Attribution chain depth** — `derived_from` is a single hop; a remix of a remix needs a chain; decide whether to follow chains automatically or keep it flat
