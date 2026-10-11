// Called only by the owned, loopback PostgreSQL 17 fixture harness.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { applyProjectPurge } from '../apps/lattice-control/src/project-purge.mjs';
import { previewProjectPurgeSqlite } from '../apps/lattice-control/src/project-purge-sqlite.mjs';

export async function prepareRetainedRoots({ root, env, seeded, sql, runtimeSql, rows, hash, check, git }) {
  const physical = path.join(root, 'retained-git-owner');
  const worktree = path.join(root, 'retained-worktree');
  const sharedEnv = path.join(root, 'retained-environment');
  const claimLink = path.join(root, 'claim-junction');
  const source = seeded.targetCanonicalPath;
  fs.renameSync(source, physical);
  fs.mkdirSync(sharedEnv);
  git(physical, ['init']); git(physical, ['add', 'synthetic.txt']);
  git(physical, ['-c', 'commit.gpgsign=false', '-c', 'user.name=Synthetic fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'synthetic retained source']);
  git(physical, ['worktree', 'add', '--detach', worktree, 'HEAD']);
  fs.writeFileSync(path.join(sharedEnv, 'synthetic-environment.txt'), 'shared environment must remain');
  fs.writeFileSync(path.join(worktree, 'retained-knowledge.txt'), 'retained research and model evidence');
  fs.symlinkSync(sharedEnv, path.join(physical, '.venv'), 'junction');
  fs.symlinkSync(physical, source, 'junction');
  fs.symlinkSync(worktree, claimLink, 'junction');
  const quote = value => `'${String(value).replaceAll("'", "''")}'`;
  runtimeSql(env, `BEGIN ISOLATION LEVEL SERIALIZABLE;
    SELECT control_product.metadata_write_v1('${seeded.targetTaskRef}','retained-meta',repeat('1',64),0,'Synthetic','Retained roots',2,NULL,ARRAY[]::text[]);
    SELECT control_product.claim_v1('${seeded.targetTaskRef}','retained-claim',repeat('2',64),'EXECUTION','synthetic','gpt-6-astra',${quote(claimLink)});
    SELECT control_product.observe_v1('retained-claim','retained-bound',repeat('3',64),0,'THREAD_BOUND','synthetic-retained-thread',NULL,'synthetic binding',NULL,NULL,NULL,NULL,NULL);
    SELECT control_product.observe_v1('retained-claim','retained-dispatch',repeat('5',64),1,'DISPATCH_STARTED','synthetic-retained-thread',NULL,'synthetic dispatch',NULL,NULL,NULL,'retained-claim',NULL);
    SELECT control_product.observe_v1('retained-claim','retained-turn',repeat('6',64),2,'TURN_BOUND','synthetic-retained-thread','synthetic-retained-turn','synthetic turn',NULL,NULL,NULL,'retained-claim',NULL);
    SELECT control_product.observe_v1('retained-claim','retained-complete',repeat('7',64),3,'TURN_COMPLETED','synthetic-retained-thread','synthetic-retained-turn','synthetic complete',NULL,NULL,NULL,'retained-claim',NULL);
    SELECT control_product.observe_v1('retained-claim','retained-archive',repeat('4',64),4,'ARCHIVED','synthetic-retained-thread','synthetic-retained-turn','synthetic archival',NULL,NULL,NULL,NULL,NULL);
    COMMIT;`);
  const retainedSnapshot = () => {
    const entries = [];
    const visit = dir => {
      for (const name of fs.readdirSync(dir).sort()) {
        const file = path.join(dir, name), stat = fs.lstatSync(file);
        if (stat.isSymbolicLink()) entries.push([file, 'link', fs.readlinkSync(file)]);
        else if (stat.isDirectory()) { entries.push([file, 'directory']); visit(file); }
        else entries.push([file, hash(fs.readFileSync(file))]);
      }
    };
    for (const dir of [physical, worktree, sharedEnv, seeded.survivorCanonicalPath]) visit(dir);
    return entries;
  };
  const retainedBefore = retainedSnapshot();
  return { async apply({ plan, sqlitePath, boundaries, botRows, native, repreview }) {
    assert.deepEqual(new Set(plan.files.roots), new Set([source, claimLink]));
    assert.equal(plan.files.entries.length, 2);
    assert.ok(plan.files.entries.every(entry => entry.targetAction === 'EXCLUDED_NOT_FOLLOWED'));
    for (const [key, value] of Object.entries(env)) if (key.startsWith('LATTICE_')) process.env[key] = value;
    const apply = () => applyProjectPurge(plan, { confirmDigest: plan.digest, maintenanceOffline: true, botBoundaries: boundaries });
    const pgBefore = rows(env, { includeMaintenance: true }), botBefore = botRows();
    const sqliteBefore = previewProjectPurgeSqlite(plan.sqlite).beforeDigest;
    const sqliteBytesBefore = hash(fs.readFileSync(sqlitePath));
    // The approved junction binding changes; reject before either real PG mutates.
    fs.unlinkSync(source); fs.symlinkSync(sharedEnv, source, 'junction');
    await assert.rejects(apply(), /PURGE_FILE_/);
    assert.deepEqual(rows(env, { includeMaintenance: true }), pgBefore);
    assert.deepEqual(botRows(), botBefore);
    assert.equal(previewProjectPurgeSqlite(plan.sqlite).beforeDigest, sqliteBefore);
    assert.equal(hash(fs.readFileSync(sqlitePath)), sqliteBytesBefore);
    fs.unlinkSync(source); fs.symlinkSync(physical, source, 'junction');
    const fresh = await repreview();
    assert.equal(fresh.status, 'READY', JSON.stringify(fresh.blockers));
    Object.assign(plan, fresh);
    check('real-pg-and-sqlite-unchanged-after-junction-scope-drift');
    const originalUnlink = fs.promises.unlink, originalRename = fs.promises.rename;
    let removed = null, failedSaves = 0;
    fs.promises.unlink = async file => {
      await originalUnlink(file);
      if ([source, claimLink].includes(file)) removed = file;
    };
    fs.promises.rename = async (from, to) => {
      if (removed && from === `${plan.statePath}.tmp` && to === plan.statePath && failedSaves < 3) {
        failedSaves++; throw Object.assign(new Error('synthetic progress interruption'), { code: 'EIO' });
      }
      return originalRename(from, to);
    };
    syncBuiltinESMExports();
    try { await assert.rejects(apply(), { code: 'EIO' }); }
    finally { fs.promises.unlink = originalUnlink; fs.promises.rename = originalRename; syncBuiltinESMExports(); }
    assert.equal(failedSaves, 3); assert.ok(removed);
    const progress = JSON.parse(fs.readFileSync(plan.statePath, 'utf8'));
    assert.equal(progress.files.pendingRemoval, removed);
    assert.equal(progress.files.removed.includes(removed), false);
    assert.equal(fs.existsSync(removed), false);
    const receipt = native(env, { action: 'status', projectId: plan.projectId, operationId: plan.operationId }).value;
    assert.equal(receipt.status, 'PURGED');
    assert.equal(sql(env, `SELECT count(*) FROM control.project_registry_projects WHERE project_id='${seeded.targetProjectId}'`), '0');
    const committedPg = rows(env, { includeMaintenance: true }), committedBot = botRows();
    const sqlite = new DatabaseSync(sqlitePath, { readOnly: true });
    try { assert.equal(sqlite.prepare('SELECT count(*) AS n FROM projects WHERE id=?').get(seeded.targetProjectId).n, 1); }
    finally { sqlite.close(); }
    check('real-pg-commit-survives-unlink-and-progress-save-interruption-with-sqlite-rolled-back');
    const result = await apply();
    assert.equal(result.status, 'SCOPED_PURGED');
    assert.deepEqual(rows(env, { includeMaintenance: true }), committedPg);
    assert.deepEqual(botRows(), committedBot);
    assert.equal(fs.existsSync(source), false); assert.equal(fs.existsSync(claimLink), false);
    assert.deepEqual(retainedSnapshot(), retainedBefore);
    check('resume-removes-two-junctions-once-preserving-shared-git-environment-and-retained-knowledge', { retainedDigest: hash(retainedBefore) });
    return result;
  } };
}
