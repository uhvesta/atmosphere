# FlowKit

Snapshot tests for SwiftUI apps: of single views, and of whole flows a person
takes through the app. Each step of a flow checks two references: an image,
and the screen's accessibility tree as text, so a reviewer can read what
changed from step to step in `git diff`.

## Use it

```starlark
load("//apps/libs/kits/flow:defs.bzl", "flow_host_app", "flow_snapshot_test")

# Only for flows through sheets and other presented screens.
flow_host_app(name = "host", bundle_id = "com.example.app.flowhost")

flow_snapshot_test(
    name = "flow_snapshot_test",
    srcs = glob(["FlowTests/*.swift"]),
    host = ":host",
    deps = [":app_lib"],
)

flow_snapshot_test(
    name = "mac_flow_snapshot_test",
    srcs = glob(["MacFlowTests/*.swift"]),
    mac = True,
    deps = [":app_lib"],
)
```

```swift
import FlowKit

@MainActor
func testRenaming() async {
    let flow = FlowRecorder("rename", AppView(store: store))
    await flow.step("list")
    flow.tap("Rename")
    await flow.step("renaming")
    flow.finish()
}
```

References live next to the tests in `SnapshotTests/__Snapshots__/`.

- Verify: `bazel test --config=xcode //pkg:flow_snapshot_test`
- Record: `bazel test --config=record_snapshots //pkg:flow_snapshot_test`.
  A recording run always fails, so it can't pass for a real run. Review the
  new references with `git diff`.
- Mac: `apps/libs/kits/flow/run_mac.sh //pkg:mac_flow_snapshot_test [record]`.
  The terminal needs Accessibility and Screen Recording access.
  Mac tests share the one screen, so they run one at a time: a second run
  builds, then waits for the first to finish.

## What it gives you

- `FlowRecorder` keeps one live screen through a flow. On macOS it covers
  every window on screen, sheets included.
  - `step` waits for the screen to settle, then checks it.
  - `tap` (iOS) presses a control by its label, the way VoiceOver does.
  - `scrollToEnd` (iOS) scrolls the longest list on screen to its end.
- `assertSnapshot(of:as:named:)` checks a single view or value, and
  `assertScreenImages` checks a view at several screen sizes.
- Text a model wrote changes from run to run. Give it the accessibility
  identifier `modelTextIdentifier` (`"model-text"`), and flows leave it out.
- A control whose state comes from the machine, such as the system's Paste
  button, which is on only while the clipboard holds text: give it the
  identifier `machineStateIdentifier` (`"machine-state"`), and flows leave
  out whether it's on.
- All iOS tests render on one pinned simulator
  (`//apps/libs/kits/flow:iphone_17`), so references don't drift from one
  machine to another.
- `SeededRandomNumberGenerator` gives the same numbers on every run, so
  anything made at random, such as a new record's key, matches its
  reference.
