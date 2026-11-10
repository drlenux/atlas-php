# Proposal: yii2-events

## Why

`Component::trigger(EVENT_*)` резолвится в `yii\base\Component::trigger`, без связи с обработчиками. Подписки `Event::on(Class, EVENT_*, handler)` есть в коде (HandlePayPlace), но trigger→handler в графе нет.

## What

Yii2 enrich:

1. Скан `Event::on` / `\yii\base\Event::on` (positional + named `class:`/`name:`/`handler:`).
2. Узлы `kind: event` (`event:<Owner>::<EVENT_CONST>`).
3. `handles`: event → method handler (`[Cls,'m']` или `Cls::m` внутри `static fn`).
4. `calls` с `framework=yii2` с call_site `->trigger(…EVENT_…)` на каждый handler — `called_by` на обработчике показывает trigger.

## Also

- `$obj->on($name, $handler)` / `$this->on` (в т.ч. `[$this,'m']`).
- `Event::off` / `$obj->off` → рёбра `unsubscribes`; handlers с парой on+off сохраняются для trigger (register/unregister).
- Имена: `EVENT_*` и строковые литералы; матч через `metadata.value` константы.
