import SwiftUI
import UIKit

extension View {
    /// Selects the whole text of the field that starts editing while `active`,
    /// so typing replaces it. For a rename alert's prefilled name: SwiftUI
    /// gives an alert's text field no selection, and modifiers on the field
    /// itself are dropped when the alert becomes a `UIAlertController`.
    func selectsAllOnEdit(while active: Bool) -> some View {
        onReceive(NotificationCenter.default.publisher(for: UITextField.textDidBeginEditingNotification)) { note in
            guard active, let field = note.object as? UITextField else { return }
            // After UIKit places the caret for this edit, or it wins.
            DispatchQueue.main.async { field.selectAll(nil) }
        }
    }
}
