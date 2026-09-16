package blue.rae.spirit.demo

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import java.io.File

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val storeDir = File(filesDir, "spirit-store").absolutePath
        setContent {
            App(storeDir = storeDir)
        }
    }
}
