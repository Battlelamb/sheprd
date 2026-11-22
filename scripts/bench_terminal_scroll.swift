import ApplicationServices
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2,
      let duration = Double(CommandLine.arguments[1]), duration > 0 else {
    fputs("usage: bench-terminal-scroll SECONDS\n", stderr)
    exit(2)
}
guard AXIsProcessTrusted() else {
    fputs("scroll helper lacks Accessibility permission\n", stderr)
    exit(1)
}

let end = ProcessInfo.processInfo.systemUptime + duration
var step = 0
while ProcessInfo.processInfo.systemUptime < end {
    let delta: Int32 = step % 200 < 100 ? 3 : -3
    guard let event = CGEvent(scrollWheelEvent2Source: nil, units: .line,
                              wheelCount: 1, wheel1: delta, wheel2: 0, wheel3: 0) else {
        fputs("could not create scroll event\n", stderr)
        exit(1)
    }
    event.location = CGPoint(x: 700, y: 450)
    event.post(tap: .cghidEventTap)
    step += 1
    Thread.sleep(forTimeInterval: 0.02)
}
