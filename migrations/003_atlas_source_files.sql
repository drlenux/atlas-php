-- Per-source content hashes so a restart can skip unchanged files (INDEX restart baseline).
CREATE TABLE IF NOT EXISTS atlas_source_files (
  path text PRIMARY KEY,
  content_hash text NOT NULL
);
CREATE INDEX IF NOT EXISTS atlas_source_files_hash ON atlas_source_files (content_hash);
