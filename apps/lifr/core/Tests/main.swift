import TestingKit

await runSuites([
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
    ("Saved formats", savedFormatTests()),
    ("Merging synced records", mergeTests()),
    ("Saved filters", savedFilterTests()),
])
