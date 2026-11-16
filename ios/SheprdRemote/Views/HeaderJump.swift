import SwiftUI
import UIKit

/// Brings a list's section header to just below its bars.
///
/// A `List` scrolls to a header's id by lining the section's first row up
/// with the top of what the bars leave visible, which leaves the header
/// itself behind a search field, and SwiftUI offers no way to scroll a list
/// by a given amount. So each header carries a `HeaderMark`, an invisible
/// view whose window position says where the header is, and the jump moves
/// the list's own scroll view until that header's top meets the bars.
@MainActor
final class HeaderJump {
    private final class Mark {
        weak var view: UIView?
    }

    private var marks: [String: Mark] = [:]
    /// Room left between the bars and the header's title.
    private static let gap: CGFloat = 8

    fileprivate func register(_ view: UIView, for key: String) {
        let mark = marks[key] ?? Mark()
        mark.view = view
        marks[key] = mark
    }

    /// Scrolls so the header marked `key` sits just below the bars, once
    /// `proxy` has brought it into view; nothing when it is not on screen.
    func settle(_ key: String) {
        guard let view = marks[key]?.view,
              let scroll = sequence(first: view.superview, next: { $0?.superview }).lazy
                  .compactMap({ $0 as? UIScrollView }).first
        else { return }
        let top = view.convert(view.bounds, to: scroll).minY
        let lowest = -scroll.adjustedContentInset.top
        let highest = max(lowest, scroll.contentSize.height + scroll.adjustedContentInset.bottom - scroll.bounds.height)
        let y = min(max(top - scroll.adjustedContentInset.top - Self.gap, lowest), highest)
        scroll.setContentOffset(CGPoint(x: scroll.contentOffset.x, y: y), animated: false)
    }
}

/// The invisible view a section header carries for `HeaderJump`, as wide
/// and tall as the header itself.
struct HeaderMark: UIViewRepresentable {
    let jump: HeaderJump
    let key: String

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        view.backgroundColor = .clear
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        jump.register(view, for: key)
    }
}
