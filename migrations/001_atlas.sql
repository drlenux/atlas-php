CREATE TABLE IF NOT EXISTS atlas_nodes (
  id text PRIMARY KEY,
  data jsonb NOT NULL,
  CHECK (data->>'id' = id),
  CHECK ((data->>'trust')::integer BETWEEN 1 AND 3),
  CHECK ((data->>'revision')::bigint >= 1)
);
CREATE INDEX IF NOT EXISTS atlas_nodes_name ON atlas_nodes ((data->>'qualified_name'));
CREATE INDEX IF NOT EXISTS atlas_nodes_path ON atlas_nodes ((data->'span'->>'path'));
CREATE INDEX IF NOT EXISTS atlas_nodes_kind ON atlas_nodes ((data->>'kind'));

CREATE TABLE IF NOT EXISTS atlas_edges (
  id text PRIMARY KEY,
  data jsonb NOT NULL,
  from_id text GENERATED ALWAYS AS (data->>'from') STORED REFERENCES atlas_nodes(id) ON DELETE CASCADE,
  to_id text GENERATED ALWAYS AS (data->>'to') STORED REFERENCES atlas_nodes(id) ON DELETE CASCADE,
  CHECK (data->>'id' = id),
  CHECK (from_id IS NOT NULL),
  CHECK ((data->>'trust')::integer BETWEEN 1 AND 3),
  CHECK ((data->>'revision')::bigint >= 1)
);
CREATE INDEX IF NOT EXISTS atlas_edges_from ON atlas_edges (from_id);
CREATE INDEX IF NOT EXISTS atlas_edges_to ON atlas_edges (to_id);
CREATE INDEX IF NOT EXISTS atlas_edges_path ON atlas_edges ((data->'span'->>'path'));

CREATE TABLE IF NOT EXISTS atlas_corrections (
  id text PRIMARY KEY,
  data jsonb NOT NULL
);
CREATE INDEX IF NOT EXISTS atlas_corrections_target ON atlas_corrections ((data->'correction'->>'target_kind'), (data->'correction'->>'target_id'));
CREATE INDEX IF NOT EXISTS atlas_corrections_status ON atlas_corrections ((data->'correction'->>'status'));

CREATE TABLE IF NOT EXISTS atlas_diagnostics (
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  data jsonb NOT NULL
);

CREATE TABLE IF NOT EXISTS atlas_deliveries (
  id text PRIMARY KEY,
  occurred_at bigint NOT NULL,
  data jsonb NOT NULL
);
CREATE INDEX IF NOT EXISTS atlas_deliveries_time ON atlas_deliveries (occurred_at);
