import test from 'node:test';
import assert from 'node:assert/strict';
import { createIndexingPoller } from './indexing.mjs';

function scheduler() {
  const tasks = new Map();
  let sequence = 0;
  return {
    tasks,
    schedule(fn, delay) { const id = ++sequence; tasks.set(id, { fn, delay }); return id; },
    cancel(id) { tasks.delete(id); },
    run(delay) {
      const [id, task] = [...tasks].find(([, task]) => task.delay === delay);
      tasks.delete(id);
      return task.fn();
    },
  };
}

test('polling waits for completion, recovers from failure and follows server cadence', async () => {
  const timers = scheduler();
  const updates = [];
  let resolve;
  let calls = 0;
  const poller = createIndexingPoller({ ...timers,
    request: () => { calls++; return new Promise(done => { resolve = done; }); },
    update: (data, error) => updates.push({ data, error }),
  });
  const first = poller.start();
  assert.equal(calls, 1);
  assert.deepEqual([...timers.tasks.values()].map(t => t.delay), [10000], 'only timeout exists while request is pending');
  resolve({ state: 'running', poll_interval_ms: 1700 });
  await first;
  assert.equal(updates[0].data.state, 'running');
  const second = timers.run(1700);
  assert.equal(calls, 2);
  resolve({ state: 'completed', poll_interval_ms: 1700 });
  await second;
  poller.stop();
  assert.equal(timers.tasks.size, 0);

  let attempts = 0;
  const retry = createIndexingPoller({ ...timers,
    request: async () => { if (++attempts === 1) throw new Error('offline'); return { state: 'running' }; },
    update: (data, error) => updates.push({ data, error }),
  });
  await retry.start();
  assert.equal(updates.at(-1).error.message, 'offline');
  await timers.run(1000);
  assert.equal(updates.at(-1).data.state, 'running');
  retry.stop();
});

test('timeout aborts the request and reconnect ignores responses from an old token', async () => {
  const timers = scheduler();
  const updates = [];
  const poller = createIndexingPoller({ ...timers,
    request: signal => new Promise((resolve, reject) => signal.addEventListener('abort', () => reject(new Error('timeout')))),
    update: (data, error) => updates.push({ data, error }),
  });
  const pending = poller.start();
  timers.run(10000);
  await pending;
  assert.equal(updates[0].error.message, 'timeout');
  poller.stop();

  const resolves = [];
  const reconnect = createIndexingPoller({ ...timers,
    request: () => new Promise(resolve => resolves.push(resolve)),
    update: data => updates.push(data),
  });
  const old = reconnect.start();
  const current = reconnect.start();
  resolves[1]({ state: 'running' });
  await current;
  const count = updates.length;
  resolves[0]({ state: 'completed' });
  await old;
  assert.equal(updates.length, count);
  reconnect.stop();
  assert.equal(timers.tasks.size, 0);
});

test('rejected credentials stop polling until a new connection', async () => {
  const timers = scheduler();
  const poller = createIndexingPoller({ ...timers,
    request: async () => { throw Object.assign(new Error('unauthorized'), { status: 401 }); },
    update: () => {},
  });
  await poller.start();
  assert.equal(timers.tasks.size, 0);
});
