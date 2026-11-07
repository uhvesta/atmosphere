import TestingKit

await runSuites([
    ("Import", importTests()),
    ("Book", bookTests()),
    ("Shelf", shelfTests()),
])
