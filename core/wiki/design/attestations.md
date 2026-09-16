# Signed claims

Audience: maintainers of trust and resolution. Read the
[vocabulary](../../../wiki/design/terminology.md) first.

An attestation separates a claim from its proof. A content claim connects a
content identity, transform definition, and blob. Relation claims connect
identities for purposes such as correction or version lineage.

Verify the signature over the claim's defined signing scope. Then apply local
trust, ownership, and expiry rules. Successfully decoding a proof is not the
same as verifying it; verifying it is not the same as trusting its signer.

Invalid or untrusted records can exist in storage without becoming eligible
resolution candidates. Keep storage, indexing, and eligibility distinct.

The implemented proof model is group signing. Do not describe deferred
threshold, reproducibility, or zero-knowledge proofs as available options.

See [specification section 5.4](../../../wiki/spec.md#54-attestation-record-built)
for the record shape and [section 7](../../../wiki/spec.md#7-naming-index-and-resolution)
for selection. Test bad signatures, wrong owners, expiry, and competing claims.
