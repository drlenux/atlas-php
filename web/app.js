import { periodFromDates, timelinePoints, rankElements, indexingView } from './model.mjs';
import { createIndexingPoller } from './indexing.mjs';
import { SEARCH_GROUPS, entityLabel, contextView, isOccurrence } from './explore.mjs';

const $ = (id) => document.getElementById(id);
const state = { token: '', analytics: null, offset: 0, limit: 20, generation: 0, visible: 100, search: null, history: [] };
const number = new Intl.NumberFormat('ru-RU');
const date = new Intl.DateTimeFormat('ru-RU', { day: 'numeric', month: 'short', timeZone: 'UTC' });

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

function notice(message, success = false) {
  $('notice').textContent = message;
  $('notice').classList.toggle('success', success);
  $('notice').hidden = !message;
}

async function api(path, options = {}) {
  if (!state.token) throw new Error('Введите токен разработчика и подключитесь.');
  const response = await fetch(path, {
    ...options,
    headers: { authorization: `Bearer ${state.token}`, 'content-type': 'application/json', ...options.headers },
    cache: 'no-store',
  });
  const body = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (response.status === 401) throw Object.assign(new Error('Токен не принят. Используйте токен разработчика для веб-интерфейса.'), { status: 401 });
    if (response.status === 409) throw new Error('Сведения уже изменились. Обновите список перед повторным решением.');
    throw new Error(body.error?.message || body.error || `Запрос не выполнен (${response.status}).`);
  }
  return body;
}

function connection(ready) {
  $('connection').classList.toggle('ready', ready);
  $('connection').replaceChildren(element('span'), document.createTextNode(ready ? 'Подключено к атласу' : 'Ожидание подключения'));
}

let lastIndexing;
const indexingPoller = createIndexingPoller({
  request: signal => api('/api/indexing', { signal }),
  update: (snapshot, error) => {
    if (snapshot) lastIndexing = snapshot;
    const view = indexingView(lastIndexing, !error);
    $('indexing-title').textContent = view.title;
    $('indexing-stage').textContent = view.stage;
    $('indexing-count').textContent = view.count;
    $('indexing').classList.toggle('failed', view.failed);
    const bar = $('indexing-progress');
    bar.hidden = !view.active && view.percent === null;
    if (view.percent === null) bar.removeAttribute('value');
    else bar.value = view.percent;
    bar.setAttribute('aria-label', snapshot?.state === 'completed' ? 'Индексация завершена' : 'Прогресс текущего этапа индексации');
    $('indexing-elapsed').textContent = lastIndexing && lastIndexing.state !== 'waiting' && !error ? `Прошло ${view.elapsed}` : '';
    $('indexing-eta').textContent = view.eta;
    $('indexing-workers').textContent = view.workers;
    $('indexing-path').textContent = view.path;
    $('indexing-path').hidden = !view.path;
    const message = error ? error.status === 401 ? error.message : 'Сервер не отвечает. Подключение восстановится автоматически.' : view.error;
    $('indexing-error').textContent = message;
    $('indexing-error').hidden = !message;
    $('reindex').disabled = !view.reindex.enabled;
    $('reset-atlas').disabled = Boolean(error);
    $('reindex').textContent = view.reindex.label;
    connection(!error);
  },
});
window.addEventListener('pagehide', () => indexingPoller.stop());
window.addEventListener('pageshow', event => { if (event.persisted && state.token) indexingPoller.start(); });

function setMetrics(analytics) {
  $('metric-responses').textContent = number.format(analytics.total_responses);
  $('metric-nodes').textContent = number.format(analytics.unique_nodes);
  $('metric-edges').textContent = number.format(analytics.unique_edges);
  $('period-label').textContent = `${date.format(analytics.from * 1000)} — ${date.format((analytics.to - 1) * 1000)} · UTC`;
}

function renderTimeline(analytics) {
  const points = timelinePoints(analytics);
  const svgNode = (tag, attributes, text) => {
    const node = document.createElementNS('http://www.w3.org/2000/svg', tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const svg = svgNode('svg', { viewBox: '0 0 1000 200', role: 'img', 'aria-label': `Динамика выдачи: ${analytics.total_responses} ответов за выбранный период` });
  const max = Math.max(1, ...points.map(p => p.responses));
  for (let row = 0; row <= 3; row++) {
    const y = 22 + row * 45;
    svg.append(svgNode('line', { x1: 42, x2: 986, y1: y, y2: y, stroke: '#e9eee8', 'stroke-dasharray': row === 3 ? '0' : '3 5' }));
    svg.append(svgNode('text', { x: 28, y: y + 4, 'text-anchor': 'end', fill: '#91a094', 'font-size': 10 }, number.format(Math.round(max * (3 - row) / 3))));
  }
  const slot = 930 / Math.max(points.length, 1);
  points.forEach((point, index) => {
    const height = 132 * point.responses / max;
    const bar = svgNode('rect', { x: 49 + index * slot + slot * .2, y: 157 - height, width: Math.max(1, slot * .6), height: Math.max(1, height), fill: point.responses ? '#60967b' : '#e5ece4', rx: Math.min(3, slot * .15) });
    bar.append(svgNode('title', {}, `${date.format(point.start * 1000)}: ${number.format(point.responses)} ответов`));
    svg.append(bar);
    const interval = Math.max(1, Math.ceil(points.length / 8));
    if (index % interval === 0 || index === points.length - 1) svg.append(svgNode('text', { x: 49 + index * slot + slot * .5, y: 185, 'text-anchor': 'middle', fill: '#8b978f', 'font-size': 10 }, date.format(point.start * 1000)));
  });
  $('timeline').replaceChildren(svg);
}

function renderElements() {
  if (!state.analytics) return;
  const elements = rankElements(state.analytics.elements, $('kind-filter').value, $('element-filter').value);
  const rows = elements.slice(0, state.visible).map(item => {
    const row = element('tr');
    const label = element('td');
    const name = item.target_kind === 'edge' ? { title: item.qualified_name || item.label || item.id, subtitle: '' } : entityLabel(item);
    const title = element('span', name.title, `object-name${isOccurrence(item.kind) ? ' code' : ''}`);
    title.title = item.id;
    label.append(title, element('small', name.subtitle || item.id));
    const kind = element('td');
    kind.append(element('span', `${item.target_kind === 'edge' ? 'Связь' : 'Вершина'} · ${item.kind}`, `type-pill ${item.target_kind === 'edge' ? 'edge' : ''}`));
    const context = element('td', item.path || '—');
    if (item.target_kind === 'edge') context.append(element('small', `${item.from || '?'} → ${item.to || 'цель не установлена'}`));
    const count = element('td', number.format(item.count), 'number');
    const track = element('span', undefined, 'frequency-track');
    const fill = element('i');
    fill.style.width = `${100 * item.count / Math.max(elements[0]?.count || 1, 1)}%`;
    track.append(fill);
    count.append(track);
    row.append(label, kind, context, count);
    return row;
  });
  if (!rows.length) {
    const row = element('tr');
    const cell = element('td', state.analytics.total_responses ? 'Нет элементов по выбранному фильтру.' : 'За выбранный период выдач контекста нет.', 'empty');
    cell.colSpan = 4;
    row.append(cell);
    rows.push(row);
  }
  $('elements').replaceChildren(...rows);
  $('element-count').replaceChildren(document.createTextNode(`${number.format(Math.min(elements.length, state.visible))} из ${number.format(elements.length)} элементов`));
  if (elements.length > state.visible) {
    const button = element('button', 'Показать ещё', 'button secondary');
    button.addEventListener('click', () => { state.visible += 100; renderElements(); });
    $('element-count').append(document.createTextNode(' '), button);
  }
}

async function loadAnalytics() {
  const period = periodFromDates($('date-from').value, $('date-to').value);
  const generation = ++state.generation;
  const button = $('period-form').querySelector('button');
  button.disabled = true;
  button.textContent = 'Загрузка…';
  try {
    const analytics = await api(`/api/analytics?from=${period.from}&to=${period.to}`);
    if (generation !== state.generation) return;
    state.analytics = analytics;
    state.visible = 100;
    setMetrics(analytics);
    renderTimeline(analytics);
    renderElements();
  } finally {
    if (generation === state.generation) { button.disabled = false; button.textContent = 'Обновить'; }
  }
}

function reviewCard(correction) {
  const card = element('article', undefined, 'card review-card');
  const head = element('div', undefined, 'review-head');
  head.append(element('h3', correction.target_id), element('span', `${correction.target_kind === 'edge' ? 'Связь' : 'Вершина'} · уровень 2`, 'type-pill'));
  const info = element('div', undefined, 'review-info');
  info.append(element('strong', 'Причина: '), document.createTextNode(correction.reason || 'Не указана'));
  const diff = element('div', undefined, 'diff');
  for (const [label, value] of [[`До исправления · уровень ${correction.before_trust}`, correction.before], ['Исправление агента · уровень 2', correction.after]]) {
    const section = element('div');
    section.append(element('h4', label), element('pre', JSON.stringify(value, null, 2)));
    diff.append(section);
  }
  const actions = element('div', undefined, 'review-actions');
  for (const [decision, title, style] of [['approve', 'Подтвердить → уровень 3', 'primary'], ['decline', 'Отменить исправление', 'danger']]) {
    const button = element('button', title, `button ${style}`);
    button.addEventListener('click', async () => {
      actions.querySelectorAll('button').forEach(b => b.disabled = true);
      try {
        await api(`/api/reviews/${encodeURIComponent(correction.id)}/decision`, { method: 'POST', body: JSON.stringify({ decision, expected_revision: correction.revision }) });
        notice(decision === 'approve' ? 'Исправление подтверждено. Сведения получили уровень 3.' : 'Исправление отменено. Предшествующие сведения восстановлены.', true);
        await loadReviews();
      } catch (error) { notice(error.message); actions.querySelectorAll('button').forEach(b => b.disabled = false); }
    });
    actions.append(button);
  }
  actions.append(element('span', `Ревизия ${correction.revision} · ${date.format(correction.created_at * 1000)}`));
  card.append(head, info, diff, actions);
  return card;
}

async function loadReviews() {
  const result = await api(`/api/reviews?offset=${state.offset}&limit=${state.limit}`);
  $('reviews').replaceChildren(...(result.items.length ? result.items.map(reviewCard) : [element('div', 'В этой выборке нет исправлений, ожидающих проверки.', 'card empty')]));
  $('pending-badge').textContent = `${state.offset + result.items.length}${result.has_more ? '+' : ''}`;
  $('reviews-page').textContent = result.items.length ? `${state.offset + 1}–${state.offset + result.items.length}` : 'Нет исправлений';
  $('reviews-prev').disabled = state.offset === 0;
  $('reviews-next').disabled = !result.has_more;
}

async function guarded(action) {
  notice('');
  try { await action(); } catch (error) { notice(error.message); }
}

$('connect-form').addEventListener('submit', event => {
  event.preventDefault();
  state.token = $('token').value.trim();
  lastIndexing = undefined;
  indexingPoller.start();
  guarded(async () => {
    connection(false);
    await Promise.all([loadAnalytics(), loadReviews()]);
    connection(true);
    $('token').value = '';
    $('token').placeholder = 'Подключение установлено';
  });
});
$('period-form').addEventListener('submit', event => { event.preventDefault(); guarded(loadAnalytics); });
$('kind-filter').addEventListener('change', () => { state.visible = 100; renderElements(); });
$('element-filter').addEventListener('input', () => { state.visible = 100; renderElements(); });
$('refresh-reviews').addEventListener('click', () => guarded(loadReviews));
$('reviews-next').addEventListener('click', () => { state.offset += state.limit; guarded(loadReviews); });
$('reviews-prev').addEventListener('click', () => { state.offset = Math.max(0, state.offset - state.limit); guarded(loadReviews); });
document.querySelectorAll('.tab').forEach(tab => tab.addEventListener('click', () => {
  document.querySelectorAll('.tab').forEach(t => { t.classList.toggle('active', t === tab); t.setAttribute('aria-selected', String(t === tab)); });
  document.querySelectorAll('.panel').forEach(panel => panel.hidden = panel.id !== `panel-${tab.dataset.panel}`);
}));
$('reindex').addEventListener('click', () => guarded(async () => {
  $('reindex').disabled = true;
  await api('/api/reindex', { method: 'POST' });
  notice('Переиндексация поставлена в очередь. Прогресс отображается в карточке индексации.', true);
}));

const RESET_WARNING = 'Удалить весь атлас?\n\nБудут удалены граф, диагностики, все исправления агента и подтверждения разработчика, аналитика выдач. Действие необратимо.\n\nСразу после удаления начнётся полная переиндексация.';

$('reset-atlas').addEventListener('click', () => guarded(async () => {
  if (!window.confirm(RESET_WARNING)) return;
  $('reset-atlas').disabled = true;
  try {
    await api('/api/atlas/reset', { method: 'POST', body: JSON.stringify({ confirm: 'reset' }) });
  } finally {
    $('reset-atlas').disabled = false;
  }
  state.search = null;
  state.history = [];
  state.contextId = null;
  $('search-results').replaceChildren();
  $('search-summary').textContent = '';
  $('search-more').hidden = true;
  $('context-output').replaceChildren(element('h3', 'Контекст сущности'), element('p', 'Выберите результат поиска.', 'empty'));
  await Promise.all([loadAnalytics(), loadReviews()]);
  notice('Атлас удалён. Полная переиндексация поставлена в очередь, прогресс отображается в карточке индексации.', true);
}));

for (const group of SEARCH_GROUPS) {
  const option = element('option', group.label);
  option.value = group.id;
  $('search-kind').append(option);
}

function conditionList(conditions, limit = 4) {
  if (!conditions?.length) return null;
  const box = element('div', undefined, 'conditions');
  const hidden = Math.max(0, conditions.length - limit);
  if (hidden) box.append(element('span', `…ещё ${hidden} внешн. ${hidden === 1 ? 'условие' : hidden < 5 ? 'условия' : 'условий'}`, 'more'));
  const list = element('ol');
  list.start = hidden + 1;
  list.setAttribute('aria-label', 'Условия пути, от внешнего к ближайшему');
  for (const condition of conditions.slice(hidden)) list.append(element('li', condition.replace(/\s+/g, ' ')));
  box.append(list);
  return box;
}

function entityLink(ref, className = 'entity-link') {
  if (!ref) return element('span', 'цель не распознана', 'unresolved');
  const link = element('button', ref.title, className);
  link.type = 'button';
  link.title = ref.id;
  link.addEventListener('click', () => guarded(() => openContext(ref.id, true)));
  return link;
}

function searchResult(node) {
  const label = entityLabel(node);
  const button = element('button', undefined, 'search-result');
  button.type = 'button';
  button.title = node.id;
  button.append(element('span', label.title, `result-title${isOccurrence(node.kind) ? ' code' : ''}`));
  const meta = element('small');
  meta.append(element('span', node.kind, 'type-pill'), document.createTextNode(` ${label.subtitle ? `${label.subtitle} · ` : ''}${node.span.path}:${node.span.start_line} · доверие ${node.trust}`));
  button.append(meta);
  const conditions = conditionList(node.metadata?.conditions, 2);
  if (conditions) button.append(conditions);
  button.addEventListener('click', () => guarded(() => openContext(node.id, false)));
  return button;
}

async function loadSearchPage() {
  const search = state.search;
  const params = new URLSearchParams({ q: search.query, limit: '50' });
  if (search.kinds.length) params.set('kinds', search.kinds.join(','));
  if (search.cursor) params.set('cursor', search.cursor);
  if (search.symbol) params.set('mode', 'symbol');
  const page = await api(`/api/search?${params}`);
  if (search !== state.search) return;
  search.count += page.nodes.length;
  search.cursor = page.next_cursor;
  if (page.nodes.length) {
    $('search-results').append(...page.nodes.map(searchResult));
  } else if (!search.count) {
    $('search-results').replaceChildren(element('p', 'По вашему запросу сущности не найдены.', 'empty'));
  }
  $('search-summary').textContent = search.count ? `Показано ${number.format(search.count)}${search.cursor ? ', есть ещё' : ''}` : '';
  $('search-more').hidden = !search.cursor;
}

$('search-form').addEventListener('submit', event => {
  event.preventDefault();
  const group = SEARCH_GROUPS.find(g => g.id === $('search-kind').value) || SEARCH_GROUPS[0];
  state.search = { query: $('search-query').value, kinds: group.kinds, cursor: null, count: 0, symbol: $('search-symbol').checked };
  $('search-results').replaceChildren();
  $('search-summary').textContent = 'Поиск…';
  $('search-more').hidden = true;
  guarded(loadSearchPage);
});
$('search-more').addEventListener('click', () => guarded(async () => {
  $('search-more').disabled = true;
  try { await loadSearchPage(); } finally { $('search-more').disabled = false; }
}));

function effectItem(item, { showOwner = false, showTarget = false } = {}) {
  const row = element('li', undefined, 'effect');
  const head = element('div', undefined, 'effect-head');
  head.append(entityLink(item.site, 'entity-link code'));
  if (showTarget && item.target) {
    head.append(element('span', '→', 'arrow'), entityLink(item.target));
  }
  if (showOwner && item.owner) {
    head.append(element('span', 'в', 'arrow'), entityLink(item.owner));
  }
  if (item.resolution === 'unresolved' || item.resolution === 'ambiguous') head.append(element('span', item.resolution === 'ambiguous' ? 'неоднозначно' : 'не распознано', 'unresolved'));
  if (item.resolution === 'annotated') {
    const badge = element('span', 'по аннотации', 'annotated');
    badge.title = 'Цель найдена через тип из PHPDoc (@var, @param, @return, @property). Аннотация может быть неверной.';
    head.append(badge);
  }
  if (item.resolution === 'candidate') head.append(element('span', 'несколько реализаций', 'annotated'));
  if (item.queue) head.append(element('span', `очередь ${item.queue}`, 'type-pill'));
  row.append(head);
  const where = element('small', item.site?.location || '');
  row.append(where);
  const conditions = conditionList(item.conditions);
  if (conditions) row.append(conditions);
  return row;
}

const OPEN_SECTION_LIMIT = 10;

function section(title, items, render) {
  if (!items.length) return null;
  // Long groups (e.g. dozens of reads) start collapsed so callers stay visible.
  const block = element('details', undefined, 'context-section');
  block.open = items.length <= OPEN_SECTION_LIMIT;
  const summary = element('summary');
  summary.append(element('h4', `${title} · ${number.format(items.length)}`));
  block.append(summary);
  const list = element('ul', undefined, 'effects');
  list.append(...items.map(render));
  block.append(list);
  return block;
}

function renderContext(context) {
  const view = contextView(context);
  const output = $('context-output');
  const head = element('header', undefined, 'context-head');
  if (state.history.length) {
    const back = element('button', '← Назад', 'button secondary back');
    back.type = 'button';
    back.addEventListener('click', () => guarded(() => openContext(state.history.pop(), false, true)));
    head.append(back);
  }
  head.append(element('span', view.root.kind, 'type-pill'));
  const title = element('h3', view.root.title, isOccurrence(view.root.kind) ? 'code' : '');
  title.title = view.root.id;
  head.append(title);
  head.append(element('small', [view.root.subtitle, view.root.location, `доверие ${view.root.trust}`].filter(Boolean).join(' · ')));
  const rootConditions = conditionList(view.root.metadata?.conditions, 12);
  const parts = [head];
  if (rootConditions) parts.push(rootConditions);
  const owners = section('Владелец', view.owners, ref => { const li = element('li', undefined, 'effect'); li.append(entityLink(ref), element('small', ref.location)); return li; });
  parts.push(
    owners,
    section('Вызывают', view.incoming.calls, item => effectItem(item, { showOwner: true })),
    section('Пишут', view.incoming.writes, item => effectItem(item, { showOwner: true })),
    section('Читают', view.incoming.reads, item => effectItem(item, { showOwner: true })),
    section('Ставят в очередь', view.incoming.dispatches, item => effectItem(item, { showOwner: true })),
    section('Вызывает', view.effects.calls, item => effectItem(item, { showTarget: true })),
    section('Пишет', view.effects.writes, item => effectItem(item, { showTarget: true })),
    section('Читает', view.effects.reads, item => effectItem(item, { showTarget: true })),
    section('Ставит в очередь', view.effects.dispatches, item => effectItem(item, { showTarget: true })),
    section('Возвращает', view.effects.returns, item => effectItem(item)),
    section('Ветки', view.branches, ref => { const li = element('li', undefined, 'effect'); li.append(entityLink(ref, 'entity-link code'), element('small', ref.location)); return li; }),
    section('Прочие связи', view.other, item => { const li = element('li', undefined, 'effect'); li.append(element('span', `${item.outgoing ? '→' : '←'} ${item.kind} `, 'arrow'), entityLink(item.node)); return li; }),
  );
  if (view.truncated) parts.push(element('p', 'Показан ближайший участок графа: часть связей не поместилась в пределы ответа. Откройте связанную сущность, чтобы продолжить.', 'context-note'));
  if (context.diagnostics?.length) {
    const details = element('details', undefined, 'context-details');
    details.append(element('summary', `Диагностика анализатора в затронутых файлах · ${number.format(context.diagnostics.length)}`));
    const list = element('ul', undefined, 'diagnostics');
    list.append(...context.diagnostics.slice(0, 200).map(d => element('li', `${d.path}:${d.line} · ${d.code} · ${d.message}`)));
    details.append(list);
    parts.push(details);
  }
  const raw = element('details', undefined, 'context-details');
  raw.append(element('summary', 'Полные данные вершины (JSON)'));
  const pre = element('pre', 'Загрузка…');
  raw.addEventListener('toggle', () => {
    if (!raw.open || raw.dataset.loaded) return;
    raw.dataset.loaded = '1';
    guarded(async () => {
      const full = await api(`/api/context/${encodeURIComponent(view.root.id)}?depth=0&detail=full&include_diagnostics=false`);
      pre.textContent = JSON.stringify(full.nodes.find(n => n.id === view.root.id) || full, null, 2);
    });
  });
  raw.append(pre);
  parts.push(raw);
  output.replaceChildren(...parts.filter(Boolean));
}

async function openContext(id, remember, fromHistory = false) {
  const current = state.contextId;
  const context = await api(`/api/context/${encodeURIComponent(id)}?depth=1&detail=summary&include_diagnostics=true`);
  if (remember && current && current !== id) state.history.push(current);
  if (!remember && !fromHistory) state.history = [];
  state.contextId = id;
  renderContext(context);
  if (window.matchMedia('(max-width: 900px)').matches) $('context-output').scrollIntoView({ behavior: 'smooth', block: 'start' });
}
const today = new Date();
$('date-to').value = today.toISOString().slice(0, 10);
$('date-from').value = new Date(today.getTime() - 6 * 86400000).toISOString().slice(0, 10);
