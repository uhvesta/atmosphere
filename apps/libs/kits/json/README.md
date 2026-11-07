# JSONKit

Any JSON value as a Swift enum, for what goes over the wire, tools' schemas
and models' answers. It's plain Swift with Foundation, so it builds with the
hermetic toolchains.

## Use it

```starlark
deps = ["//apps/libs/kits/json"]
```

```swift
import JSONKit

let args: JSONValue = ["title": "Buy milk", "done": false, "tags": ["home"]]
let task = try args.decode(TaskArguments.self)
let back = try JSONValue(task)

// One message per line, with sorted keys and ISO 8601 dates.
let line = try Wire.line(message)
let decoded = try Wire.decoder.decode(Message.self, from: line)
```

## What it gives you

- `JSONValue`: `.string`, `.number`, `.bool`, `.array`, `.object` and
  `.null`, written as Swift literals. `JSONValue(_:)` turns any `Encodable`
  into one, and `decode(_:)` turns one back.
- `Wire.encoder` and `Wire.decoder` agree on the format: sorted keys,
  unescaped slashes and ISO 8601 dates. `Wire.line(_:)` writes one value as
  a line of JSON. JSON escapes newlines inside strings, so a value never
  spans two lines.
