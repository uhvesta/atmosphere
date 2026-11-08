import TestingKit

await runSuites([
    ("Changes", changeTests()),
    ("Snippet", snippetTests()),
    ("Markdown", markdownTests()),
])
