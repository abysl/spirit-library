package blue.rae.spirit.client

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.nio.file.Files
import kotlin.io.path.absolutePathString
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

/**
 * Runs the generated uniffi bindings against the real `spirit-client-ffi`
 * shared library (built by `jvm-native`, loaded via the `jna.library.path`
 * this module's `build.gradle.kts` points at the Cargo workspace's
 * `target/release`). This is the sentence "the Kotlin Multiplatform
 * wrapper works" turned into something that actually runs a real Rust
 * daemon from the JVM and checks the answers it gives back.
 */
class SpiritStoreTest {
    private fun tempStore(): String = Files.createTempDirectory("spirit-store-test").absolutePathString()

    // Block bodies on purpose, not `fun f() = runBlocking { ... }`: JUnit4
    // (the runner Kotlin Multiplatform's jvm target uses by default) requires
    // `@Test` methods to return void, and an expression body infers its
    // return type from the last statement inside `runBlocking` — which here
    // is an `assertNotNull`/`assertEquals` call that returns a value, not
    // `Unit`. A block body always returns `Unit` regardless.
    @Test
    fun `minting sharing and resolving round trips through the real daemon`() {
        runBlocking {
            val dir = tempStore()
            val store = SpiritStore.openLocal(dir)
            try {
                val ci = store.mintCir(
                    "song",
                    JsonObject(mapOf("title" to JsonPrimitive("Leaves from the Vine"))),
                )
                val blob = store.putBlob("pretend this is flac bytes".encodeToByteArray())
                val td = store.mintTdr("flac-encode", JsonObject(mapOf("variant" to JsonPrimitive("flac"))))
                store.share("favorites", ci, td, blob, "Leaves from the Vine")

                val resolved = assertNotNull(store.resolve(ci))
                assertEquals(blob, resolved.blob)
                assertEquals(
                    "pretend this is flac bytes",
                    store.resolveBytes(ci)!!.decodeToString(),
                )

                // No daemon at all in Mode.Local — network calls fail cleanly
                // rather than hanging.
                assertNotNull(runCatching { store.addPeer("endpoint...") }.exceptionOrNull())
            } finally {
                store.shutdown()
            }
        }
    }

    @Test
    fun `two embedded daemons pair and seed each other over real iroh endpoints`() {
        runBlocking {
            val a = SpiritStore.openEmbedded(tempStore())
            val b = SpiritStore.openEmbedded(tempStore())
            try {
                assertNotNull(a.nodeId())
                assertNotNull(a.ticket())

                val offer = a.offerPairing()
                b.join(offer.url)
                assertEquals(a.dgid(), b.dgid())

                val bTicket = assertNotNull(b.ticket())
                val seededId = a.addPeer(bTicket)
                assertEquals(b.nodeId(), seededId)
            } finally {
                a.shutdown()
                b.shutdown()
            }
        }
    }
}
