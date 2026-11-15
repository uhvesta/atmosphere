# terminal_grid

What a terminal renderer reads, and nothing else: the cells in view, the
cursor and its shape, the colors and palette, the selection, and which rows
changed since the last frame. It also holds the key and mouse event types
the view emits and the engine encodes.

`ghostty_vt` in `terminal/vt` implements the `Grid` trait over
`libghostty-vt`'s render state. The `GPUI` view in `terminal/view` draws any
`Grid`, so it builds and tests without the engine, and an
`alacritty_terminal` fallback would only have to implement the trait.

It has no dependencies and uses no I/O, so it builds for
`wasm32-unknown-unknown` along with the view.

## The frame

```rust
grid.sync();                 // capture the engine's screen
for y in 0..grid.size().rows {
    if grid.is_row_dirty(y) {
        // reshape row y
    }
}
// paint, then
grid.clear_dirty();
```

## The fake

With the `fake` feature, `FakeGrid` is a grid filled by hand. Writing to a
row marks that row dirty; resizing, scrolling and changing colors mark
everything. Its width rules are rough (East Asian wide characters and
emoji), enough for test text.
