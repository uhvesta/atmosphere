import TestingKit

await runSuites([
    ("Access", accessTests()),
    ("Rules", ruleTests()),
    ("Requests", requestTests()),
    ("Wire", wireTests()),
    ("Plan files", planFileTests()),
    ("Plan guide", planGuideTests()),
    ("Plan prompts", planPromptTests()),
    ("Adding plans", addPlanTests()),
])
