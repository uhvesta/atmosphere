import TestingKit

await runSuites([
    ("Skills", skillTests()),
    ("Proposals", proposalTests()),
])
