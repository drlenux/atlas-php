import test from 'node:test';
import assert from 'node:assert/strict';
import { periodFromDates, timelinePoints, rankElements, indexingView } from './model.mjs';

test('indexing shows stage progress and labels ETA as a stage estimate', () => {
  const view = indexingView({ state: 'running', stage: 'analysis', detail: 'php_flow', completed: 3, total: 12, unit: 'functions', current_path: 'src/A.php', elapsed_ms: 65000, eta_seconds: 90 });
  assert.equal(view.percent, 25);
  assert.match(view.title, /функций PHP|Анализ/);
  assert.match(view.count, /3.*12/);
  assert.match(view.eta, /этапа.*1 мин 30 с/);
  assert.equal(view.path, 'src/A.php');
  assert.equal(view.elapsed, '1 мин 5 с');
  assert.match(view.stage, /из 5/);
});

test('publishing progress exposes scoped file totals beside item counts', () => {
  const view = indexingView({
    state: 'running',
    stage: 'publishing',
    detail: 'prepare_edges',
    completed: 100,
    total: 400,
    unit: 'items',
    files_completed: 12,
    files_total: 80,
  });
  assert.match(view.count, /100.*400/);
  assert.match(view.count, /файлов 12.*80/);
});

test('parallel indexing exposes busy and available worker counts', () => {
  assert.equal(indexingView({ state: 'running', stage: 'analysis', workers: 4, active_workers: 3 }).workers, 'Потоки: 3 из 4 заняты');
  assert.equal(indexingView({ state: 'running', stage: 'analysis', workers: 1, active_workers: 0 }).workers, '1 поток');
  assert.equal(indexingView({ state: 'completed', workers: 4 }).workers, '');
  assert.equal(indexingView({ state: 'running', workers: 4 }, false).workers, '');
});

test('unknown work, failures and lost connection never claim completion or a live ETA', () => {
  const running = { state: 'running', stage: 'publishing', completed: 0, total: null, elapsed_ms: 5000, eta_seconds: null };
  assert.equal(indexingView(running).percent, null);
  assert.match(indexingView(running).eta, /оценивается/);
  const failed = indexingView({ ...running, state: 'failed', error: 'storage unavailable', completed: 10, total: 10, eta_seconds: 0 });
  assert.equal(failed.percent, null);
  assert.match(failed.title, /Ошибка/);
  assert.equal(failed.error, 'storage unavailable');
  const offline = indexingView(running, false);
  assert.match(offline.title, /связ/);
  assert.equal(offline.percent, null);
  assert.equal(offline.eta, '');
  assert.equal(indexingView({ state: 'completed', elapsed_ms: 5000 }).percent, 100);
  assert.equal(indexingView({ state: 'waiting' }).percent, null);
});

test('inclusive calendar dates map to a half-open UTC analytics period', () => {
  const period = periodFromDates('2026-11-01', '2026-11-02');
  assert.equal(new Date(period.from * 1000).toISOString(), '2026-11-01T00:00:00.000Z');
  assert.equal(new Date(period.to * 1000).toISOString(), '2026-11-03T00:00:00.000Z');
});

test('invalid or reversed dates never reach the analytics API', () => {
  assert.throws(() => periodFromDates('2026-02-30', '2026-11-02'));
  assert.throws(() => periodFromDates('2026-11-03', '2026-11-02'));
  assert.throws(() => periodFromDates('', '2026-11-02'));
});

test('timeline preserves empty days between actual deliveries', () => {
  const { from, to } = periodFromDates('2026-11-01', '2026-11-03');
  const points = timelinePoints({ from, to, bucket_seconds: 86400, timeline: [
    { start: from, responses: 2 }, { start: from + 172800, responses: 5 },
  ] });
  assert.deepEqual(points.map(p => p.responses), [2, 0, 5]);
});

test('rank filtering preserves identities of equally named graph elements', () => {
  const elements = [
    { id: 'b', target_kind: 'node', label: 'pay', qualified_name: 'B::pay', count: 2 },
    { id: 'edge', target_kind: 'edge', label: 'calls', count: 10 },
    { id: 'a', target_kind: 'node', label: 'pay', qualified_name: 'A::pay', count: 4 },
  ];
  assert.deepEqual(rankElements(elements, 'node', '').map(e => e.id), ['a', 'b']);
  assert.deepEqual(rankElements(elements, 'all', 'B::pay').map(e => e.id), ['b']);
  assert.equal(elements[0].id, 'b');
});
