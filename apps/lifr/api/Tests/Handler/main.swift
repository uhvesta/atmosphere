import TestingKit

await runSuites([
    ("Access", accessTests()),
    ("Rules", ruleTests()),
    ("Requests", requestTests()),
    ("Wire", wireTests()),
    ("Plan files", planFileTests()),
    ("Plan prompts", planPromptTests()),
    ("Adding plans", addPlanTests()),
])
