/// The accessibility identifier an app gives text a model wrote, such as
/// its replies. Flow steps don't show it, and Mac flows can leave the text
/// itself out, since it can differ from one GPU to the next.
public let modelTextIdentifier = "model-text"

/// The accessibility identifier an app gives a control whose state comes
/// from the machine rather than the app, such as the system's Paste button,
/// which is on only while the clipboard holds text. Flow steps leave out
/// whether it's on, and the identifier itself.
public let machineStateIdentifier = "machine-state"
