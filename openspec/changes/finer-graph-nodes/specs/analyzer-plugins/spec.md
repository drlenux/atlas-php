# Analyzer plugins — finer PHP graph

## ADDED Requirements

### Requirement: PHP-FINE-01 Эмиссия мелких вершин

Нативный PHP-плагин SHALL при анализе тела функции/метода создавать вершины `branch` и occurrence (`call_site`, `write_site`, `read_site`, `return_site`) согласно `code-graph`, и SHALL NOT помещать полный текст тела в metadata method/function.

#### Scenario: Fixture с веткой и вызовом

- **WHEN** анализируется метод с `if` и вызовом внутри ветки
- **THEN** снимок содержит `branch` и `call_site`
- **AND** method-нода не содержит ключей `body` и `source`

### Requirement: PHP-FINE-02 Комментарии как источник нод

Нативный PHP-плагин SHALL сохранять spans комментариев при лексическом разборе и SHALL пытаться извлечь из них объявления и occurrence при наличии PHP-сигналов (`$`, `->`, `::`, `function`, `class`, `return`). Извлечённые вершины SHALL иметь `commented: true`. Ошибка разбора отдельного комментария SHALL NOT срывать индексацию файла.

#### Scenario: Строчный комментарий с вызовом

- **WHEN** в теле метода есть строка `// $wallet->charge(1);`
- **THEN** создаётся occurrence с `commented: true`
- **AND** остальной анализ файла завершается успешно
