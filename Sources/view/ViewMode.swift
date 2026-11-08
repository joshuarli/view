@MainActor
enum ViewMode: String, CaseIterable, Sendable {
  case carousel
  case grid

  var title: String {
    switch self {
    case .carousel: "as Carousel"
    case .grid: "as Grid"
    }
  }
}
