# Help the user read a change in a sensible order

The user is reviewing a change in Humata and wants help following it. You
**arrange and explain**; you don't review. Never say whether the change is
right, safe, good or ready, never give a verdict or a score, and don't list
"issues". Notes explain what the code does, how the pieces connect and where
to look; if something looks surprising, describe what it does and let the
user decide what to make of it.

## 1. Hand it to a small, fast model when you can

Ordering files and writing short notes doesn't need a large model. If your
agent can start a subagent on a smaller, faster model (for example a Haiku
subagent in Claude Code), give it this skill's steps 2 to 4, the diff spec
and the repository paths, and ask it to return the `order`, `groups` and
`notes` as JSON. Check what comes back against the diff (step 5) and apply
it yourself. If your agent can't do that, do the steps inline.

## 2. Get the diff

- In a review chat, call `review_context` for the diff spec showing now and
  what the user is looking at.
- Otherwise use the spec the user asked for (see /render-humata-review;
  `shared/diff-spec.md` has the fields).
- List the files with git (`git diff --stat <base>` and friends for that
  spec) and read the hunks of the files you order. Only files the diff
  shows can be ordered or noted.

## 3. Pick a direction

Ask the user only if they care; otherwise pick by the change's shape and
say which you picked:

- **Top-down**: entry points first (a route, a CLI command, a UI action, a
  public API), then what they call, then helpers, then tests. Good when the
  change adds or changes a feature: the user sees why before how.
- **Bottom-up**: leaf changes first (types, schemas, migrations, pure
  functions), then their callers, then the entry points. Good for refactors
  and data-model changes, where the callers only make sense once the new
  shape is known.

Put generated files, lockfiles and snapshots last, in their own group.
Put each test next to, or right after, the code it tests.

## 4. Group and note

- **Groups**: 2 to 6, each a short title for one step of the story
  ("Rounding in the refund calculation", "Callers", "Tests"), starting at
  the first file of that step in the order, with an optional one-sentence
  note on what the step does.
- **Notes**: few and useful, at most one or two per file. Each explains
  something the user would otherwise have to work out: what a function now
  does, which caller relies on it, why two files change together, what a
  renamed thing used to be called. Put a note on lines (`lines: {first,
  last}`, on the `new` side unless it is about removed code), on a hunk
  (`hunk_at`: any line inside it) or on the file (neither). Write plainly,
  in a sentence or two, without judging.

## 5. Check and apply

Check before applying: every file in `order`, `groups[].first` and `notes`
is in the diff; no file is listed twice; each group's `first` is in
`order`; line numbers are on the side you name. Files you didn't order
follow at the end on their own.

Then call `review_prepare`:

```json
{
  "title": "Refund rounding, top-down",
  "spec": { "repos": [{ "repo": "acme/api",
    "base": { "merge_base_with": "origin/main" } }] },
  "order": [
    { "repo": "acme/api", "path": "src/routes/refund.rs" },
    { "repo": "acme/api", "path": "src/billing/refund.rs" },
    { "repo": "acme/api", "path": "src/billing/money.rs" },
    { "repo": "acme/api", "path": "tests/refund.rs" }
  ],
  "groups": [
    { "title": "Where a refund starts",
      "first": { "repo": "acme/api", "path": "src/routes/refund.rs" } },
    { "title": "The calculation",
      "note": "Partial captures now round each part, then sum.",
      "first": { "repo": "acme/api", "path": "src/billing/refund.rs" } },
    { "title": "Tests",
      "first": { "repo": "acme/api", "path": "tests/refund.rs" } }
  ],
  "notes": [
    { "repo": "acme/api", "path": "src/billing/refund.rs",
      "lines": { "first": 140, "last": 152 },
      "text": "This loop is the new part: it rounds each capture with round_half_even before adding them up, where it used to round the total once." }
  ]
}
```

In a review chat it applies at once and shows in the chat with Undo. If
the user later edits the code, notes on lines that changed are kept and
marked outdated. If a tool says the user turned a switch off ("Order" or
"Notes"), tell them and stop; don't work around it.

Afterwards, tell the user in a sentence what order you chose and why, and
offer to walk them through it with `review_focus` one step at a time.
