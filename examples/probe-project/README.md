# Probe project for Atlas expression acceptance

Минимальный PHP-проект с выражениями владельца: `(int)Yii::$app->request->post(...)`, тернарный/`?:`/`??`, `$data['id']`, `+=`, вложенный `(object)[...]`.

## Локальная проверка без Docker

Из корня репозитория Atlas:

```sh
cargo run --locked --example probe_verify -- examples/probe-project
cargo run --locked --example probe_verify -- examples/probe-project --vendor
```

## Docker (готовый образ)

```sh
# из корня Atlas, один раз
ATLAS_IMAGE=drlenux/memory-tools:probe-local docker compose -f docker-compose.build.yml build atlas

cd examples/probe-project
ATLAS_IMAGE=drlenux/memory-tools:probe-local docker compose up -d --no-build
# UI: http://127.0.0.1:18085/  tokens: probe-agent / probe-reviewer
```
