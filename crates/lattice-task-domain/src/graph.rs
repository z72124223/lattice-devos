use std::collections::BTreeMap;

use crate::TaskDomainError;
use crate::validation::validate_task_id;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Visit {
    Visiting,
    Visited,
}

/// Validates a complete dependency graph with stable traversal order.
///
/// # Errors
///
/// Rejects invalid task IDs, unknown dependencies, and cycles.
pub fn validate_task_graph(graph: &BTreeMap<String, Vec<String>>) -> Result<(), TaskDomainError> {
    for (task_id, dependencies) in graph {
        validate_task_id(task_id)?;
        for dependency in dependencies {
            validate_task_id(dependency)?;
            if !graph.contains_key(dependency) {
                return Err(TaskDomainError::UnknownTaskDependency {
                    task_id: task_id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }

    let mut marks = BTreeMap::new();
    let mut stack = Vec::new();
    for task_id in graph.keys() {
        visit(task_id, graph, &mut marks, &mut stack)?;
    }
    Ok(())
}

fn visit(
    task_id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    marks: &mut BTreeMap<String, Visit>,
    stack: &mut Vec<String>,
) -> Result<(), TaskDomainError> {
    match marks.get(task_id) {
        Some(Visit::Visited) => return Ok(()),
        Some(Visit::Visiting) => {
            let start = stack
                .iter()
                .position(|entry| entry == task_id)
                .expect("visiting task must be on traversal stack");
            let mut cycle = stack[start..].to_vec();
            cycle.push(task_id.to_owned());
            return Err(TaskDomainError::TaskDependencyCycle { cycle });
        }
        None => {}
    }

    marks.insert(task_id.to_owned(), Visit::Visiting);
    stack.push(task_id.to_owned());
    let mut dependencies = graph
        .get(task_id)
        .expect("validated graph node")
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    dependencies.sort_unstable();
    for dependency in dependencies {
        visit(dependency, graph, marks, stack)?;
    }
    let popped = stack.pop();
    debug_assert_eq!(popped.as_deref(), Some(task_id));
    marks.insert(task_id.to_owned(), Visit::Visited);
    Ok(())
}
