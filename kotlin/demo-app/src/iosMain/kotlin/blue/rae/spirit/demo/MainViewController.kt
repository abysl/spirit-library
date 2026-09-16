package blue.rae.spirit.demo

import androidx.compose.ui.window.ComposeUIViewController
import platform.Foundation.NSHomeDirectory
import platform.UIKit.UIViewController

/**
 * The standard Compose Multiplatform iOS entry point: an Xcode project's
 * `SceneDelegate`/`App.swift` calls this to get the root view controller.
 * No Xcode project is included here — add this framework
 * (`:demo-app` built as `iosMain`) to one and call `MainViewController()`
 * from Swift.
 */
fun MainViewController(): UIViewController = ComposeUIViewController {
    App(storeDir = NSHomeDirectory() + "/Documents/spirit-store")
}
