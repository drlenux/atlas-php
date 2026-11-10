// One request at a time; connection changes invalidate outstanding responses.
export function createIndexingPoller({ request, update, schedule = setTimeout, cancel = clearTimeout }) {
  let generation = 0;
  let next;
  let active;
  function stop() {
    generation++;
    cancel(next);
    active?.abort();
    active = null;
  }
  async function poll(current) {
    const controller = new AbortController();
    active = controller;
    const timeout = schedule(() => controller.abort(), 10000);
    let interval = 1000;
    let retry = true;
    try {
      const data = await request(controller.signal);
      if (current !== generation) return;
      interval = Math.max(250, Number(data.poll_interval_ms) || 1000);
      update(data, null);
    } catch (error) {
      if (current !== generation) return;
      retry = error.status !== 401;
      update(null, error);
    } finally {
      cancel(timeout);
      if (active === controller) active = null;
      if (current === generation && retry) next = schedule(() => poll(current), interval);
    }
  }
  return { stop, start() { stop(); return poll(generation); } };
}
