# Соответствие техзаданию

## Уточнение по диагностике PHP от 2026-11-04

Источник уточнения — присланный владельцем контекст `TestController.php`, локальное воспроизведение и прямой запрос «в спеку запиши». Это новые критерии исправления текущего дефекта; их наличие в OpenSpec не означает выполненную реализацию.

| Наблюдение / критерий | Контракт | Статус |
| --- | --- | --- |
| Приведения теряют вызовы; тернарные выражения, доступ к массивам и вложенные object casts разбираются неполно | [PLUGIN-05](specs/analyzer-plugins/spec.md#requirement-plugin-05-сохранение-семантики-составных-php-выражений) | Локально воспроизведено; исправление открыто. `+=` дополнительно включён по сообщению владельца. |
| Одна конструкция порождает каскад фрагментных сообщений и неполный контекст | [PLUGIN-06](specs/analyzer-plugins/spec.md#requirement-plugin-06-локализованные-диагностики-и-честная-неполнота) | Локально воспроизведено; регрессии и исправление открыты. |
| Определения Yii отсутствуют, но причина не установлена | [PLUGIN-07](specs/analyzer-plugins/spec.md#requirement-plugin-07-разделение-недоступных-определений-и-дефектов-разбора) | Наблюдение владельца; диагностика конфигурации его контейнера не выполнена. |

Границы решения — [design.md](design.md), свидетельства — [verification.md](verification.md), работа — [tasks.md, раздел 12](tasks.md#12-исправление-разбора-php-выражений-и-каскадных-диагностик).

## Исходное покрытие ТЗ

Источник: [code-atlas-specification.md, версия 0.3 от 4 ноября 2026 года](../../../code-atlas-specification.md).

Идентификаторы `G-*`, `D-*`, `R-*`, `H-*`, `A-*` и номера проверочных сценариев ниже взяты из ТЗ. Идентификаторы `RUNTIME-*`, `GRAPH-*` и остальные добавлены только для ссылок на требования OpenSpec. Таблицы показывают покрытие требований, а не результаты функциональных тестов. Для аналитики сохранено различие между требованием владельца и рабочей конкретизацией метрик.

## Цели

| Источник | Покрытие OpenSpec |
| --- | --- |
| G-01: структурированные сведения о коде | [GRAPH-01–GRAPH-05](specs/code-graph/spec.md), [RUNTIME-02](specs/atlas-runtime/spec.md), [MCP-01](specs/mcp-context/spec.md) |
| G-02: уменьшение повторного поиска связей | [MCP-01](specs/mcp-context/spec.md), [INDEX-01](specs/project-indexing/spec.md); численный эффект не заявлен |
| G-03: контекст механизма работы | [MCP-01](specs/mcp-context/spec.md), [RETURN-02–RETURN-03](specs/return-analysis/spec.md) |
| G-04: одноимённые методы и интерфейсы | [GRAPH-04, GRAPH-06](specs/code-graph/spec.md), [PLUGIN-02](specs/analyzer-plugins/spec.md) |
| G-05: исправление и проверка | [MCP-03](specs/mcp-context/spec.md), [TRUST-01–TRUST-05](specs/knowledge-trust/spec.md), [REVIEW-01–REVIEW-04](specs/correction-review/spec.md) |
| G-06: подробная память, в том числе вне текущих запросов | [RUNTIME-02](specs/atlas-runtime/spec.md), [GRAPH-02–GRAPH-03](specs/code-graph/spec.md), [RETURN-01–RETURN-03](specs/return-analysis/spec.md), [INDEX-01](specs/project-indexing/spec.md); состав хранения не определяется наличием запроса MCP |
| G-07: наглядная аналитика использования графа | [ANALYTICS-01–ANALYTICS-04](specs/graph-usage-analytics/spec.md); рабочая базовая метрика — выдача контекста |

## Нумерованные требования

| Источник | Содержание | Требования OpenSpec |
| --- | --- | --- |
| D-01 | Отдельный контейнер с доступом к проекту | [RUNTIME-01](specs/atlas-runtime/spec.md) |
| D-02 | Основа PostgreSQL и бинарник в одном контейнере | [RUNTIME-01](specs/atlas-runtime/spec.md) |
| D-03 | Установленный pgvector | [RUNTIME-01](specs/atlas-runtime/spec.md) |
| D-04 | Хранение в PostgreSQL с индексами | [RUNTIME-02](specs/atlas-runtime/spec.md) |
| D-05 | MCP и веб-интерфейс на порту с проверкой исправлений и аналитикой | [MCP-01](specs/mcp-context/spec.md), [REVIEW-01](specs/correction-review/spec.md), [ANALYTICS-01](specs/graph-usage-analytics/spec.md) |
| D-06 | Конфигурация приложения через ENV в docker-compose | [RUNTIME-05](specs/atlas-runtime/spec.md) |
| R-01 | Расположение и строка return | [RETURN-01](specs/return-analysis/spec.md) |
| R-02 | Выражение или значение | [RETURN-01](specs/return-analysis/spec.md) |
| R-03 | Тип каждого случая | [RETURN-01](specs/return-analysis/spec.md) |
| R-04 | Условия возврата и if | [RETURN-02](specs/return-analysis/spec.md) |
| R-05 | Небольшой фрагмент вокруг return | [RETURN-03](specs/return-analysis/spec.md) |
| R-06 | Происхождение значения без дополнительного текстового поиска | [RETURN-03](specs/return-analysis/spec.md) |
| H-01 | Последняя правка строки или участка | [GIT-01](specs/git-provenance/spec.md) |
| H-02 | Предыдущее и текущее содержимое | [GIT-02](specs/git-provenance/spec.md) |
| H-03 | Различение добавления и изменения | [GIT-02](specs/git-provenance/spec.md) |
| H-04 | Контекст выше и ниже | [GIT-02](specs/git-provenance/spec.md) |
| H-05 | Более широкая правка метода | [GIT-03](specs/git-provenance/spec.md) |
| H-06 | Хеш и название или сообщение коммита | [GIT-01](specs/git-provenance/spec.md) |
| A-01 | Наглядная аналитика элементов и частоты | [ANALYTICS-01](specs/graph-usage-analytics/spec.md) |
| A-02 | Явное определение измеряемого события | [ANALYTICS-04](specs/graph-usage-analytics/spec.md) |
| A-03 | Рабочий учёт выдач контекста и включений элементов | [ANALYTICS-02](specs/graph-usage-analytics/spec.md); уточняется в Q-20 и Q-22 |
| A-04 | Сводка, динамика и детализация за период | [ANALYTICS-03](specs/graph-usage-analytics/spec.md); рабочая конкретизация представления |

## Подходы к реализации в порядке приоритета

Подходы регулируют инженерный процесс и архитектуру, поэтому закреплены в [AGENTS.md](../../../AGENTS.md) и [правилах OpenSpec](../../config.yaml), а не оформлены как пользовательские функции приложения.

| Приоритет из раздела 15 | Подход | Как проверять применение при реализации |
| --- | --- | --- |
| 1 | Design architecture first | В design.md до реализации описаны границы, обязанности, зависимости и контракты для затрагиваемого поведения. |
| 2 | Design patterns | Для выбранного паттерна названы решаемая задача и причина выбора; конкретные паттерны заранее не предписаны. |
| 3 | TDD | Для исполняемого поведения зафиксирован цикл падающего поведенческого теста, минимальной реализации и рефакторинга с проходящими проверками. |
| 4 | Service Layer | MCP и веб-обработчики используют слой прикладных сценариев; общие правила не дублируются в точках входа. |
| 5 | SOLID + DRY + KISS + YAGNI | При проверке кода оцениваются ответственности, зависимости, отсутствие дублирования правил и необходимость абстракций для текущих требований. |
| 6 | Clean Simple Code | Имена и поток управления понятны, операции связны, обработка ошибок последовательна. |

## Требования без исходных идентификаторов

| Раздел ТЗ | Содержание | Требования OpenSpec |
| --- | --- | --- |
| 2.2 | Старая и новая реализации общего интерфейса | [GRAPH-06](specs/code-graph/spec.md) |
| 3.2 | Транзакции всех источников и откат | [RUNTIME-03](specs/atlas-runtime/spec.md) |
| 3.3 | ENV при запуске и изменение конфигурации без пересборки | [RUNTIME-05](specs/atlas-runtime/spec.md), [Q-23](open-questions.md) |
| 4.1 | Граф и типизированные рёбра, начало снизу | [GRAPH-01](specs/code-graph/spec.md) |
| 4.2, 4.4 | Переменные, функции, методы, классы и их сведения | [GRAPH-02–GRAPH-03](specs/code-graph/spec.md) |
| 4.2 | Доверие вершины и ребра отдельно | [TRUST-01](specs/knowledge-trust/spec.md), [MCP-02](specs/mcp-context/spec.md) |
| 4.3 | Принадлежность, вызовы, количество, наследование и доступность | [GRAPH-01, GRAPH-04–GRAPH-06](specs/code-graph/spec.md) |
| 5.1 | Все return, включая разные типы legacy-кода | [RETURN-01](specs/return-analysis/spec.md) |
| 5.2 | Источник значения за пределами короткого фрагмента | [RETURN-03](specs/return-analysis/spec.md) |
| 6.2 | Исправление атласа отдельно от правки исходников | [MCP-03](specs/mcp-context/spec.md) |
| 7.1 | Постоянное наблюдение, исключения и метаданные Git | [INDEX-01–INDEX-02](specs/project-indexing/spec.md) |
| 7.2 | Настраиваемая задержка | [INDEX-03](specs/project-indexing/spec.md) |
| 7.3 | Встроенный разбор без PHP CLI (уточнение v0.4) | [INDEX-04](specs/project-indexing/spec.md) |
| 7.4 | Ограничения CPU и RAM | [RUNTIME-04](specs/atlas-runtime/spec.md) |
| 8 | Расширение языками и фреймворками | [PLUGIN-01](specs/analyzer-plugins/spec.md) |
| 8.1 | PHP, наследование, трейты, parent и магия | [PLUGIN-02–PLUGIN-03](specs/analyzer-plugins/spec.md) |
| 8.2 | Yii 2, магические обращения и сведения из vendor | [PLUGIN-04](specs/analyzer-plugins/spec.md) |
| 9.1–9.2 | MCP, контекст, исправления, автоматическое обновление | [MCP-01–MCP-03](specs/mcp-context/spec.md), [INDEX-01](specs/project-indexing/spec.md) |
| 10.1 | Приоритеты источников и немедленное действие уровня 2 | [TRUST-01–TRUST-02](specs/knowledge-trust/spec.md) |
| 10.2 | Полномочия создания, обновления и удаления | [TRUST-03](specs/knowledge-trust/spec.md) |
| 10.3 | Защита от обновления меньшим уровнем | [TRUST-04](specs/knowledge-trust/spec.md) |
| 10.4 | Удаление исходного кода выше доверия | [TRUST-05](specs/knowledge-trust/spec.md) |
| 11 | Веб-проверка и сведения для отмены | [REVIEW-01](specs/correction-review/spec.md) |
| 11.1 | approve, decline, отсутствие решения | [REVIEW-02–REVIEW-04](specs/correction-review/spec.md) |
| 11.2, 13 | Нерешённые детали | [open-questions.md](open-questions.md) |
| 14 | Идеи, не включённые в обязательный объём | [open-questions.md](open-questions.md), раздел «Что не становится обязательным при переносе в OpenSpec» |
| 15 | Приоритеты инженерных подходов | [AGENTS.md](../../../AGENTS.md), [config.yaml](../../config.yaml), таблица подходов выше |
| 16 | Веб-аналитика и смысл использования | [ANALYTICS-01–ANALYTICS-04](specs/graph-usage-analytics/spec.md), [Q-20–Q-22](open-questions.md) |

## Проверочные сценарии ТЗ

| № из раздела 12 | Сценарий OpenSpec | Спецификация |
| --- | --- | --- |
| 1 | Одноимённые методы | [GRAPH-04](specs/code-graph/spec.md) |
| 2 | Старая и новая платёжные реализации | [GRAPH-06](specs/code-graph/spec.md) |
| 3 | Возвраты разных типов; Возврат внутри вложенных условий; Локальный фрагмент возврата | [RETURN-01–RETURN-03](specs/return-analysis/spec.md) |
| 4 | Наследование и переопределение; Метод трейта доступен нескольким классам | [GRAPH-04](specs/code-graph/spec.md), [PLUGIN-02](specs/analyzer-plugins/spec.md) |
| 5 | Конструктор вызывает родительский конструктор | [PLUGIN-02](specs/analyzer-plugins/spec.md) |
| 6 | Магическое свойство реализовано в vendor | [PLUGIN-04](specs/analyzer-plugins/spec.md) |
| 7 | Серия изменений файла | [INDEX-03](specs/project-indexing/spec.md) |
| 8 | Проверка включена и синтаксис ошибочен | [INDEX-04](specs/project-indexing/spec.md) |
| 9 | Ошибка во время обновления | [RUNTIME-03](specs/atlas-runtime/spec.md) |
| 10 | Исправление сведений анализатора; Исправление ожидает проверки | [MCP-03](specs/mcp-context/spec.md), [TRUST-02](specs/knowledge-trust/spec.md) |
| 11 | Переиндексация после исправления агента; Переиндексация после подтверждения разработчика | [TRUST-04](specs/knowledge-trust/spec.md) |
| 12 | Подтверждение вершины или ребра | [REVIEW-02](specs/correction-review/spec.md) |
| 13 | Исправление агентом подтверждённых данных | [TRUST-04](specs/knowledge-trust/spec.md) |
| 14 | Отклонение исправления агента | [REVIEW-03](specs/correction-review/spec.md); точное восстановление зависит от Q-05 |
| 15 | Удалён код исправленного или подтверждённого объекта | [TRUST-05](specs/knowledge-trust/spec.md); критерий удаления зависит от Q-01–Q-02 |
| 16 | Запрет создания и удаления для агента и разработчика | [TRUST-03](specs/knowledge-trust/spec.md) |
| 17 | Добавление нового кода; Изменение существовавшего кода | [GIT-02](specs/git-provenance/spec.md) |
| 18 | Частота использования элементов; Одноимённые методы и разные связи | [ANALYTICS-01](specs/graph-usage-analytics/spec.md) |
| 19 | Два ответа с пересекающимся контекстом; Есть выдача без отчёта агента | [ANALYTICS-02, ANALYTICS-04](specs/graph-usage-analytics/spec.md); базовая метрика является рабочим допущением |
| 20 | Согласованные показатели за период | [ANALYTICS-03](specs/graph-usage-analytics/spec.md) |
| 21 | Запуск с настройками из docker-compose | [RUNTIME-05](specs/atlas-runtime/spec.md) |
| 22 | Изменение конфигурации без пересборки | [RUNTIME-05](specs/atlas-runtime/spec.md) |

Сценарии, зависящие от ещё не выбранной семантики, фиксируют только подтверждённую часть поведения. Например, отмена правки обязательна, но конкретное восстановленное значение и его уровень пока не заданы.

## Дополнение v0.5

| Источник | Поведение | Спецификация |
| --- | --- | --- |
| Уточнение v0.5, D-07 | Публикация готового Docker-образа и подключение в чужой Compose через image | [RUNTIME-06](specs/atlas-runtime/spec.md) |
