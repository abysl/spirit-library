# Resolution and transform capabilities

Audience: contributors implementing artifact selection.
Read the [integration guide](../../../wiki/api/overview.md) first.

Resolution chooses bytes for a content identity using locally available
attestations and policy. Verify signatures, owner restrictions, minimum trust,
and expiry before ranking eligible candidates.

Ranking considers an explicitly preferred transform, local availability when
requested, trust, snapshot recency, and a deterministic tie-break. The artifact
view explains candidates; it should not hide the distinction between absent,
untrusted, and expired content.

A hash verifies fetched bytes regardless of provider. It does not remove the
need to trust the claim that those bytes represent the requested identity.

Transform definitions describe provenance and inputs. Locking pins inputs;
execution requires a capability supplied by the application. The daemon does
not contain a general-purpose build runtime. Query federation is deferred.

Test policy changes, multiple eligible artifacts, unknown signers, unavailable
bytes, locked inputs, and missing capabilities. See
[specification sections 7 and 10](../../../wiki/spec.md).
