package blue.rae.spirit.demo

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.integration.android.IntentIntegrator
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
actual fun rememberFilePicker(): FilePicker {
    var pending by remember { mutableStateOf<CompletableDeferred<android.net.Uri?>?>(null) }
    val context = LocalContext.current
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        pending?.complete(uri)
        pending = null
    }
    return FilePicker {
        val deferred = CompletableDeferred<android.net.Uri?>()
        pending = deferred
        launcher.launch(arrayOf("*/*"))
        val uri = deferred.await() ?: return@FilePicker null
        withContext(Dispatchers.IO) {
            context.contentResolver.openInputStream(uri)?.use { stream ->
                PickedFile(displayNameOf(context, uri), stream.readBytes())
            }
        }
    }
}

private fun displayNameOf(context: Context, uri: android.net.Uri): String {
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
        val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
        if (column >= 0 && cursor.moveToFirst()) return cursor.getString(column)
    }
    return uri.lastPathSegment ?: "file"
}

actual fun renderQrCodeBitmap(text: String): ImageBitmap {
    val size = 512
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size)
    val bitmap = Bitmap.createBitmap(matrix.width, matrix.height, Bitmap.Config.RGB_565)
    for (x in 0 until matrix.width) {
        for (y in 0 until matrix.height) {
            bitmap.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
        }
    }
    return bitmap.asImageBitmap()
}

@Composable
actual fun rememberQrScanner(): QrScanner {
    var pending by remember { mutableStateOf<CompletableDeferred<String?>?>(null) }
    val activity = LocalContext.current as Activity
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        // IntentIntegrator.createScanIntent()/parseActivityResult are the
        // documented way to drive zxing-android-embedded through your own
        // ActivityResult launcher instead of its own onActivityResult path
        // — confirm the exact visibility/name on the pinned library version
        // when this is actually built.
        val scanned = IntentIntegrator.parseActivityResult(result.resultCode, result.data)
        pending?.complete(scanned?.contents)
        pending = null
    }
    return QrScanner {
        val deferred = CompletableDeferred<String?>()
        pending = deferred
        launcher.launch(IntentIntegrator(activity).createScanIntent())
        deferred.await()
    }
}
