import TestingKit

await runSuites([
    ("Cloud items", itemTests()),
    ("Sync", syncTests()),
])
