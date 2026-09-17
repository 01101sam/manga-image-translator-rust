import SwiftUI
import UIKit

typealias PlatformImage = UIImage

extension Image {
    init(platformImage: UIImage) {
        self.init(uiImage: platformImage)
    }
}
