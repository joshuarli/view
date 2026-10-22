import Foundation

enum PathOrder {
  private static let options: String.CompareOptions = [
    .numeric,
    .caseInsensitive,
    .forcedOrdering,
  ]

  static func comesBefore(_ lhs: String, _ rhs: String) -> Bool {
    let comparison = lhs.compare(rhs, options: options)
    if comparison != .orderedSame {
      return comparison == .orderedAscending
    }
    return lhs.utf8.lexicographicallyPrecedes(rhs.utf8)
  }
}
