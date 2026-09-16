package blue.rae.spirit.demo

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.awt.FileDialog
import java.awt.Frame
import java.awt.image.BufferedImage
import javax.swing.JOptionPane

/**
 * Desktop has no camera in this demo. `pick`/`scan` use plain AWT/Swing
 * dialogs — functional, but blocking calls made straight from a coroutine
 * rather than dispatched onto the Swing event thread properly (via
 * `kotlinx-coroutines-swing`'s `Dispatchers.Swing`, not added here to keep
 * this module's dependency list small). Fine for a demo; worth fixing
 * before this is anything more than that.
 */
@Composable
actual fun rememberFilePicker(): FilePicker = FilePicker {
    withContext(Dispatchers.IO) {
        val dialog = FileDialog(null as Frame?, "Pick a file to share", FileDialog.LOAD)
        dialog.isVisible = true
        val name = dialog.file ?: return@withContext null
        val file = java.io.File(dialog.directory, name)
        PickedFile(name, file.readBytes())
    }
}

@Composable
actual fun rememberQrScanner(): QrScanner = QrScanner {
    withContext(Dispatchers.IO) {
        JOptionPane.showInputDialog(
            null,
            "No camera on desktop in this demo — paste the ticket or pairing link:",
            "Add a peer",
            JOptionPane.PLAIN_MESSAGE,
        )
    }
}

actual fun renderQrCodeBitmap(text: String): ImageBitmap {
    val size = 512
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size)
    val image = BufferedImage(matrix.width, matrix.height, BufferedImage.TYPE_INT_RGB)
    for (x in 0 until matrix.width) {
        for (y in 0 until matrix.height) {
            image.setRGB(x, y, if (matrix[x, y]) 0x000000 else 0xFFFFFF)
        }
    }
    return image.toComposeImageBitmap()
}
