import AppKit
import CoreGraphics
import Testing

@Suite("Thumbnail image sizing")
struct ThumbnailImageSizingTests {
  @Test @MainActor func zeroSizeKeepsSquarePortraitLandscapeAndPanoramicDimensions() {
    for (width, height) in [(400, 400), (300, 600), (800, 450), (2400, 120)] {
      let image = makeImage(width: width, height: height)
      let nsImage = NSImage(cgImage: image, size: .zero)

      #expect(nsImage.size == NSSize(width: width, height: height))
    }
  }

  private func makeImage(width: Int, height: Int) -> CGImage {
    let context = CGContext(
      data: nil,
      width: width,
      height: height,
      bitsPerComponent: 8,
      bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    )!
    return context.makeImage()!
  }
}
