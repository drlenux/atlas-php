// Presentation model for the code explorer: readable labels and a grouped context.
// Pure functions; app.js binds them to the DOM.

const OCCURRENCE_KINDS = new Set([
  'call_site', 'read_site', 'write_site', 'return_site', 'branch', 'sql_site', 'url_site',
]);
const EFFECTS = {
  calls: 'calls',
  writes: 'writes',
  reads: 'reads',
  dispatches: 'dispatches',
  sql_reads: 'sql_reads',
  sql_writes: 'sql_writes',
};
const RELATION_KINDS = new Set([
  'inherits', 'implements', 'uses', 'overrides', 'overridden_by',
  'references', 'indexes', 'contains', 'subscribes', 'unsubscribes', 'triggers',
]);

export const SEARCH_GROUPS = [
  { id: 'all', label: 'Все сущности', kinds: [] },
  { id: 'declarations', label: 'Объявления', kinds: ['class', 'interface', 'trait', 'enum', 'method', 'function', 'property'] },
  { id: 'calls', label: 'Вызовы', kinds: ['call_site'] },
  { id: 'writes', label: 'Записи', kinds: ['write_site'] },
  { id: 'reads', label: 'Чтения', kinds: ['read_site'] },
  { id: 'returns', label: 'Возвраты', kinds: ['return_site'] },
  { id: 'branches', label: 'Ветки', kinds: ['branch'] },
  { id: 'variables', label: 'Переменные', kinds: ['variable'] },
  { id: 'sql', label: 'SQL-сайты', kinds: ['sql_site'] },
  { id: 'tables', label: 'Таблицы / DDL', kinds: ['table', 'schema', 'column', 'function', 'index'] },
  { id: 'yii_views', label: 'Yii2 views', kinds: ['view'] },
  { id: 'config', label: 'Конфиг / маршруты', kinds: ['config_key', 'route', 'url_site'] },
  { id: 'events', label: 'События', kinds: ['event'] },
];

// PHP plugin ids: <kind>:<function id>[:<kind>]:[comment:]php-struct-v1:<hash>[:n]
const OWNER = /^(?:call_site|read_site|write_site|return_site|branch|sql_site|url_site):(.+?)(?::(?:call_site|read_site|write_site|return_site|branch|sql_site|url_site))?:(?:comment:)?php-struct-v1:/;

export function ownerOf(id) {
  return OWNER.exec(String(id ?? ''))?.[1] ?? null;
}

export function isOccurrence(kind) {
  return OCCURRENCE_KINDS.has(kind);
}

function shortName(id) {
  const name = String(id).replace(/^(?:method|function):/, '');
  return name.slice(name.lastIndexOf('\\') + 1);
}

function oneLine(text) {
  return String(text ?? '').replace(/\s+/g, ' ').trim();
}

export function entityLabel(entity) {
  const kind = entity.kind;
  if (isOccurrence(kind)) {
    const ownerId = ownerOf(entity.id);
    let title = oneLine(entity.name ?? entity.label);
    if (kind === 'branch') {
      const keyword = entity.metadata?.keyword || title || 'if';
      const condition = oneLine(entity.metadata?.condition);
      title = condition ? `${keyword} (${condition})` : keyword;
    }
    return { title: title || kind, subtitle: ownerId ? shortName(ownerId) : '', ownerId };
  }
  if (kind === 'table' || kind === 'schema' || kind === 'column' || kind === 'view') {
    return { title: entity.qualified_name || entity.name || entity.id, subtitle: kind, ownerId: null };
  }
  return { title: entity.qualified_name || entity.name || entity.label || entity.id, subtitle: '', ownerId: null };
}

function location(node) {
  const span = node?.span;
  return span?.path ? `${span.path}:${span.start_line}` : '';
}

function reference(node, id) {
  if (!node) return id ? { id, title: id, kind: '', location: '' } : null;
  const label = entityLabel(node);
  return { id: node.id, kind: node.kind, title: label.title, subtitle: label.subtitle, location: location(node), trust: node.trust };
}

/** Groups a GraphContext relative to its root for reading. */
export function contextView(context) {
  const nodes = new Map((context.nodes || []).map(n => [n.id, n]));
  const edges = context.edges || [];
  const rootId = context.root_id;
  const ownerOfSite = siteId => {
    const contains = edges.find(e => e.kind === 'contains' && e.to === siteId && !isOccurrence(nodes.get(e.from)?.kind));
    return contains ? reference(nodes.get(contains.from), contains.from) : reference(null, ownerOf(siteId));
  };
  const conditions = site => site?.metadata?.conditions || [];
  const effectEdges = siteId => edges.filter(e => e.from === siteId && EFFECTS[e.kind]);

  const owners = edges
    .filter(e => e.kind === 'contains' && e.to === rootId && !isOccurrence(nodes.get(e.from)?.kind) && nodes.has(e.from))
    .map(e => reference(nodes.get(e.from)));

  const incoming = { calls: [], writes: [], reads: [], dispatches: [], sql_reads: [], sql_writes: [] };
  for (const e of edges) {
    if (e.to !== rootId || !EFFECTS[e.kind]) continue;
    const site = nodes.get(e.from);
    incoming[e.kind].push({
      site: reference(site, e.from),
      owner: ownerOfSite(e.from),
      conditions: conditions(site),
      resolution: e.metadata?.resolution ?? site?.metadata?.resolution ?? null,
      queue: e.metadata?.queue ?? null,
    });
  }

  const effects = { calls: [], writes: [], reads: [], returns: [], dispatches: [], sql_reads: [], sql_writes: [] };
  const branches = [];
  const addSite = site => {
    if (site.kind === 'branch') {
      branches.push({ ...reference(site), conditions: conditions(site) });
      return;
    }
    if (site.kind === 'return_site') {
      effects.returns.push({ site: reference(site), target: null, conditions: conditions(site), resolution: null });
      return;
    }
    const group = {
      call_site: 'calls',
      write_site: 'writes',
      read_site: 'reads',
      sql_site: 'sql_reads',
    }[site.kind];
    if (!group) return;
    const found = effectEdges(site.id);
    if (!found.length) {
      effects[group].push({ site: reference(site), target: null, conditions: conditions(site), resolution: site.metadata?.resolution ?? null });
    }
    for (const e of found) {
      const bucket = EFFECTS[e.kind] || group;
      effects[bucket].push({
        site: reference(site),
        target: e.to ? reference(nodes.get(e.to), e.to) : null,
        conditions: conditions(site),
        resolution: e.metadata?.resolution ?? site.metadata?.resolution ?? null,
        queue: e.metadata?.queue ?? null,
      });
    }
  };
  const root = nodes.get(rootId);
  if (root && isOccurrence(root.kind)) {
    if (root.kind !== 'branch') addSite(root);
    if (!owners.length) {
      const owner = ownerOfSite(rootId);
      if (owner) owners.push(owner);
    }
  }
  for (const e of edges) {
    if (e.kind === 'contains' && e.from === rootId && isOccurrence(nodes.get(e.to)?.kind)) addSite(nodes.get(e.to));
  }

  const shown = new Set();
  const relations = [];
  const other = [];
  for (const e of edges) {
    if (!(e.from === rootId || e.to === rootId)) continue;
    if (e.kind === 'contains' && (e.to === rootId || isOccurrence(nodes.get(e.to)?.kind))) continue;
    if (EFFECTS[e.kind] && (e.to === rootId || e.from === rootId)) continue;
    if (shown.has(e.id) || !shown.add(e.id)) continue;
    const outgoing = e.from === rootId;
    const otherId = outgoing ? e.to : e.from;
    const item = { kind: e.kind, outgoing, node: otherId ? reference(nodes.get(otherId), otherId) : null };
    if (RELATION_KINDS.has(e.kind) && !(e.kind === 'contains' && isOccurrence(nodes.get(e.to)?.kind))) {
      relations.push(item);
    } else {
      other.push(item);
    }
  }

  return {
    root: { ...reference(root, rootId), metadata: root?.metadata || {} },
    owners,
    incoming,
    effects,
    branches,
    relations,
    other,
    diagnostics: (context.diagnostics || []).length,
    truncated: Boolean(context.truncated),
  };
}
