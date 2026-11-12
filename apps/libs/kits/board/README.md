# Board

SwiftUI columns and cards for a board, such as Lifr's tasks or Humata's
tasks and subtasks. The views take plain data and closures and have no
store, so they work with TCA, with an observable model, or in a preview.
The app supplies every word they show.

- `BoardColumn`: a header with the column's title and count, its cards,
  then a footer such as an Add button. Dropping cards on it hands their
  ids to `onDrop`.
- `BoardCard`: a card you tap to open, drag by its id, and move from its
  context menu.

## Use it

```starlark
deps = ["//apps/libs/kits/board"]
```

```swift
import Board

ScrollView(.horizontal) {
    HStack(alignment: .top, spacing: 12) {
        ForEach(columns) { column in
            BoardColumn(
                title: column.title, count: column.cards.count,
                countLabel: Text("\(column.title), \(column.cards.count) tasks")
            ) {
                ForEach(column.cards) { card in
                    BoardCard(id: card.id) {
                        Text(card.title)
                    } menu: {
                        Button("Move to Done") { move([card.id], to: .done) }
                    } onTap: {
                        open(card)
                    }
                }
            } onDrop: { ids in
                move(ids, to: column)
            }
        }
    }
}
```
