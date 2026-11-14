import TestingKit

await runSuites([
    ("Changes", changeTests()),
    ("Snippet", snippetTests()),
    ("Markdown", markdownTests()),
    ("Code colors", codeColorsTests()),
    ("Time marks", timeMarkTests()),
    ("Math", mathTests()),
    ("Text previews", textPreviewTests()),
    ("Text scale", textScaleTests()),
])
