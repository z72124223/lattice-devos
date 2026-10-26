import test from 'node:test';
import assert from 'node:assert/strict';
import { Checklist } from './functional-checklist.mjs';
test('新增、完成和重新開啟保留相同工作', () => {
 const list = new Checklist(); list.add('buy-milk', ' 買牛奶 '); list.add('walk', '散步'); list.complete('buy-milk');
 const restored = new Checklist(list.save());
 assert.deepEqual(restored.list(), [{id:'buy-milk',title:'買牛奶',done:true},{id:'walk',title:'散步',done:false}]);
});
test('重複工作與無效操作不改動現有資料', () => {
 const list = new Checklist(); list.add('one', '第一件工作'); const saved = list.save();
 assert.throws(() => list.add('one', '同一件工作'), /Duplicate ID/);
 assert.throws(() => list.complete('missing'), /Unknown ID/);
 assert.throws(() => list.add('two', ' '), /required/);
 assert.equal(list.save(), saved);
});
test('讀取結果不會意外修改內部資料，損坏存檔會被拒絕', () => {
 const list = new Checklist(); list.add('one', '第一件工作'); list.list()[0].done=true;
 assert.equal(list.list()[0].done, false);
 assert.throws(() => new Checklist('[{"id":"one","title":"A","done":"yes"}]'), /Invalid checklist/);
 assert.throws(() => new Checklist('[{"id":"one","title":"A","done":true},{"id":"one","title":"B","done":false}]'), /Invalid checklist/);
});