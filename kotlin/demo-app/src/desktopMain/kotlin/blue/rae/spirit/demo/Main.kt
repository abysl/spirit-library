package blue.rae.spirit.demo

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import java.io.File

fun main() = application {
    val storeDir = File(System.getProperty("user.home"), ".spirit-demo/store").absolutePath
    Window(onCloseRequest = ::exitApplication, title = "spirit demo") {
        App(storeDir = storeDir)
    }
}
