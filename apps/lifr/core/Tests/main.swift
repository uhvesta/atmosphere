import LifrTesting

runSuites([
    ("Identifiers", identifierTests()),
    ("Dates", dateTests()),
    ("Hierarchy", hierarchyTests()),
    ("Statuses", statusTests()),
    ("Procrastinated", procrastinationTests()),
    ("Comments and trash", commentAndTrashTests()),
    ("Reminders", reminderTests()),
    ("Reconciliation", reconciliationTests()),
    ("Reverting from history", revertTests()),
    ("Plans", planTests()),
    ("Repairing plans", planRepairTests()),
    ("Projects and Avenues", projectAndAvenueTests()),
    ("Journal", journalTests()),
    ("Merging synced records", mergeTests()),
])
