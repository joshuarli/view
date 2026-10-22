import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

enum CorpusError: Error, LocalizedError {
  case usage
  case outsideWorkDirectory
  case destinationExists(URL)
  case imageCreation(URL)
  case videoToolFailed(String)

  var errorDescription: String? {
    switch self {
    case .usage:
      "usage: swift scripts/make-corpus.swift .work/corpus [media-item-count]"
    case .outsideWorkDirectory:
      "corpus destination must be beneath the repository's .work directory"
    case .destinationExists(let url):
      "corpus destination already exists: \(url.path)"
    case .imageCreation(let url):
      "could not create PNG: \(url.path)"
    case .videoToolFailed(let message):
      message
    }
  }
}

func createDirectory(_ url: URL) throws {
  try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
}

func createPNG(at url: URL, index: Int) throws {
  try createDirectory(url.deletingLastPathComponent())
  let dimensions = [(400, 300), (300, 400), (500, 500), (1200, 120)][index % 4]
  guard
    let context = CGContext(
      data: nil,
      width: dimensions.0,
      height: dimensions.1,
      bitsPerComponent: 8,
      bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ),
    let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
  else {
    throw CorpusError.imageCreation(url)
  }

  let red = CGFloat((index * 67 + 31) % 256) / 255
  let green = CGFloat((index * 131 + 73) % 256) / 255
  let blue = CGFloat((index * 197 + 149) % 256) / 255
  context.setFillColor(red: red, green: green, blue: blue, alpha: 1)
  context.fill(CGRect(x: 0, y: 0, width: dimensions.0, height: dimensions.1))
  guard let image = context.makeImage() else { throw CorpusError.imageCreation(url) }
  CGImageDestinationAddImage(destination, image, nil)
  guard CGImageDestinationFinalize(destination) else { throw CorpusError.imageCreation(url) }
  _ = image
}

func createVideo(at url: URL, index: Int) throws {
  try createDirectory(url.deletingLastPathComponent())
  let red = (index * 67 + 31) % 256
  let green = (index * 131 + 73) % 256
  let blue = (index * 197 + 149) % 256
  let process = Process()
  process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
  process.arguments = [
    "ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
    "-f", "lavfi", "-i", "color=c=0x\(String(format: "%02x%02x%02x", red, green, blue)):s=320x240:r=24:d=1",
    "-an", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-t", "1", url.path,
  ]
  process.standardOutput = FileHandle.nullDevice
  let errors = Pipe()
  process.standardError = errors
  try process.run()
  let output = errors.fileHandleForReading.readDataToEndOfFile()
  process.waitUntilExit()
  guard process.terminationStatus == 0 else {
    throw CorpusError.videoToolFailed(String(decoding: output, as: UTF8.self))
  }
}

func corpusDestination(_ argument: String) throws -> URL {
  let repository = URL(filePath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .standardizedFileURL
  let work = repository.appending(path: ".work").resolvingSymlinksInPath().standardizedFileURL
  let requested = URL(filePath: argument, relativeTo: repository).standardizedFileURL
  let parent = requested.deletingLastPathComponent().resolvingSymlinksInPath().standardizedFileURL
  let destination = parent.appending(path: requested.lastPathComponent).standardizedFileURL
  guard parent.path == work.path || parent.path.hasPrefix(work.path + "/") else {
    throw CorpusError.outsideWorkDirectory
  }
  guard !FileManager.default.fileExists(atPath: destination.path) else {
    throw CorpusError.destinationExists(destination)
  }
  return destination
}

do {
  guard (2...3).contains(CommandLine.arguments.count) else { throw CorpusError.usage }
  let root = try corpusDestination(CommandLine.arguments[1])
  let itemCount: Int
  if CommandLine.arguments.count == 3 {
    guard let count = Int(CommandLine.arguments[2]), count >= 2 else { throw CorpusError.usage }
    itemCount = count
  } else {
    itemCount = 2_000
  }
  let videoCount = max(1, itemCount / 100)
  let imageCount = itemCount - videoCount
  let originalImageCount = (imageCount + videoCount) / 2
  let copiedImageCount = imageCount - originalImageCount
  try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)

  var originals: [URL] = []
  originals.reserveCapacity(originalImageCount)
  for index in 0..<originalImageCount {
    let folder = String(format: "%02d", index / 100)
    let image = root.appending(path: "images/group-\(folder)/image-\(String(format: "%05d", index)).png")
    try createPNG(at: image, index: index)
    originals.append(image)
  }

  for index in 0..<copiedImageCount {
    let folder = String(format: "%02d", index / 100)
    let copy = root.appending(path: "copies/group-\(folder)/copy-\(String(format: "%05d", index)).png")
    try createDirectory(copy.deletingLastPathComponent())
    try FileManager.default.copyItem(at: originals[index], to: copy)
  }

  let originalVideoCount = min(videoCount, 20)
  var originalVideos: [URL] = []
  originalVideos.reserveCapacity(originalVideoCount)
  for index in 0..<originalVideoCount {
    let video = root.appending(path: "videos/originals/clip-\(String(format: "%03d", index)).mov")
    try createVideo(at: video, index: index)
    originalVideos.append(video)
  }
  for index in 0..<(videoCount - originalVideoCount) {
    let copy = root.appending(path: "videos/copies/clip-\(String(format: "%03d", index)).mov")
    try createDirectory(copy.deletingLastPathComponent())
    try FileManager.default.copyItem(at: originalVideos[index % originalVideos.count], to: copy)
  }

  let noise = root.appending(path: "noise")
  try createDirectory(noise)
  try Data("not media".utf8).write(to: noise.appending(path: "notes.txt"))
  try Data().write(to: noise.appending(path: "audio.wav"))
  try Data().write(to: root.appending(path: ".hidden.png"))
  let packageItem = root.appending(path: "sample.app/item.png")
  try createDirectory(packageItem.deletingLastPathComponent())
  try Data().write(to: packageItem)
  try FileManager.default.createSymbolicLink(
    at: root.appending(path: "linked-image.png"),
    withDestinationURL: originals[0]
  )
  try FileManager.default.createSymbolicLink(
    at: root.appending(path: "linked-directory"),
    withDestinationURL: originals[0].deletingLastPathComponent()
  )

  print("corpus media items=\(itemCount) images=\(imageCount) videos=\(videoCount) path=\(root.path)")
} catch {
  FileHandle.standardError.write(Data("make-corpus: \(error.localizedDescription)\n".utf8))
  exit(1)
}
