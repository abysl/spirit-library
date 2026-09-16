package blue.rae.spirit.client

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonObject
import uniffi.spirit_client_ffi.FfiException
import uniffi.spirit_client_ffi.FfiPairingOffer
import uniffi.spirit_client_ffi.FfiPeer
import uniffi.spirit_client_ffi.FfiResolved
import uniffi.spirit_client_ffi.SpiritClient

/**
 * The idiomatic Kotlin surface over `spirit-client-ffi`'s generated
 * [SpiritClient]. Every one of `SpiritClient`'s methods is a blocking JNI
 * call — this wrapper's whole job is moving that off the caller's thread
 * (via [dispatcher], [Dispatchers.IO] by default) and turning spirit's
 * `ci:`/`td:`/`att:`/`blob:`-prefixed strings and [FfiException] into
 * small Kotlin types, so app code never touches the generated bindings
 * directly.
 *
 * Mirrors `spirit-client`'s Rust API 1:1 — see `../../../client/src/lib.rs`
 * for what each method actually does.
 */
class SpiritStore private constructor(
    private val client: SpiritClient,
    private val dispatcher: CoroutineDispatcher,
) {
    companion object {
        /** Start (or reuse) a daemon embedded in this process. */
        suspend fun openEmbedded(
            storeDir: String,
            seeds: List<String> = emptyList(),
            dispatcher: CoroutineDispatcher = Dispatchers.IO,
        ): SpiritStore = withContext(dispatcher) {
            SpiritStore(SpiritClient.openEmbedded(storeDir, seeds), dispatcher)
        }

        /** Data only — no daemon, no network. */
        suspend fun openLocal(
            storeDir: String,
            dispatcher: CoroutineDispatcher = Dispatchers.IO,
        ): SpiritStore = withContext(dispatcher) {
            SpiritStore(SpiritClient.openLocal(storeDir), dispatcher)
        }
    }

    private suspend fun <T> io(block: () -> T): T = withContext(dispatcher) { block() }

    suspend fun shutdown() = io { client.shutdown() }

    suspend fun dgid(): String = io { client.dgid() }
    suspend fun nodeId(): String? = io { client.nodeId() }

    /** This daemon's dialing ticket — what you'd render as a QR code. */
    suspend fun ticket(): String? = io { client.ticket() }

    // ---- data ----

    suspend fun putBlob(bytes: ByteArray): String = io { client.putBlob(bytes) }
    suspend fun getBlob(hash: String): ByteArray = io { client.getBlob(hash) }

    suspend fun mintCir(kind: String, body: JsonObject): String =
        io { client.mintCir(kind, body.toString()) }

    suspend fun mintTdr(kind: String, body: JsonObject): String =
        io { client.mintTdr(kind, body.toString()) }

    suspend fun attestContent(ci: String, td: String, blob: String): String =
        io { client.attestContent(ci, td, blob) }

    /**
     * Mint a content attestation and add the item to [collectionName], in
     * one call. Reach for this over [attestContent] whenever the goal is
     * "share this" — an attestation on its own is not discoverable; a
     * collection only replicates and indexes what it explicitly carries.
     */
    suspend fun share(collectionName: String, ci: String, td: String, blob: String, label: String? = null): String =
        io { client.share(collectionName, ci, td, blob, label) }

    suspend fun collectionAdd(name: String, ci: String, label: String? = null): String =
        io { client.collectionAdd(name, ci, label) }

    suspend fun collectionShowJson(name: String): String = io { client.collectionShow(name) }

    suspend fun resolve(ci: String, minimum: TrustLevel = TrustLevel.CACHE): FfiResolved? =
        io { client.resolve(ci, minimum.wire) }

    suspend fun resolveBytes(ci: String, minimum: TrustLevel = TrustLevel.CACHE): ByteArray? =
        io { client.resolveBytes(ci, minimum.wire) }

    suspend fun indexJson(): String = io { client.indexJson() }

    suspend fun setTrust(who: String, level: TrustLevel) = io { client.setTrust(who, level.wire) }

    // ---- network ----

    suspend fun addPeer(seed: String): String = io { client.addPeer(seed) }
    suspend fun want(collectionName: String) = io { client.want(collectionName) }
    suspend fun peers(): List<FfiPeer> = io { client.peers() }
    suspend fun offerPairing(): FfiPairingOffer = io { client.offerPairing() }
    suspend fun join(pairingUrl: String) = io { client.join(pairingUrl) }
}

enum class TrustLevel(internal val wire: String) {
    UNKNOWN("unknown"),
    CONTACT("contact"),
    CACHE("cache"),
    MESH("mesh"),
}

/** True if this is one of spirit's own [FfiException] variants. */
fun Throwable.asSpiritError(): FfiException? = this as? FfiException
