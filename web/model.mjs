const DAY = 86400;

function duration(milliseconds = 0) {
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  if (seconds >= 3600) return `${Math.floor(seconds / 3600)} ч ${Math.floor(seconds % 3600 / 60)} мин`;
  if (seconds >= 60) return `${Math.floor(seconds / 60)} мин ${seconds % 60} с`;
  return `${seconds} с`;
}

export function indexingView(snapshot = { state: 'waiting' }, online = true) {
  const stages = {
    discovery: [1, 'Поиск файлов'], reading: [2, 'Чтение исходников'],
    analysis: [3, 'Анализ кода'],
    validation: [4, 'Проверка изменений'], publishing: [5, 'Сохранение индекса'],
  };
  const details = {
    php_parse: 'Разбор PHP', php_graph: 'Построение связей PHP',
    php_flow: 'Анализ функций PHP', php_finalize: 'Завершение анализа PHP',
    yii2: 'Анализ Yii2',
  };
  const [step, label] = stages[snapshot.stage] || [0, 'Подготовка'];
  const active = snapshot.state === 'running';
  const complete = snapshot.state === 'completed';
  const failed = snapshot.state === 'failed';
  const known = Number.isFinite(snapshot.total) && snapshot.total > 0;
  const percent = !online || failed ? null : complete ? 100 : active && known
    ? Math.min(100, Math.max(0, Math.floor(100 * snapshot.completed / snapshot.total))) : null;
  const unit = { files: 'файлов', functions: 'функций', items: 'элементов', paths: 'файлов' }[snapshot.unit] || 'элементов';
  const filesKnown = Number.isFinite(snapshot.files_total) && snapshot.files_total > 0;
  const files = filesKnown
    ? ` · файлов ${Number(snapshot.files_completed || 0).toLocaleString('ru-RU')} / ${Number(snapshot.files_total).toLocaleString('ru-RU')}`
    : '';
  return {
    title: !online ? 'Нет связи с индексатором' : failed ? 'Ошибка индексации' : complete ? 'Индекс обновлён'
      : active ? details[snapshot.detail] || label : 'Ожидание индексации',
    stage: online && active ? `Этап ${step} из 5 · ${label}` : '',
    percent,
    count: !online || failed ? '' : complete ? 'Индексация завершена'
      : active && known ? `${snapshot.completed.toLocaleString('ru-RU')} / ${snapshot.total.toLocaleString('ru-RU')} ${unit}${files} · ${percent}% этапа`
      : active && filesKnown ? `Выполняется…${files}`
      : active ? 'Выполняется…' : 'Запустится автоматически при изменении файлов или по кнопке',
    elapsed: duration(snapshot.elapsed_ms),
    workers: !online || !active ? '' : snapshot.workers > 1
      ? `Потоки: ${snapshot.active_workers || 0} из ${snapshot.workers} заняты` : '1 поток',
    eta: !online || !active ? '' : Number.isFinite(snapshot.eta_seconds)
      ? `До конца этапа ≈ ${duration(snapshot.eta_seconds * 1000)}` : 'Время до конца этапа пока не оценивается',
    path: online && (active || failed) ? snapshot.current_path || '' : '',
    error: online && failed ? snapshot.error || 'Индексация прервана' : '',
    active: online && active,
    failed: failed || !online,
    reindex: !online ? { enabled: false, label: 'Переиндексировать' }
      : snapshot.reindex_queued ? { enabled: false, label: 'Переиндексация в очереди' }
      : active ? { enabled: true, label: 'Переиндексировать после текущего прогона' }
      : { enabled: true, label: 'Переиндексировать' },
  };
}

export function periodFromDates(start, end) {
  const parse = (value) => {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) throw new Error('Укажите обе даты периода.');
    const date = new Date(`${value}T00:00:00Z`);
    if (!Number.isFinite(date.getTime()) || date.toISOString().slice(0, 10) !== value) {
      throw new Error('Укажите существующую календарную дату.');
    }
    return date.getTime() / 1000;
  };
  const from = parse(start);
  const to = parse(end) + DAY;
  if (from >= to) throw new Error('Дата начала должна быть не позже даты окончания.');
  return { from, to };
}

export function timelinePoints(analytics) {
  const step = analytics.bucket_seconds || DAY;
  const first = Math.floor(analytics.from / step) * step;
  const buckets = new Map((analytics.timeline || []).map(b => [b.start, b.responses]));
  const count = Math.ceil((analytics.to - first) / step);
  // Keep SVG bounded for long selected periods while preserving totals.
  const stride = Math.max(1, Math.ceil(count / 120));
  const points = [];
  for (let start = first; start < analytics.to; start += step * stride) {
    points.push({ start, responses: 0 });
  }
  for (const [start, responses] of buckets) {
    const index = Math.floor((start - first) / (step * stride));
    if (index >= 0 && index < points.length) points[index].responses += responses;
  }
  return points;
}

export function rankElements(elements, kind = 'all', query = '') {
  const needle = query.trim().toLocaleLowerCase();
  return elements.filter(e => (kind === 'all' || e.target_kind === kind)
    && [e.id, e.kind, e.label, e.qualified_name, e.path, e.from, e.to]
      .some(value => String(value || '').toLocaleLowerCase().includes(needle)))
    .sort((a, b) => b.count - a.count || String(a.id).localeCompare(String(b.id)));
}
