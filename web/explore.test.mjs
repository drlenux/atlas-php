import test from 'node:test';
import assert from 'node:assert/strict';
import { ownerOf, entityLabel, contextView, SEARCH_GROUPS } from './explore.mjs';
import { indexingView } from './model.mjs';

const WRITE = 'write_site:method:admin\\controllers\\WithdrawController::actionCreateWithdraw:write_site:php-struct-v1:4a8e796315194ea0:1';
const BRANCH = 'branch:method:common\\models\\BackgroundGameProcess::processingGameTransaction:php-struct-v1:04cc028bf0c518a3';
const COMMENTED = 'call_site:function:helper:comment:php-struct-v1:9f';

test('ownerOf extracts the owning function from occurrence and branch ids', () => {
  assert.equal(ownerOf(WRITE), 'method:admin\\controllers\\WithdrawController::actionCreateWithdraw');
  assert.equal(ownerOf(BRANCH), 'method:common\\models\\BackgroundGameProcess::processingGameTransaction');
  assert.equal(ownerOf(COMMENTED), 'function:helper');
  assert.equal(ownerOf('method:A::b'), null);
  assert.equal(ownerOf(undefined), null);
});

test('entityLabel shows expressions for occurrences and qualified names for declarations', () => {
  const site = entityLabel({ id: WRITE, kind: 'write_site', name: '$wallet->dep_tax_control = 0', qualified_name: WRITE });
  assert.equal(site.title, '$wallet->dep_tax_control = 0');
  assert.equal(site.subtitle, 'WithdrawController::actionCreateWithdraw');
  assert.equal(site.ownerId, 'method:admin\\controllers\\WithdrawController::actionCreateWithdraw');

  const branch = entityLabel({ id: BRANCH, kind: 'branch', name: 'if', qualified_name: BRANCH, metadata: { keyword: 'if', condition: '$model->type == 1' } });
  assert.equal(branch.title, 'if ($model->type == 1)');
  const elseBranch = entityLabel({ id: BRANCH, kind: 'branch', name: 'else', metadata: { keyword: 'else', condition: '' } });
  assert.equal(elseBranch.title, 'else');

  const method = entityLabel({ id: 'method:A\\B::c', kind: 'method', name: 'c', qualified_name: 'A\\B::c' });
  assert.equal(method.title, 'A\\B::c');
  assert.equal(method.subtitle, '');

  // Usage elements carry `label` instead of `name`.
  const usage = entityLabel({ id: WRITE, kind: 'write_site', label: '$w->x = 1', qualified_name: WRITE });
  assert.equal(usage.title, '$w->x = 1');

  const long = entityLabel({ id: WRITE, kind: 'call_site', name: `$this->render('view', [\n  'model' => 1,\n])` });
  assert.equal(long.title, "$this->render('view', [ 'model' => 1, ])");
});

test('search groups map to valid node kinds', () => {
  for (const group of SEARCH_GROUPS) {
    for (const kind of group.kinds) assert.match(kind, /^[a-z0-9_]+$/);
  }
  assert.deepEqual(SEARCH_GROUPS.find(g => g.id === 'writes').kinds, ['write_site']);
  assert.deepEqual(SEARCH_GROUPS.find(g => g.id === 'all').kinds, []);
});

function node(id, kind, name, metadata = {}) {
  return { id, kind, name, qualified_name: id, span: { path: 'w.php', start_line: 3 }, trust: 1, metadata };
}
function edge(kind, from, to, metadata = {}) {
  return { id: `${kind}:${from}->${to}`, kind, from, to, metadata };
}

test('contextView groups callers, owner, effects and branches around a method', () => {
  const M = 'method:W::pay';
  const context = {
    root_id: M,
    truncated: true,
    diagnostics: [{ code: 'x' }],
    nodes: [
      node('class:W', 'class', 'W'),
      node(M, 'method', 'pay'),
      node('method:G::run', 'method', 'run'),
      node('call_site:method:G::run:call_site:php-struct-v1:1:1', 'call_site', '$w->pay()', { conditions: ['$ok'] }),
      node('write_site:method:W::pay:write_site:php-struct-v1:2:1', 'write_site', '$this->tax = 0', { conditions: ['$a > 0'] }),
      node('read_site:method:W::pay:read_site:php-struct-v1:3:1', 'read_site', '$this->amount'),
      node('call_site:method:W::pay:call_site:php-struct-v1:4:1', 'call_site', 'Log::info()'),
      node('return_site:method:W::pay:return_site:php-struct-v1:5:1', 'return_site', 'return 1'),
      node('branch:method:W::pay:php-struct-v1:6', 'branch', 'if', { keyword: 'if', condition: '$a > 0', conditions: ['$a > 0'] }),
      node('property:W::$tax', 'property', '$tax'),
    ],
    edges: [
      edge('contains', 'class:W', M),
      edge('contains', 'method:G::run', 'call_site:method:G::run:call_site:php-struct-v1:1:1'),
      edge('calls', 'call_site:method:G::run:call_site:php-struct-v1:1:1', M, { resolution: 'resolved' }),
      edge('contains', M, 'write_site:method:W::pay:write_site:php-struct-v1:2:1'),
      edge('writes', 'write_site:method:W::pay:write_site:php-struct-v1:2:1', 'property:W::$tax'),
      edge('contains', M, 'read_site:method:W::pay:read_site:php-struct-v1:3:1'),
      edge('contains', M, 'call_site:method:W::pay:call_site:php-struct-v1:4:1'),
      edge('calls', 'call_site:method:W::pay:call_site:php-struct-v1:4:1', null, { resolution: 'unresolved' }),
      edge('contains', M, 'return_site:method:W::pay:return_site:php-struct-v1:5:1'),
      edge('contains', M, 'branch:method:W::pay:php-struct-v1:6'),
    ],
  };
  const view = contextView(context);
  assert.equal(view.root.id, M);
  assert.deepEqual(view.owners.map(o => o.id), ['class:W']);

  assert.equal(view.incoming.calls.length, 1);
  const caller = view.incoming.calls[0];
  assert.equal(caller.site.title, '$w->pay()');
  assert.equal(caller.owner.id, 'method:G::run');
  assert.deepEqual(caller.conditions, ['$ok']);
  assert.equal(caller.resolution, 'resolved');

  assert.equal(view.effects.writes.length, 1);
  assert.equal(view.effects.writes[0].target.id, 'property:W::$tax');
  assert.deepEqual(view.effects.writes[0].conditions, ['$a > 0']);
  assert.equal(view.effects.reads.length, 1);
  assert.equal(view.effects.reads[0].target, null, 'site without loaded effect edge has no target');
  assert.equal(view.effects.calls.length, 1);
  assert.equal(view.effects.calls[0].resolution, 'unresolved');
  assert.equal(view.effects.returns.length, 1);
  assert.equal(view.branches.length, 1);
  assert.equal(view.branches[0].title, 'if ($a > 0)');
  assert.equal(view.diagnostics, 1);
  assert.equal(view.truncated, true);
});

test('contextView of an occurrence shows its owner and its own effect', () => {
  const S = 'write_site:method:W::pay:write_site:php-struct-v1:2:1';
  const view = contextView({
    root_id: S, truncated: false, diagnostics: [],
    nodes: [node('method:W::pay', 'method', 'pay'), node(S, 'write_site', '$this->tax = 0', { conditions: ['$a'] }), node('property:W::$tax', 'property', '$tax')],
    edges: [edge('contains', 'method:W::pay', S), edge('writes', S, 'property:W::$tax')],
  });
  assert.deepEqual(view.owners.map(o => o.id), ['method:W::pay']);
  assert.equal(view.effects.writes[0].target.id, 'property:W::$tax');
  assert.deepEqual(view.effects.writes[0].conditions, ['$a']);
});

test('contextView of a property lists writers and readers with their owners', () => {
  const P = 'property:W::$tax';
  const S = 'write_site:method:W::pay:write_site:php-struct-v1:2:1';
  const view = contextView({
    root_id: P, truncated: false, diagnostics: [],
    nodes: [node(P, 'property', '$tax'), node(S, 'write_site', '$this->tax = 0', { conditions: ['$a'] }), node('method:W::pay', 'method', 'pay')],
    edges: [edge('writes', S, P), edge('contains', 'method:W::pay', S)],
  });
  assert.equal(view.incoming.writes.length, 1);
  assert.equal(view.incoming.writes[0].owner.id, 'method:W::pay');
});

test('reindex button reflects connection, queue and running state', () => {
  assert.deepEqual(indexingView({ state: 'completed' }).reindex, { enabled: true, label: 'Переиндексировать' });
  assert.deepEqual(indexingView({ state: 'waiting' }).reindex, { enabled: true, label: 'Переиндексировать' });
  assert.deepEqual(indexingView({ state: 'completed', reindex_queued: true }).reindex, { enabled: false, label: 'Переиндексация в очереди' });
  assert.deepEqual(indexingView({ state: 'running', stage: 'analysis' }).reindex, { enabled: true, label: 'Переиндексировать после текущего прогона' });
  assert.equal(indexingView({ state: 'completed' }, false).reindex.enabled, false);
  assert.match(indexingView({ state: 'waiting' }).count, /кнопк|изменении файлов/);
});

test('contextView groups queue dispatches in both directions', () => {
  const E = 'method:Job::execute';
  const S = 'call_site:method:P::run:call_site:php-struct-v1:1:1';
  const view = contextView({
    root_id: E, truncated: false, diagnostics: [],
    nodes: [node(E, 'method', 'execute'), node(S, 'call_site', '$q->push(new Job())'), node('method:P::run', 'method', 'run')],
    edges: [edge('dispatches', S, E, { resolution: 'annotated', queue: 'queueGame' }), edge('contains', 'method:P::run', S)],
  });
  assert.equal(view.incoming.dispatches.length, 1);
  assert.equal(view.incoming.dispatches[0].owner.id, 'method:P::run');
  assert.equal(view.incoming.dispatches[0].queue, 'queueGame');
});
