# Spirit vocabulary

Audience: readers of Spirit's developer guides. No protocol background is required.

| Term | Meaning |
|---|---|
| Blob | Immutable bytes addressed by their BLAKE3 hash |
| Record | Deterministically encoded structured data, stored as a blob |
| Content identity record (CIR) | Identifying information about a thing, independent of one encoding |
| Transform definition record (TDR) | A description of how an artifact was or can be produced |
| Attestation | A signed claim, for example linking an identity and transform to bytes |
| Collection | An owner's named set of signed edits, folded into an ordered list |
| Head | A published snapshot of a collection's known records and operations |
| Ref | A local name pointing at a collection head |
| Device group ID (DGID) | The group's signing public key |
| Node ID | One device's network endpoint public key |
| Index | Derived lookups rebuilt from available records |

A content identity is not a blob hash with a different label: it is the
address of a record describing content. Several different blobs may be
attested to the same identity.

A hash verifies bytes. A signature identifies the signer of a claim.
Trust determines whether to use that claim. None of those alone proves that
a file is safe to execute or that its publisher owns redistribution rights.

Typed prefixes such as `ci:`, `td:`, and `blob:` make addresses readable.
See [specification section 3](../spec.md#3-terminology-and-addresses-built)
for exact encodings and validation rules.
