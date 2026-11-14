// Prints the id of the first on-screen window owned by a process, for
// `screencapture -l`. Exits 1 if it has none yet.
//
//   swift scripts/window-id.swift <pid>
import CoreGraphics
import Foundation

let pid = Int(CommandLine.arguments[1])!
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
    as? [[String: Any]] ?? []
for w in windows {
    if w[kCGWindowOwnerPID as String] as? Int == pid, w[kCGWindowLayer as String] as? Int == 0,
        let id = w[kCGWindowNumber as String] as? Int
    {
        print(id)
        exit(0)
    }
}
exit(1)
