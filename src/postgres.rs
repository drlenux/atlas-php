//! Minimal synchronous libpq adapter. Every dynamic value is passed as a parameter.
use crate::{domain::*, repository::Repository};
use serde::{de::DeserializeOwned, Serialize};
use std::ffi::{c_char, c_int, CStr, CString};
use std::ptr::{self, NonNull};

#[repr(C)]
struct PgConn {
    _private: [u8; 0],
}
#[repr(C)]
struct PgResult {
    _private: [u8; 0],
}
#[link(name = "pq")]
extern "C" {
    fn PQconnectdb(info: *const c_char) -> *mut PgConn;
    fn PQstatus(connection: *const PgConn) -> c_int;
    fn PQerrorMessage(connection: *const PgConn) -> *mut c_char;
    fn PQfinish(connection: *mut PgConn);
    fn PQexec(connection: *mut PgConn, query: *const c_char) -> *mut PgResult;
    fn PQexecParams(
        connection: *mut PgConn,
        query: *const c_char,
        count: c_int,
        types: *const u32,
        values: *const *const c_char,
        lengths: *const c_int,
        formats: *const c_int,
        result_format: c_int,
    ) -> *mut PgResult;
    fn PQresultStatus(result: *const PgResult) -> c_int;
    fn PQresultErrorMessage(result: *const PgResult) -> *mut c_char;
    fn PQntuples(result: *const PgResult) -> c_int;
    fn PQnfields(result: *const PgResult) -> c_int;
    fn PQgetisnull(result: *const PgResult, row: c_int, column: c_int) -> c_int;
    fn PQgetvalue(result: *const PgResult, row: c_int, column: c_int) -> *mut c_char;
    fn PQclear(result: *mut PgResult);
}

struct SqlResult(NonNull<PgResult>);
impl Drop for SqlResult {
    fn drop(&mut self) {
        unsafe { PQclear(self.0.as_ptr()) }
    }
}

pub struct PostgresRepository {
    connection: NonNull<PgConn>,
    transaction: bool,
}
// The owned connection is moved between threads, never concurrently accessed. All API
// operations require &mut self, and the service serializes access. libpq supports this.
unsafe impl Send for PostgresRepository {}
impl Drop for PostgresRepository {
    fn drop(&mut self) {
        unsafe { PQfinish(self.connection.as_ptr()) }
    }
}

impl PostgresRepository {
    pub fn connect(database_url: &str) -> Result<Self> {
        let info = cstring(database_url)?;
        let pointer = unsafe { PQconnectdb(info.as_ptr()) };
        let connection = NonNull::new(pointer)
            .ok_or_else(|| AtlasError::storage("PostgreSQL allocation failed"))?;
        let repo = Self {
            connection,
            transaction: false,
        };
        if unsafe { PQstatus(pointer) } != 0 {
            return Err(AtlasError::storage(format!(
                "PostgreSQL connection failed: {}",
                unsafe { string(PQerrorMessage(pointer)) }
            )));
        }
        Ok(repo)
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.execute_static("BEGIN")?;
        for migration in [
            include_str!("../migrations/001_atlas.sql"),
            include_str!("../migrations/002_atlas_meta.sql"),
        ] {
            if let Err(error) = self.execute_static(migration) {
                let _ = self.execute_static("ROLLBACK");
                return Err(error);
            }
        }
        self.execute_static("COMMIT")
    }

    fn execute_static(&mut self, sql: &str) -> Result<()> {
        let query = cstring(sql)?;
        let pointer = unsafe { PQexec(self.connection.as_ptr(), query.as_ptr()) };
        self.check_result(pointer)?;
        Ok(())
    }

    fn check_result(&self, pointer: *mut PgResult) -> Result<SqlResult> {
        let result = SqlResult(
            NonNull::new(pointer)
                .ok_or_else(|| AtlasError::storage("PostgreSQL returned no result"))?,
        );
        let status = unsafe { PQresultStatus(pointer) };
        if status != 1 && status != 2 {
            return Err(AtlasError::storage(format!(
                "PostgreSQL query failed: {}",
                unsafe { string(PQresultErrorMessage(pointer)) }
            )));
        }
        Ok(result)
    }

    fn query(&mut self, sql: &str, params: &[&str]) -> Result<Vec<Vec<Option<String>>>> {
        let query = cstring(sql)?;
        let values: Vec<_> = params.iter().map(|p| cstring(p)).collect::<Result<_>>()?;
        let pointers: Vec<_> = values.iter().map(|s| s.as_ptr()).collect();
        let result = self.check_result(unsafe {
            PQexecParams(
                self.connection.as_ptr(),
                query.as_ptr(),
                values.len() as c_int,
                ptr::null(),
                pointers.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
            )
        })?;
        let mut rows = vec![];
        for row in 0..unsafe { PQntuples(result.0.as_ptr()) } {
            let mut cells = vec![];
            for column in 0..unsafe { PQnfields(result.0.as_ptr()) } {
                cells.push(
                    if unsafe { PQgetisnull(result.0.as_ptr(), row, column) } != 0 {
                        None
                    } else {
                        Some(unsafe { string(PQgetvalue(result.0.as_ptr(), row, column)) })
                    },
                );
            }
            rows.push(cells);
        }
        Ok(rows)
    }
    fn objects<T: DeserializeOwned>(&mut self, sql: &str, params: &[&str]) -> Result<Vec<T>> {
        self.query(sql, params)?
            .into_iter()
            .map(|row| {
                let value = row
                    .first()
                    .and_then(Option::as_deref)
                    .ok_or_else(|| AtlasError::storage("missing stored JSON"))?;
                serde_json::from_str(value)
                    .map_err(|e| AtlasError::storage(format!("invalid stored JSON: {e}")))
            })
            .collect()
    }
    fn write<T: Serialize>(&mut self, sql: &str, id: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value).map_err(|e| AtlasError::storage(e.to_string()))?;
        self.query(sql, &[id, &json])?;
        Ok(())
    }

    fn upsert_json_rows<T: Serialize>(&mut self, table: &str, rows: &[T]) -> Result<()> {
        const CHUNK: usize = 5_000;
        for chunk in rows.chunks(CHUNK) {
            let payload: Vec<_> = chunk
                .iter()
                .map(|row| {
                    let data = serde_json::to_value(row)
                        .map_err(|e| AtlasError::storage(e.to_string()))?;
                    let id = data
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| AtlasError::storage("stored object missing id"))?
                        .to_owned();
                    Ok(serde_json::json!({"id": id, "data": data}))
                })
                .collect::<Result<Vec<_>>>()?;
            let json =
                serde_json::to_string(&payload).map_err(|e| AtlasError::storage(e.to_string()))?;
            let sql = format!(
                "INSERT INTO {table} (id,data) SELECT id, data FROM jsonb_to_recordset($1::jsonb) AS x(id text, data jsonb) ON CONFLICT(id) DO UPDATE SET data=EXCLUDED.data"
            );
            self.query(&sql, &[&json])?;
        }
        Ok(())
    }

    fn objects_for_paths<T: DeserializeOwned>(
        &mut self,
        table: &str,
        paths: &std::collections::BTreeSet<String>,
    ) -> Result<Vec<T>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let json = serde_json::to_string(paths).map_err(|e| AtlasError::storage(e.to_string()))?;
        // No ORDER BY: publish matching does not need sorted rows.
        let sql = format!(
            "SELECT data FROM {table} WHERE data->'span'->>'path' IN (SELECT jsonb_array_elements_text($1::jsonb))"
        );
        self.objects(&sql, &[&json])
    }

    fn delete_ids(&mut self, table: &str, ids: &[String]) -> Result<()> {
        const CHUNK: usize = 1_000;
        for chunk in ids.chunks(CHUNK) {
            let json =
                serde_json::to_string(chunk).map_err(|e| AtlasError::storage(e.to_string()))?;
            let sql = format!(
                "DELETE FROM {table} WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb))"
            );
            self.query(&sql, &[&json])?;
        }
        Ok(())
    }
}

impl Repository for PostgresRepository {
    fn source_paths(&mut self) -> Result<Vec<String>> {
        Ok(self.query("SELECT path FROM (SELECT data->'span'->>'path' AS path FROM atlas_nodes UNION SELECT data->'span'->>'path' AS path FROM atlas_edges) p WHERE path IS NOT NULL AND path <> '' ORDER BY path",&[])?.into_iter().filter_map(|row|row.into_iter().next().flatten()).collect())
    }
    fn begin(&mut self) -> Result<()> {
        if self.transaction {
            return Err(AtlasError::storage("nested transaction"));
        }
        self.execute_static("BEGIN")?;
        // All application writers acquire the same transaction-scoped lock. The service
        // checks expected_revision after the lock, including across separate processes.
        if let Err(error) = self.query("SELECT pg_advisory_xact_lock(714032196)", &[]) {
            let _ = self.execute_static("ROLLBACK");
            return Err(error);
        }
        self.transaction = true;
        Ok(())
    }
    fn commit(&mut self) -> Result<()> {
        self.execute_static("COMMIT")?;
        self.transaction = false;
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        let result = self.execute_static("ROLLBACK");
        self.transaction = false;
        result
    }
    fn nodes(&mut self) -> Result<Vec<Node>> {
        self.objects("SELECT data FROM atlas_nodes", &[])
    }
    fn edges(&mut self) -> Result<Vec<Edge>> {
        self.objects("SELECT data FROM atlas_edges", &[])
    }
    fn nodes_for_paths(&mut self, paths: &std::collections::BTreeSet<String>) -> Result<Vec<Node>> {
        self.objects_for_paths("atlas_nodes", paths)
    }
    fn edges_for_paths(
        &mut self,
        paths: &std::collections::BTreeSet<String>,
        node_ids: &std::collections::BTreeSet<String>,
    ) -> Result<Vec<Edge>> {
        if paths.is_empty() && node_ids.is_empty() {
            return Ok(Vec::new());
        }
        let paths_json =
            serde_json::to_string(paths).map_err(|e| AtlasError::storage(e.to_string()))?;
        let nodes_json =
            serde_json::to_string(node_ids).map_err(|e| AtlasError::storage(e.to_string()))?;
        self.objects(
            "SELECT data FROM atlas_edges WHERE data->'span'->>'path' IN (SELECT jsonb_array_elements_text($1::jsonb)) OR from_id IN (SELECT jsonb_array_elements_text($2::jsonb)) OR to_id IN (SELECT jsonb_array_elements_text($2::jsonb))",
            &[&paths_json, &nodes_json],
        )
    }
    fn node(&mut self, id: &str) -> Result<Option<Node>> {
        Ok(self
            .objects("SELECT data FROM atlas_nodes WHERE id=$1", &[id])?
            .pop())
    }
    fn edge(&mut self, id: &str) -> Result<Option<Edge>> {
        Ok(self
            .objects("SELECT data FROM atlas_edges WHERE id=$1", &[id])?
            .pop())
    }
    fn put_node(&mut self, n: &Node) -> Result<()> {
        self.put_nodes(std::slice::from_ref(n))
    }
    fn put_edge(&mut self, e: &Edge) -> Result<()> {
        self.put_edges(std::slice::from_ref(e))
    }
    fn put_nodes(&mut self, nodes: &[Node]) -> Result<()> {
        self.upsert_json_rows("atlas_nodes", nodes)
    }
    fn put_edges(&mut self, edges: &[Edge]) -> Result<()> {
        self.upsert_json_rows("atlas_edges", edges)
    }
    fn delete_node(&mut self, id: &str) -> Result<()> {
        self.delete_nodes(&[id.to_owned()])
    }
    fn delete_edge(&mut self, id: &str) -> Result<()> {
        self.delete_edges(&[id.to_owned()])
    }
    fn delete_nodes(&mut self, ids: &[String]) -> Result<()> {
        self.delete_ids("atlas_nodes", ids)
    }
    fn delete_edges(&mut self, ids: &[String]) -> Result<()> {
        self.delete_ids("atlas_edges", ids)
    }
    fn corrections(&mut self) -> Result<Vec<StoredCorrection>> {
        self.objects("SELECT data FROM atlas_corrections ORDER BY id", &[])
    }
    fn put_correction(&mut self, c: &StoredCorrection) -> Result<()> {
        self.write("INSERT INTO atlas_corrections(id,data) VALUES ($1,$2::jsonb) ON CONFLICT(id) DO UPDATE SET data=EXCLUDED.data",&c.correction.id,c)
    }
    fn diagnostics(&mut self) -> Result<Vec<Diagnostic>> {
        Ok(self
            .objects::<Vec<Diagnostic>>(
                "SELECT data FROM atlas_diagnostics WHERE singleton=true",
                &[],
            )?
            .pop()
            .unwrap_or_default())
    }
    fn set_diagnostics(&mut self, d: &[Diagnostic]) -> Result<()> {
        let json = serde_json::to_string(d).map_err(|e| AtlasError::storage(e.to_string()))?;
        self.query("INSERT INTO atlas_diagnostics(singleton,data) VALUES(true,$1::jsonb) ON CONFLICT(singleton) DO UPDATE SET data=EXCLUDED.data",&[&json])?;
        Ok(())
    }
    fn delivery(&mut self, id: &str) -> Result<Option<Delivery>> {
        Ok(self
            .objects("SELECT data FROM atlas_deliveries WHERE id=$1", &[id])?
            .pop())
    }
    fn deliveries(&mut self, from: i64, to: i64) -> Result<Vec<Delivery>> {
        self.objects("SELECT data FROM atlas_deliveries WHERE occurred_at >= $1::bigint AND occurred_at < $2::bigint ORDER BY occurred_at,id",&[&from.to_string(),&to.to_string()])
    }
    fn meta(&mut self, key: &str) -> Result<Option<String>> {
        Ok(self
            .query("SELECT value FROM atlas_meta WHERE key=$1", &[key])?
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next().flatten()))
    }
    fn set_meta(&mut self, key: &str, value: &str) -> Result<()> {
        self.query(
            "INSERT INTO atlas_meta(key,value) VALUES ($1,$2) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value",
            &[key, value],
        )?;
        Ok(())
    }
    fn existing_node_ids(
        &mut self,
        ids: &std::collections::BTreeSet<String>,
    ) -> Result<std::collections::BTreeSet<String>> {
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let list = serde_json::to_string(ids).map_err(|e| AtlasError::storage(e.to_string()))?;
        Ok(self
            .query(
                "SELECT id FROM atlas_nodes WHERE id IN (SELECT jsonb_array_elements_text($1::jsonb))",
                &[&list],
            )?
            .into_iter()
            .filter_map(|row| row.into_iter().next().flatten())
            .collect())
    }
    fn reset(&mut self) -> Result<()> {
        self.execute_static(
            "TRUNCATE atlas_edges, atlas_nodes, atlas_corrections, atlas_diagnostics, atlas_deliveries, atlas_meta",
        )
    }
    fn put_delivery(&mut self, d: &Delivery) -> Result<()> {
        let data = serde_json::to_string(d).map_err(|e| AtlasError::storage(e.to_string()))?;
        self.query("INSERT INTO atlas_deliveries(id,occurred_at,data) VALUES ($1,$2::bigint,$3::jsonb) ON CONFLICT(id) DO NOTHING",&[&d.id,&d.timestamp.to_string(),&data])?;
        Ok(())
    }
    fn search_nodes(&mut self, search: &NodeSearch) -> Result<Vec<Node>> {
        // Kinds are validated by the service as [a-z0-9_]+, so a comma list is unambiguous.
        // Ranked by relevance (exact → declarations → other → occurrences), then id.
        // Cursor is "{score}|{id}" from encode_search_cursor.
        let (after_score, after_id) = match search.after.as_deref() {
            Some(cursor) if cursor.contains('|') || cursor.contains('\t') => {
                let (score, id) = cursor
                    .split_once('|')
                    .or_else(|| cursor.split_once('\t'))
                    .unwrap_or(("0", cursor));
                (score.parse::<i32>().unwrap_or(0).to_string(), id.to_owned())
            }
            Some(id) if !id.is_empty() => ("0".into(), id.to_owned()),
            _ => ("-1".into(), String::new()),
        };
        self.objects(
            "WITH q AS (
                -- One case-insensitive pattern: escaped query, plus identifier
                -- boundaries at identifier edges in text mode (substring: none).
                -- Each predicate runs a cheap strpos first; the regex only checks candidates.
                SELECT CASE WHEN $5='text' AND lower($1) ~ '^[a-z0-9_]' THEN '(^|[^a-z0-9_])' ELSE '' END
                  || regexp_replace(lower($1), '([.^$*+?()\\[\\]{}|\\\\-])', '\\\\\\1', 'g')
                  || CASE WHEN $5='text' AND lower($1) ~ '[a-z0-9_]$' THEN '($|[^a-z0-9_])' ELSE '' END AS pat,
                -- Quoted string literals; text mode matches code fields without them.
                '''(?:[^''\\\\]|\\\\.)*''|\"(?:[^\"\\\\]|\\\\.)*\"'::text AS lit
            )
            SELECT data FROM (
                SELECT data, id,
                  CASE
                    WHEN id=$1 OR lower(ltrim(data->>'name','$'))=lower(ltrim($1,'$')) OR lower(coalesce(data->>'qualified_name',''))=lower($1) THEN 0
                    WHEN data->>'kind' IN ('class','method','function','variable','property','trait','interface','enum','route','event','view','config_key') THEN 1
                    WHEN data->>'kind' IN ('call_site','write_site','read_site','return_site','url_site','branch') THEN 3
                    ELSE 2
                  END AS score
                FROM atlas_nodes, q
                WHERE (id=$1 OR ($5 IN ('text','substring') AND ((strpos(lower(data->>'name'),lower($1))>0 AND lower(CASE WHEN $5='text' THEN regexp_replace(data->>'name', q.lit, ' ', 'g') ELSE data->>'name' END) ~ q.pat) OR (data->>'kind' IN ('class','method','function','property','trait','interface','enum','route','event','view','config_key') AND (strpos(lower(data->>'qualified_name'),lower($1))>0 AND lower(data->>'qualified_name') ~ q.pat)) OR (strpos(lower(coalesce(data->'metadata'->>'expression','')),lower($1))>0 AND lower(CASE WHEN $5='text' THEN regexp_replace(coalesce(data->'metadata'->>'expression',''), q.lit, ' ', 'g') ELSE coalesce(data->'metadata'->>'expression','') END) ~ q.pat) OR (strpos(lower(coalesce(data->'metadata'->>'value_expression','')),lower($1))>0 AND lower(CASE WHEN $5='text' THEN regexp_replace(coalesce(data->'metadata'->>'value_expression',''), q.lit, ' ', 'g') ELSE coalesce(data->'metadata'->>'value_expression','') END) ~ q.pat) OR (strpos(lower(coalesce(data->'metadata'->>'value_constant','')),lower($1))>0 AND lower(coalesce(data->'metadata'->>'value_constant','')) ~ q.pat) OR (strpos(lower(coalesce(data->'metadata'->>'member_name','')),lower($1))>0 AND lower(coalesce(data->'metadata'->>'member_name','')) ~ q.pat))) OR ($5='symbol' AND ((data->>'kind' IN ('class','method','function','variable','property','trait','interface','enum','route','event','view','config_key') AND (lower(ltrim(data->>'name','$'))=lower(ltrim($1,'$')) OR lower(data->>'qualified_name')=lower($1))) OR lower(ltrim(data->'metadata'->>'member_name','$'))=lower(ltrim($1,'$')) OR (data->'metadata'->>'value_constant' IS NOT NULL AND (lower(data->'metadata'->>'value_constant')=lower($1) OR lower(substring(data->'metadata'->>'value_constant' from '[^:]*$'))=lower(ltrim(regexp_replace($1,'^.*::',''),'$')) OR (strpos($1,'::')=0 AND lower(substring(data->'metadata'->>'value_constant' from '[^:]*$'))=lower(ltrim($1,'$'))))) OR (strpos($1,'::')>0 AND lower(ltrim(data->'metadata'->>'member_name','$'))=lower(ltrim(regexp_replace($1,'^.*::',''),'$')) AND EXISTS (SELECT 1 FROM (SELECT lower(regexp_replace($1,'::[^:]*$','')) AS own) o, (VALUES ('receiver_class'),('receiver_root_class')) AS v(cls), LATERAL unnest(string_to_array(translate(coalesce(data->'metadata'->>cls,''),'&','|'),'|')) AS part WHERE lower(btrim(part))=own OR right(lower(btrim(part)),length(own)+1)=chr(92)||own OR (strpos(own,chr(92))=0 AND lower(substring(btrim(part) from '[^\\]*$'))=own))))))
                  AND ($3='' OR data->>'kind'=ANY(string_to_array($3,',')))
                  AND ($7='' OR coalesce(data->'span'->>'path','') !~ $7)
                  AND ($8='' OR coalesce(data->'span'->>'path','') ~ $8)
            ) s
            WHERE ($6::int < 0 OR score>$6::int OR (score=$6::int AND id>$4))
            ORDER BY score, id
            LIMIT $2::bigint",
            &[
                &search.text,
                &search.limit.to_string(),
                &search.kinds.join(","),
                &after_id,
                match search.mode {
                    SearchMode::Text => "text",
                    SearchMode::Substring => "substring",
                    SearchMode::Symbol => "symbol",
                },
                &after_score,
                &search.exclude_paths.clone().unwrap_or_default(),
                &search.include_paths.clone().unwrap_or_default(),
            ],
        )
    }
    fn adjacent_edges(&mut self, id: &str) -> Result<Vec<Edge>> {
        self.objects(
            "SELECT data FROM atlas_edges WHERE from_id=$1 OR to_id=$1 ORDER BY id",
            &[id],
        )
    }
}

fn cstring(value: &str) -> Result<CString> {
    CString::new(value).map_err(|_| AtlasError::invalid("NUL byte in database parameter"))
}
unsafe fn string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}
