import LifrTesting

runSuites([
    ("Access", accessTests()),
    ("Rules", ruleTests()),
    ("Requests", requestTests()),
    ("Wire", wireTests()),
])
