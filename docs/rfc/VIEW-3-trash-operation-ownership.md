# VIEW-3: Own trash operations outside tab catalogs

Status: accepted (implemented)

## Summary

Give the application session one owner for pending file operations and trash
history. Capture each operation's inputs before calling `NSWorkspace`; apply its
completion from that snapshot and reconcile every affected tab. Navigation and
tab closure must not erase a completed move or redirect its undo into another
directory.

## Motivation

Today `trash_selected` captures paths, then finds the first tab whose current
catalog contains a successful path. It reconstructs the undo transaction from
that catalog. Duplicate tabs can assign history to the wrong tab; navigation can
leave no matching tab. Undo restores a file to its recorded original path but
inserts the item into the tab's current catalog, even when that tab shows another
directory. [VIEW-3](../../TODO.md#view-3--p2--trash-completion-and-undo-follow-mutable-tab-catalogs)
records the reproduced cases and verification limits.

Two adjacent defects constrain the repair: mixed-result recycle completions must
retain successes despite an error (VIEW-4), and catalog reconciliation must
preserve actual selected paths when a selection becomes sparse (VIEW-5).

## Design

### Capture operations independently of views

Add a small session-owned file-operation state to `AppState`. Before dispatch,
capture an operation ID and the selected item snapshots, including original
paths. An optional stable originating-tab ID may guide selection or feedback;
an array index or a search through current catalogs must not establish ownership.
Closing the originating tab leaves the operation and its history intact.

Complete each operation on the main thread using its captured inputs and the
returned original-to-Trash mapping. Record every successful move, remove it from
affected catalogs, and present any failure. Failed inputs remain available for
retry. A non-null error does not invalidate successful mappings.

Reject conflicting mutations of paths already involved in a pending operation.
Keep navigation available while the filesystem operation runs. Undo and redo
must not race an unfinished operation on the same paths; define their availability
from pending state rather than from the active tab's catalog. Avoid a background
queue or persistent journal unless a separate requirement calls for one.

### Keep history in the session

Recommend one window-wide undo/redo history for Move to Trash. Each transaction
records successful original and Trash paths, plus the item information needed
for reconciliation. Cmd+Z acts on the latest completed trash transaction even
after navigation or closure of its originating tab. The session ends when the
application exits; persistence across restarts is outside this proposal.

Undo and redo move files using those recorded paths. Preserve exclusive rename,
destination-conflict errors, and retryable failed entries on their current
history stack. Keep history path updates consistent with rename. Never derive a
restore destination or catalog insertion from the directory currently displayed.

### Reconcile every affected catalog

After each successful mutation, reconcile all tabs whose root and browser mode
can include the changed paths. Reloading those listings through an error-aware
boundary is a simple initial implementation; targeted updates are acceptable
only if they obey the same membership and ordering rules. An unavailable listing
must remain distinguishable from a successful empty one (VIEW-6).

Preserve each tab's surviving selected paths, anchor, and focus explicitly.
Restoring a file must not select it merely because it falls between the anchor
and focus. Refresh the displayed tab once after reconciliation. A tab showing
`/B` must never acquire `/A/x` solely because an undo transaction originated in
`/A`.

## Alternatives considered

- **History attached to stable tab IDs.** This preserves per-tab undo, but requires
  an explicit history destination when a tab closes and makes the same filesystem
  mutation appear differently in duplicate views. Stable identity alone does not
  solve reconciliation or undo after navigation.
- **Match a completion to its original root.** A root can identify affected views,
  but several tabs may share it and all may navigate away. It cannot own history.
- **Disable navigation during trash.** This reduces one race while leaving
  duplicate tabs, partial failures, and later cross-directory undo unresolved.

## Open questions

Settled on implementation (user decision):

1. **Undo scope:** window-wide undo/redo history. Cmd+Z acts on the latest
   completed trash transaction regardless of navigation or tab closure.
2. **Unavailable roots:** retain the tab with an unavailable-directory message.
   Reconciliation does not change navigation. An unavailable tab keeps its
   catalog, shows the error instead of its contents, and rejects stale file
   operations until a listing succeeds again.

## Validation

Exercise the real application callback and presentation on macOS, with a
controlled completion seam for deterministic success and failure combinations:

- Trash from the second of two tabs on the same directory; reconcile both and
  create exactly one history transaction.
- Navigate away or close the initiating tab before completion; retain the
  successful move and make its undo available under the chosen history policy.
- Return a success mapping together with an error; preserve the success, show
  the failure, and undo only the successful subset.
- Trash in `/A`, navigate to `/B`, then undo and redo; verify filesystem paths and
  every catalog without inserting foreign-root items.
- Restore an item into the middle of a sparse selection; preserve the selected
  paths in drawing, dragging, rename reconciliation, and subsequent trash.
- Encounter an existing restore destination or a failed move; preserve its
  contents and keep the failed history entry retryable.

## Decision

Accepted: session-owned operations, window-wide trash history, and retained
unavailable tabs, as implemented. The mixed-result completion is covered in the
app regression harness through a controlled seam; native NSWorkspace
mixed-result mappings were not exercised end-to-end.
