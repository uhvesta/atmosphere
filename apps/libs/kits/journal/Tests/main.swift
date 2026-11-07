import TestingKit

await runSuites([
    ("Journal", journalTests()),
    ("Revert", revertTests()),
    ("Merge", mergeTests()),
    ("Archive", archiveTests()),
])
