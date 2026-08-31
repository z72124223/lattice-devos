/** Parses Linux /proc/<pid>/stat without splitting the parenthesized comm field. */
export function parseProcStat(stat) {
  if (typeof stat !== "string") throw new TypeError("WSL2_PROC_STAT_REJECTED");
  const close = stat.lastIndexOf(") ");
  const open = stat.indexOf(" (");
  if (open < 1 || close <= open) throw new TypeError("WSL2_PROC_STAT_REJECTED");
  const pid = stat.slice(0, open);
  const tail = stat.slice(close + 2).trim().split(/\s+/u);
  // tail[0] is state (field 3); pgrp is field 5 and starttime is field 22.
  if (!/^\d+$/u.test(pid) || !/^\d+$/u.test(tail[2] ?? "") || !/^\d+$/u.test(tail[19] ?? "")) {
    throw new TypeError("WSL2_PROC_STAT_REJECTED");
  }
  return Object.freeze({
    pid: Number(pid),
    processGroupId: Number(tail[2]),
    startTime: tail[19],
  });
}
