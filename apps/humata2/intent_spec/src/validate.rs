use std::collections::{HashMap, HashSet};

use crate::{IntentSpec, ValidationIssue, ValidationReport, SCHEMA_VERSION};

pub fn validate_spec(spec: &IntentSpec) -> ValidationReport {
    let mut errors = Vec::new();
    let mut blocking_open_questions = Vec::new();

    if spec.schema.trim() != SCHEMA_VERSION {
        errors.push(ValidationIssue {
            path: "schema".to_string(),
            message: format!(
                "expected schema `{SCHEMA_VERSION}`, found `{}`",
                spec.schema
            ),
        });
    }

    if spec.metadata.id.trim().is_empty() {
        errors.push(ValidationIssue {
            path: "metadata.id".to_string(),
            message: "metadata.id must not be empty".to_string(),
        });
    }
    if spec.metadata.title.trim().is_empty() {
        errors.push(ValidationIssue {
            path: "metadata.title".to_string(),
            message: "metadata.title must not be empty".to_string(),
        });
    }
    if spec.metadata.channel.trim().is_empty() {
        errors.push(ValidationIssue {
            path: "metadata.channel".to_string(),
            message: "metadata.channel must not be empty".to_string(),
        });
    }
    if spec.outcome.trim().is_empty() {
        errors.push(ValidationIssue {
            path: "outcome".to_string(),
            message: "outcome must not be empty".to_string(),
        });
    }

    if spec.steps.is_empty() {
        errors.push(ValidationIssue {
            path: "steps".to_string(),
            message: "at least one step is required".to_string(),
        });
    }

    let mut step_ids: HashMap<&str, usize> = HashMap::new();
    for (idx, step) in spec.steps.iter().enumerate() {
        let base = format!("steps[{idx}]");
        if step.id.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: "step id must not be empty".to_string(),
            });
        } else if step_ids.insert(step.id.as_str(), idx).is_some() {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: format!("duplicate step id `{}`", step.id),
            });
        }
        if step.title.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.title"),
                message: "step title must not be empty".to_string(),
            });
        }
        if step.outcome.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.outcome"),
                message: "step outcome must not be empty".to_string(),
            });
        }
        if step.verify.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.verify"),
                message: "step verify must not be empty".to_string(),
            });
        }
    }

    // Validate depends_on references
    for (idx, step) in spec.steps.iter().enumerate() {
        for (dep_idx, dep) in step.depends_on.iter().enumerate() {
            if dep == &step.id {
                errors.push(ValidationIssue {
                    path: format!("steps[{idx}].depends_on[{dep_idx}]"),
                    message: format!("step `{}` cannot depend on itself", step.id),
                });
            } else if !step_ids.contains_key(dep.as_str()) {
                errors.push(ValidationIssue {
                    path: format!("steps[{idx}].depends_on[{dep_idx}]"),
                    message: format!("unknown step dependency `{dep}`"),
                });
            }
        }
    }

    // DFS cycle detection over steps[].depends_on
    // 0 = unvisited, 1 = visiting, 2 = visited
    let mut state: HashMap<&str, u8> = HashMap::new();
    let mut reported_cycles: HashSet<String> = HashSet::new();
    for step in &spec.steps {
        if step.id.trim().is_empty() {
            continue;
        }
        if state.get(step.id.as_str()).copied().unwrap_or(0) == 0 {
            let mut stack = Vec::new();
            dfs_cycles(
                step.id.as_str(),
                &spec.steps,
                &step_ids,
                &mut state,
                &mut stack,
                &mut reported_cycles,
                &mut errors,
            );
        }
    }

    // Validate acceptance criteria
    let mut acc_ids: HashSet<&str> = HashSet::new();
    for (idx, acc) in spec.acceptance.iter().enumerate() {
        let base = format!("acceptance[{idx}]");
        if acc.id.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: "acceptance id must not be empty".to_string(),
            });
        } else if !acc_ids.insert(acc.id.as_str()) {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: format!("duplicate acceptance id `{}`", acc.id),
            });
        }
        if acc.given.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.given"),
                message: "acceptance given must not be empty".to_string(),
            });
        }
        if acc.when.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.when"),
                message: "acceptance when must not be empty".to_string(),
            });
        }
        if acc.then.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.then"),
                message: "acceptance then must not be empty".to_string(),
            });
        }
    }

    // Validate open questions
    let mut q_ids: HashSet<&str> = HashSet::new();
    for (idx, q) in spec.open_questions.iter().enumerate() {
        let base = format!("open_questions[{idx}]");
        if q.id.trim().is_empty() {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: "open_question id must not be empty".to_string(),
            });
        } else if !q_ids.insert(q.id.as_str()) {
            errors.push(ValidationIssue {
                path: format!("{base}.id"),
                message: format!("duplicate open_question id `{}`", q.id),
            });
        }
        if q.blocking && !q.resolved {
            blocking_open_questions.push(q.id.clone());
        }
    }

    ValidationReport {
        errors,
        blocking_open_questions,
    }
}

fn dfs_cycles<'a>(
    node: &'a str,
    steps: &'a [crate::Step],
    step_ids: &HashMap<&'a str, usize>,
    state: &mut HashMap<&'a str, u8>,
    stack: &mut Vec<&'a str>,
    reported_cycles: &mut HashSet<String>,
    errors: &mut Vec<ValidationIssue>,
) {
    state.insert(node, 1);
    stack.push(node);

    if let Some(&idx) = step_ids.get(node) {
        for dep in &steps[idx].depends_on {
            if dep == node {
                // Self-dependency already reported above.
                continue;
            }
            if !step_ids.contains_key(dep.as_str()) {
                continue;
            }
            match state.get(dep.as_str()).copied().unwrap_or(0) {
                0 => dfs_cycles(
                    dep.as_str(),
                    steps,
                    step_ids,
                    state,
                    stack,
                    reported_cycles,
                    errors,
                ),
                1 => {
                    if let Some(pos) = stack.iter().position(|&n| n == dep.as_str()) {
                        let mut cycle: Vec<&str> = stack[pos..].to_vec();
                        cycle.push(dep.as_str());
                        let desc = cycle.join(" -> ");
                        if reported_cycles.insert(desc.clone()) {
                            errors.push(ValidationIssue {
                                path: format!("steps[{idx}].depends_on"),
                                message: format!("dependency cycle detected: {desc}"),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    stack.pop();
    state.insert(node, 2);
}
