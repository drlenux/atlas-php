> Historical verification of the pre-v0.4 implementation. PHP subprocess / Git CLI references below are obsolete. Current native-plugin evidence is recorded in `openspec/changes/add-code-atlas/verification.md` and the native testing reports.

# Runtime TDD evidence

- Config: initial four tests failed against an explicit unimplemented parser, then passed after ENV parsing/validation.
- Discovery/debounce: initial four behavior tests failed, then passed for content hashing, excluded directories, symlink boundaries and delay measured from the last change.
- Retry: a focused failing test demonstrated the missing retry API; after retry_after and watcher integration, unchanged input can be retried after a failed publication.
- Subprocess: two tests first failed, then passed for timeout/output bounds and separated stdin/stdout/stderr.
- Git: initial provenance test failed, then passed for addition, modification, current working changes and history absence; subsequent cross-agent regression evidence is in the Git review report.
- IndexCoordinator: tests first failed for lint-preservation and restart deletion, then passed; malformed plugin output leaves the index untouched.
- Full workflow: real PHP AST subprocess → service indexing → MCP context → correction → reindex → reviewer approve → analytics passed using MemoryRepository and the actual HTTP Router without opening a socket.

No PostgreSQL or Docker result is inferred from these tests. Their unavailable verification is recorded separately.
