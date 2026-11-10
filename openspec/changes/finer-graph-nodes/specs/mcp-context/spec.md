# MCP context — finer nodes

## ADDED Requirements

### Requirement: MCP-04 Контекст опирается на мелкие вершины

При запросе контекста метода ответ SHALL включать связанные occurrence/branch через `contains` и их рёбра эффекта на границе глубины, без полного текста тела метода в metadata method-ноды.

#### Scenario: Исследование метода через MCP

- **WHEN** агент запрашивает контекст метода с depth >= 1
- **THEN** ответ позволяет определить исследуемый метод, его occurrence/branch и исходящие `calls`/`reads`/`writes`
- **AND** нода метода в ответе не содержит полного текста тела
