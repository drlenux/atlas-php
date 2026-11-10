# Design: atlas-followups

## Публикация

`node_unchanged`/`edge_unchanged` (`apply_snapshot`) пропускают запись объекта уровня 1, только если вся `metadata` равна. Объекты уровня 2/3 по-прежнему защищены `refresh_metadata`. Стоимость — сравнение JSON при совпадающих id. Metadata анализатора детерминирована (`serde_json` без `preserve_order` — отсортированные ключи), поэтому повторный прогон ничего не переписывает.

Диагностики: из прежних сохраняются только те, путь которых не входит в область публикации и о котором новый анализ ничего не сообщил.

## Замыкания

Замыкание, владелец которого — функция или метод (`nested_closure`), исключается из параллельного списка функций и анализируется в `walk_expr(Kind::Closure)` внешней функции: `analyze_with(project, closure, inherited, annotated_vars)`. `inherited` — переменные `use (...)` из текущего окружения (для `fn` — всё окружение). Узлы пишутся в тот же `FlowHost`. Замыкания в свойствах класса и вне функций анализируются, как раньше.

## Константы

Парсер собирает `const [type] A = …, B = …` и `case A [= …]` в теле класса (`Class.constants`). PHP-плагин создаёт вершины `constant:<Class>::<NAME>` (`metadata.value` — текст значения до 80 символов) и `contains`. В `flow.rs` статический доступ без `$` — константа: поиск по классу, родителям и интерфейсам; `::class` даёт тип `string` без ребра.

## Типы аргументов и очереди

`calls.metadata.arguments = [{type, type_source}]` для каждого аргумента. Yii2 `enrich` после прочих шагов строит `dispatches`. Цели вызова (`to` или все `candidates`) должны быть методами `push` классов `yii\queue\Queue`. Классы первого аргумента с методом `execute` дают ребро от места вызова к `execute`. `resolution`: `annotated`, если вызов или тип аргумента получены через аннотацию; иначе `resolved`. `queue` — имя компонента из `…$app-><name>`.

## Outline метода

`calls`: для каждого `call_site` метода — рёбра `calls`/`dispatches`, строка `L<line> <expression ≤80> → <цель|?> [resolution]` (`⇒` для очереди). `called_by`: входящие `calls`/`dispatches`, строка `<owner> L<line>: <expression> [resolution]`.
