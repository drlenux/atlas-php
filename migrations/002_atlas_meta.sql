-- Atlas-level facts such as the time of the last successful publication.
CREATE TABLE IF NOT EXISTS atlas_meta (
  key text PRIMARY KEY,
  value text NOT NULL
);
