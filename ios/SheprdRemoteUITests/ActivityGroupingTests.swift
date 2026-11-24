import UIKit
import XCTest

final class ActivityGroupingTests: XCTestCase {
    @MainActor
    func testEachComputerKeepsItsOwnChoice() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "Exercises the settings flow on iPhone.")
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-demo", "YES", "-hideDemoBar", "YES", "-tab", "settings",
                               "-openNotifications", "YES", "-provisionalAlerts", "YES", "-AppleLanguages", "(en)"]
        app.launch()
        let studio = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Studio, ")).firstMatch
        let laptop = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "MacBook Air, ")).firstMatch
        XCTAssertTrue(studio.waitForExistence(timeout: 10))
        if !studio.label.hasSuffix("One per computer") {
            studio.tap()
            app.buttons["One per computer"].tap()
        }
        XCTAssertTrue(laptop.label.hasSuffix("One per computer"))
        studio.tap()
        app.buttons["Multiple"].tap()
        app.terminate()
        app.launch()
        XCTAssertTrue(studio.waitForExistence(timeout: 10))
        XCTAssertTrue(studio.label.hasSuffix("Multiple"))
        XCTAssertTrue(laptop.label.hasSuffix("One per computer"))
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "Activity grouping per computer"
        shot.lifetime = .keepAlways
        add(shot)
        studio.tap()
        app.buttons["One per computer"].tap()
    }
}
