Prefix: VIE
Next: 2

### VIE-1 · P3 · Quick Look rebuilds its URL snapshot for each scan batch

- **Area:** Performance
- **Where:** `Sources/viewCore/PreviewSession.swift:35`, `Sources/view/PreviewPanelController.swift:100`, `Sources/view/Grid/GridViewController.swift:226`
- **Problem:** While the preview panel controls the window, each catalog batch rebuilds the URL list from every item already scanned. This repeats cumulative O(N) work on the main actor during discovery and can approach O(N²) as libraries grow. The 2,000-item startup measurement did not exercise scanning with Quick Look open.
- **Recommendation:** Measure scan responsiveness with Quick Look open on a larger corpus. If this path limits the UI, update the preview snapshot from catalog mutation deltas instead of remapping every item.
- **Fix with:** make-it-fast
- **Effort:** M (days)
