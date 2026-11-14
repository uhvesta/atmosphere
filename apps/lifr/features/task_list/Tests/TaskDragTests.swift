import Foundation
import LifrUI
import XCTest

final class TaskDragTests: XCTestCase {
    func testSeveralTasksRoundTripInOrder() {
        let ids = [UUID(), UUID(), UUID()]
        XCTAssertEqual(TaskDrag.ids(in: [TaskDrag.text(ids)]), ids)
    }

    func testOneTaskIsItsIDAlone() {
        let id = UUID()
        XCTAssertEqual(TaskDrag.text([id]), id.uuidString)
        XCTAssertEqual(TaskDrag.ids(in: [id.uuidString]), [id])
    }

    func testTextThatIsntAllTaskIDsCarriesNoTasks() {
        XCTAssertNil(TaskDrag.ids(in: ["\(UUID().uuidString)\nBuy milk"]))
        XCTAssertNil(TaskDrag.ids(in: ["Buy milk"]))
        XCTAssertNil(TaskDrag.ids(in: []))
    }
}
