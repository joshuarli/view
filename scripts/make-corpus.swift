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
      "usage: swift scripts/make-corpus.swift .work/corpus"
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

func createPNG(at url: URL, colorIndex: Int) throws {
  try createDirectory(url.deletingLastPathComponent())
  guard
    let context = CGContext(
      data: nil,
      width: 400,
      height: 300,
      bitsPerComponent: 8,
      bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ),
    let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
  else {
    throw CorpusError.imageCreation(url)
  }

  let red = CGFloat((colorIndex * 67 + 31) % 256) / 255
  let green = CGFloat((colorIndex * 131 + 73) % 256) / 255
  let blue = CGFloat((colorIndex * 197 + 149) % 256) / 255
  context.setFillColor(red: red, green: green, blue: blue, alpha: 1)
  context.fill(CGRect(x: 0, y: 0, width: 400, height: 300))
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
  guard CommandLine.arguments.count == 2 else { throw CorpusError.usage }
  let root = try corpusDestination(CommandLine.arguments[1])
  try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)

  var originals: [URL] = []
  originals.reserveCapacity(1_000)
  for index in 0..<1_000 {
    let folder = String(format: "%02d", index / 100)
    let image = root.appending(path: "images/group-\(folder)/image-\(String(format: "%04d", index)).png")
    try createPNG(at: image, colorIndex: index)
    originals.append(image)
  }

  for index in 0..<980 {
    let folder = String(format: "%02d", index / 100)
    let copy = root.appending(path: "copies/group-\(folder)/copy-\(String(format: "%04d", index)).png")
    try createDirectory(copy.deletingLastPathComponent())
    try FileManager.default.copyItem(at: originals[index], to: copy)
  }

  for index in 0..<20 {
    let video = root.appending(path: "videos/clip-\(String(format: "%02d", index)).mov")
    try createVideo(at: video, index: index)
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

  print("corpus media items=2000 images=1980 videos=20 path=\(root.path)")
} catch {
  FileHandle.standardError.write(Data("make-corpus: \(error.localizedDescription)\n".utf8))
  exit(1)
}
