import Foundation

struct TestDirectory {
  let url: URL

  static func make() throws -> TestDirectory {
    let base = packageRoot
      .appending(path: ".work/tmp/unit-tests")
      .appending(path: UUID().uuidString)
    try FileManager.default.createDirectory(at: base, withIntermediateDirectories: true)
    return TestDirectory(url: base)
  }

  @discardableResult
  func createDirectory(_ name: String) throws -> TestDirectory {
    let child = url.appending(path: name)
    try FileManager.default.createDirectory(at: child, withIntermediateDirectories: true)
    return TestDirectory(url: child)
  }

  @discardableResult
  func createFile(_ name: String, contents: String) throws -> URL {
    let child = url.appending(path: name)
    try Data(contents.utf8).write(to: child)
    return child
  }

  func remove() {
    try? FileManager.default.removeItem(at: url)
  }

  private static var packageRoot: URL {
    URL(filePath: #filePath)
      .deletingLastPathComponent()
      .deletingLastPathComponent()
      .deletingLastPathComponent()
  }
}
