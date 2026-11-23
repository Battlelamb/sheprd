import UIKit
import XCTest

final class HomeUnreadTests: XCTestCase {
    @MainActor
    func testToolbarFiltersAndRestoresDemoAgents() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "Home uses the agent list on iPhone.")
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "YES", "-hideDemoBar", "YES", "-provisionalAlerts", "YES",
                               "-AppleLanguages", "(en)"]
        app.launch()

        let unread = app.navigationBars.buttons["Unread 1"]
        XCTAssertTrue(unread.waitForExistence(timeout: 10))
        let readAgent = app.staticTexts["Ship the widget fix"]
        if unread.value as? String == "Unread Only" {
            unread.tap()
            app.buttons["Unread Only"].tap()
        }
        XCTAssertTrue(readAgent.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Mark All Read"].exists)

        unread.tap()
        XCTAssertTrue(app.buttons["Mark All Read"].exists)
        app.buttons["Unread Only"].tap()
        XCTAssertTrue(readAgent.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Weather widget"].exists)

        unread.tap()
        app.buttons["Unread Only"].tap()
        XCTAssertTrue(readAgent.waitForExistence(timeout: 5))
    }
}
