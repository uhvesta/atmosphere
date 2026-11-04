import LifrTesting

runSuites([
    ("Identifiers", identifierTests()),
    ("Dates", dateTests()),
    ("Hierarchy", hierarchyTests()),
    ("Statuses", statusTests()),
    ("Procrastinated", procrastinationTests()),
    ("Comments and trash", commentAndTrashTests()),
    ("Reconciliation", reconciliationTests()),
])
