package blue.rae.spirit.demo

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.ImageBitmap

/** A file the user picked, read fully into memory — fine for a demo, not
 * for anything that should stream large files. */
data class PickedFile(val name: String, val bytes: ByteArray)

fun interface FilePicker {
    /** Opens the picker and suspends until it closes; `null` if cancelled. */
    suspend fun pick(): PickedFile?
}

/** A platform-appropriate [FilePicker], bound to the current composition —
 * Android's needs to register an activity-result launcher while composing,
 * the same reason [rememberQrScanner] is `@Composable` too. */
@Composable
expect fun rememberFilePicker(): FilePicker

/** Render `text` (a spirit pairing URL or ticket) as a QR code bitmap. */
expect fun renderQrCodeBitmap(text: String): ImageBitmap

/** Reads one QR code and returns its text, or `null` if the user backed
 * out. Desktop has no camera in this demo, so it degrades to a manual
 * paste dialog — see `Platform.desktop.kt`. */
fun interface QrScanner {
    suspend fun scan(): String?
}

/** A platform-appropriate [QrScanner], bound to the current composition
 * (so Android's can hold the activity-result launcher it needs). */
@Composable
expect fun rememberQrScanner(): QrScanner
