import LifrCore
import LifrFeedbackClient
import XCTest

final class FeedbackTests: XCTestCase {
    func testOnlyDoneFeelsLikeSuccess() {
        XCTAssertEqual(Feedback(status: .done), .success)
        for status in [TaskStatus.todo, .inProgress, .procrastinated, .wontDo] {
            XCTAssertEqual(Feedback(status: status), .tap)
        }
    }
}
