# Devices, groups, and trust

Audience: contributors changing identity or pairing. Read the
[operator guide](../../../wiki/operations.md) before working on the protocol.

A device key identifies one network endpoint. A group key identifies the
signing authority shared by paired devices. These keys have different
lifetimes and must not be treated as aliases.

A shared-key group gives every member broad signing authority. Pairing is not
a limited-access invitation. One store belongs to one group; multiple groups
require separate stores in the current model.

Membership is represented by a collection of device records. Admission needs
the device's consent and a group-signed operation. The pairing exchange uses
a short-lived, single-use token over an authenticated connection.

Trust is local and non-transitive. Verify membership/vouches before deriving a
device's group trust. Unknown peers do not gain authority merely by being
discovered.

Test token replay/expiry, mismatched endpoint identity, malformed consent,
concurrent membership changes, and startup with an existing identity.
Do not assume revocation erases a previously shared secret from a device.

The exact contract is [specification section 6](../../../wiki/spec.md#6-identity-groups-and-trust).
