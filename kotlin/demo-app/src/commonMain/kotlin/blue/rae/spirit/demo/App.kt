package blue.rae.spirit.demo

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import blue.rae.spirit.client.SpiritStore
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import uniffi.spirit_client_ffi.FfiException

/** One file this device has minted and shared. */
private data class SharedItem(val label: String, val ci: String, val blob: String)

private const val SHARED_COLLECTION = "shared"

/**
 * The whole demo: mint a Content Identity for a picked file, share it in a
 * collection, offer this device's ticket as a QR code, and scan another
 * device's ticket to receive whatever it's sharing.
 *
 * `storeDir` is a platform-appropriate app-private directory — see each
 * platform's `Main`/entry point for where it comes from.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun App(storeDir: String) {
    val scope = rememberCoroutineScope()
    var store by remember { mutableStateOf<SpiritStore?>(null) }
    var status by remember { mutableStateOf("opening store…") }
    val items = remember { mutableStateListOf<SharedItem>() }
    var qrText by remember { mutableStateOf<String?>(null) }
    val scanner = rememberQrScanner()
    val filePicker = rememberFilePicker()

    LaunchedEffect(storeDir) {
        try {
            val opened = SpiritStore.openEmbedded(storeDir)
            store = opened
            status = "ready — dgid ${opened.dgid().take(16)}…"
        } catch (e: FfiException) {
            status = "failed to start: ${e.message}"
        }
    }

    MaterialTheme {
        Scaffold(topBar = { TopAppBar(title = { Text("spirit demo") }) }) { padding ->
            Column(
                modifier = Modifier.padding(padding).padding(16.dp).fillMaxSize(),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(status, style = MaterialTheme.typography.bodyMedium)

                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = {
                        val current = store ?: return@Button
                        scope.launch {
                            try {
                                val file = filePicker.pick() ?: return@launch
                                status = "minting ${file.name}…"
                                val ci = current.mintCir(
                                    "file",
                                    JsonObject(mapOf("name" to JsonPrimitive(file.name))),
                                )
                                val blob = current.putBlob(file.bytes)
                                val td = current.mintTdr("raw", JsonObject(emptyMap()))
                                current.share(SHARED_COLLECTION, ci, td, blob, file.name)
                                items.add(0, SharedItem(file.name, ci, blob))
                                status = "shared ${file.name} — ci:${ci.take(12)}…"
                            } catch (e: FfiException) {
                                status = "couldn't add that file: ${e.message}"
                            }
                        }
                    }) { Text("Add file") }

                    Button(onClick = {
                        val current = store ?: return@Button
                        scope.launch {
                            qrText = current.ticket()
                            if (qrText == null) status = "no ticket yet — still coming online?"
                        }
                    }) { Text("Show my QR") }

                    Button(onClick = {
                        val current = store ?: return@Button
                        scope.launch {
                            val scanned = scanner.scan() ?: return@launch
                            status = "adding peer…"
                            try {
                                current.addPeer(scanned)
                                current.want(SHARED_COLLECTION)
                                status = "peer added — pulling \"$SHARED_COLLECTION\"…"
                            } catch (e: FfiException) {
                                status = "couldn't add that peer: ${e.message}"
                            }
                        }
                    }) { Text("Scan to receive") }
                }

                qrText?.let { text ->
                    Card(modifier = Modifier.padding(top = 8.dp)) {
                        Column(
                            modifier = Modifier.padding(16.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Image(bitmap = renderQrCodeBitmap(text), contentDescription = "this device's ticket")
                            Text("scan this on another device", style = MaterialTheme.typography.labelSmall)
                            TextButton(onClick = { qrText = null }) { Text("close") }
                        }
                    }
                }

                Text("Shared files", style = MaterialTheme.typography.titleMedium)
                LazyColumn(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    items(items) { item ->
                        Card {
                            Column(Modifier.padding(12.dp)) {
                                Text(item.label, style = MaterialTheme.typography.bodyLarge)
                                Text(item.ci, style = MaterialTheme.typography.labelSmall)
                            }
                        }
                    }
                }
            }
        }
    }
}
