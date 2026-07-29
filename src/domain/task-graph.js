import { domainFailure } from "./errors.js";

export function assertAcyclicTaskGraph(graph) {
  if (
    graph === null ||
    typeof graph !== "object" ||
    Array.isArray(graph) ||
    Object.getPrototypeOf(graph) !== Object.prototype
  ) {
    domainFailure("INVALID_TASK_GRAPH", "Task graph must be a plain object.");
  }

  const taskIds = Object.keys(graph).sort();
  const knownTasks = new Set(taskIds);
  for (const taskId of taskIds) {
    const dependencies = graph[taskId];
    if (!Array.isArray(dependencies)) {
      domainFailure(
        "INVALID_TASK_GRAPH",
        `Dependencies for '${taskId}' must be an array.`,
      );
    }
    for (const dependency of dependencies) {
      if (typeof dependency !== "string" || !knownTasks.has(dependency)) {
        domainFailure(
          "UNKNOWN_TASK_DEPENDENCY",
          `Task '${taskId}' depends on unknown task '${String(dependency)}'.`,
          { task_id: taskId, dependency },
        );
      }
    }
  }

  const visiting = new Set();
  const visited = new Set();
  const stack = [];

  function visit(taskId) {
    if (visiting.has(taskId)) {
      const cycleStart = stack.indexOf(taskId);
      const cycle = [...stack.slice(cycleStart), taskId];
      domainFailure("TASK_DEPENDENCY_CYCLE", "Task dependency cycle detected.", {
        cycle,
      });
    }
    if (visited.has(taskId)) {
      return;
    }
    visiting.add(taskId);
    stack.push(taskId);
    for (const dependency of [...graph[taskId]].sort()) {
      visit(dependency);
    }
    stack.pop();
    visiting.delete(taskId);
    visited.add(taskId);
  }

  for (const taskId of taskIds) {
    visit(taskId);
  }

  return true;
}

