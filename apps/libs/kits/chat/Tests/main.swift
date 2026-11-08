import TestingKit

await runSuites([
    ("Branches", branchTests()),
    ("Compaction", compactionTests()),
    ("Search", searchTests()),
    ("Threads", threadTests()),
])
