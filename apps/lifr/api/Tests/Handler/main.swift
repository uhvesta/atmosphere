import TestingKit

await runSuites([
    ("Access", accessTests()),
    ("Rules", ruleTests()),
    ("Requests", requestTests()),
    ("Wire", wireTests()),
    ("Plan files", planFileTests()),
    ("Adding plans", addPlanTests()),
])
