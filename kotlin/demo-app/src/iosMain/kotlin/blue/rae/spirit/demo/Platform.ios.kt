package blue.rae.spirit.demo

// This file is the least-verified in the whole tree: real UIKit/AVFoundation/
// CoreImage Kotlin/Native cinterop, written without a Mac or Xcode available
// to compile it. The shape is right — these are the standard native APIs
// for the job, all auto-bridged by Kotlin/Native with no custom .def file
// needed — but expect to spend real time here getting delegate/lifetime
// details exactly right on a machine that can actually build it.

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.coroutines.CompletableDeferred
import org.jetbrains.skia.Image
import platform.AVFoundation.*
import platform.CoreImage.CIFilter
import platform.CoreImage.filterWithName
import platform.Foundation.NSData
import platform.Foundation.create
import platform.UIKit.*
import platform.darwin.NSObject

@OptIn(ExperimentalForeignApi::class)
private fun currentViewController(): UIViewController? =
    UIApplication.sharedApplication.keyWindow?.rootViewController

@Composable
actual fun rememberFilePicker(): FilePicker = FilePicker {
    val deferred = CompletableDeferred<PickedFile?>()
    val presenter = currentViewController() ?: return@FilePicker null
    val picker = UIDocumentPickerViewController(
        forOpeningContentTypes = listOf(platform.UniformTypeIdentifiers.UTTypeItem),
    )
    val delegate = object : NSObject(), UIDocumentPickerDelegateProtocol {
        override fun documentPicker(controller: UIDocumentPickerViewController, didPickDocumentsAtURLs: List<*>) {
            val url = didPickDocumentsAtURLs.firstOrNull() as? platform.Foundation.NSURL
            val data = url?.let { platform.Foundation.NSData.dataWithContentsOfURL(it) }
            val bytes = data?.toByteArray()
            deferred.complete(bytes?.let { PickedFile(url.lastPathComponent ?: "file", it) })
        }

        override fun documentPickerWasCancelled(controller: UIDocumentPickerViewController) {
            deferred.complete(null)
        }
    }
    picker.delegate = delegate
    presenter.presentViewController(picker, animated = true, completion = null)
    deferred.await()
}

@OptIn(ExperimentalForeignApi::class)
private fun NSData.toByteArray(): ByteArray {
    val length = this.length.toInt()
    val bytes = ByteArray(length)
    if (length > 0) {
        bytes.usePinned { pinned ->
            platform.posix.memcpy(pinned.addressOf(0), this.bytes, this.length)
        }
    }
    return bytes
}

actual fun renderQrCodeBitmap(text: String): ImageBitmap {
    val filter = CIFilter.filterWithName("CIQRCodeGenerator")!!
    filter.setValue(NSData.create(text.encodeToByteArray()), forKey = "inputMessage")
    val output = filter.outputImage!!
    val context = platform.CoreImage.CIContext()
    val cgImage = context.createCGImage(output, fromRect = output.extent)
    val uiImage = UIImage.imageWithCGImage(cgImage)
    // Bridge UIKit -> Skia (what Compose Multiplatform actually renders
    // with on iOS) via PNG bytes, rather than fighting CGImage/Skia interop
    // directly.
    val png = UIImagePNGRepresentation(uiImage) ?: error("could not encode the QR code as PNG")
    return Image.makeFromEncoded(png.toByteArray()).toComposeImageBitmap()
}

@Composable
actual fun rememberQrScanner(): QrScanner = QrScanner {
    val deferred = CompletableDeferred<String?>()
    val presenter = currentViewController() ?: return@QrScanner null
    val session = AVCaptureSession()
    val device = AVCaptureDevice.defaultDeviceWithMediaType(AVMediaTypeVideo) ?: return@QrScanner null
    val input = AVCaptureDeviceInput.deviceInputWithDevice(device, null) ?: return@QrScanner null
    session.addInput(input)
    val output = AVCaptureMetadataOutput()
    session.addOutput(output)

    val scanner = UIViewController()
    val previewLayer = AVCaptureVideoPreviewLayer(session = session)
    previewLayer.frame = scanner.view.bounds
    scanner.view.layer.addSublayer(previewLayer)

    val delegate = object : NSObject(), AVCaptureMetadataOutputObjectsDelegateProtocol {
        override fun captureOutput(
            output: AVCaptureOutput,
            didOutputMetadataObjects: List<*>,
            fromConnection: AVCaptureConnection,
        ) {
            val code = didOutputMetadataObjects
                .filterIsInstance<AVMetadataMachineReadableCodeObject>()
                .firstOrNull { it.type == AVMetadataObjectTypeQRCode }
                ?.stringValue
            if (code != null && !deferred.isCompleted) {
                deferred.complete(code)
                session.stopRunning()
                scanner.dismissViewControllerAnimated(true, completion = null)
            }
        }
    }
    output.setMetadataObjectsDelegate(delegate, queue = platform.darwin.dispatch_get_main_queue())
    output.metadataObjectTypes = listOf(AVMetadataObjectTypeQRCode)

    presenter.presentViewController(scanner, animated = true) { session.startRunning() }
    val result = deferred.await()
    if (session.isRunning()) session.stopRunning()
    result
}
