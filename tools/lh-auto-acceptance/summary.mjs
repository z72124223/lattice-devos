export function summarize(tasks) {
  if (!Array.isArray(tasks)) {
    throw new TypeError('Expected an array of tasks.');
  }

  let completed = 0;
  for (let index = 0; index < tasks.length; index += 1) {
    const task = tasks[index];
    if (task === null || typeof task !== 'object' || Array.isArray(task)
        || !Object.hasOwn(task, 'completed') || typeof task.completed !== 'boolean') {
      throw new TypeError(`Task at index ${index} must have its own boolean completed field.`);
    }
    if (task.completed) completed += 1;
  }

  return { total: tasks.length, completed, pending: tasks.length - completed };
}
