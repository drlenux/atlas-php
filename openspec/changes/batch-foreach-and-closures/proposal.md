# Proposal: batch foreach + closure outline

## Why

Отчёт 0.1.16: атлас пропускал 2 записи `UserBonus::$status` в `ServantController` из‑за `foreach ($q->batch() as $rows) foreach ($rows as $bonus)`; outline `activateLadderBonus` не показывал вызовы внутри `$credit = function…`; `($this->credit)()` был unresolved.

## What

- `ActiveQuery<T>::batch()` → `T[][]`, `each()` → `T[]` (vendor PHPDoc даёт `BatchQueryResult`).
- Outline метода включает call_site вложенных closure.
- `($expr)()` парсится как `__invoke` на callee; Closure::__invoke — builtin external.

## Out of scope

- Points-to: какой именно closure лежит в `$this->credit` после передачи в Gateway.
- trigger → event handler.
