import TestingKit

runSuites([
    ("Journal", journalTests()),
    ("Revert", revertTests()),
    ("Merge", mergeTests()),
])
