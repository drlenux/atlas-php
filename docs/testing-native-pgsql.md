# Native pgsql analyzer and SQL-in-PHP: evidence

Implementation: `src/plugins/pgsql/` (tokens, DDL `syntax`, DML `query`, `PgsqlLanguagePlugin`) plus PHP flow hooks in `src/plugins/php/flow.rs` that call `pgsql::embed_sql` for `createCommand` / `findBySql` / `prepare`. SQL text is data; `psql` and the project DB are never invoked.

## Observed TDD sequence

1. Added `tests/native_pgsql.rs` before a complete plugin module existed (lexer/syntax present; `mod.rs` and registry wiring missing).
2. Implemented DDL graph construction, DML table outline, registration in `src/plugins/mod.rs`, and PHP embedding for literal and `$sql = "…"` arguments.
3. Fixed statement recovery so a broken `CREATE TABLE t (;` does not swallow the next statement (skip to `;` without paren depth).
4. Updated `tests/native_runtime` catalog expectations to include `pgsql` alongside `php`.

```sh
cargo test --locked --test native_pgsql
cargo clippy --locked --lib --test native_pgsql -- -D warnings
```

**7 native_pgsql tests passed**; Clippy with warnings denied on the library and this test target.

## Coverage map

| Behavior | Evidence |
| --- | --- |
| `CREATE SCHEMA` / `TABLE` / columns / `REFERENCES` | `create_table_columns_and_references_become_graph` |
| Comments and string literals do not invent declarations | `comments_and_strings_do_not_invent_declarations` |
| Parse error keeps sibling statements | `syntax_errors_keep_other_statements` |
| Plugin catalog + graph element limit | `plugin_registered_and_limits_enforced` |
| Yii-style `createCommand('UPDATE wallet …')` → `sql_site` + `sql_writes` | `php_create_command_update_links_wallet_table` |
| `findBySql` + `$query = "SELECT…"; createCommand($query)` | `php_find_by_sql_and_variable_select` |
| CTE name skipped; `DELETE FROM` writes real table | `php_cte_delete_skips_cte_name` |

## Boundaries

- Unqualified names default to schema `public`.
- Only string literals (and variables whose last assignment was a string literal) on the listed APIs are outlined; interpolated/concatenated SQL without a recoverable literal is skipped.
- ActiveQuery `where([...])` arrays are not SQL text and are not parsed here.
- Enabling `index.languages: […, pgsql]` indexes `.sql`/`.pgsql` files; SQL-in-PHP works whenever the PHP language plugin runs.
