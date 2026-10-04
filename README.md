# rust-harness

Минимальная локальная обвязка для LLM-агентов на Rust.

Сейчас поддерживает:

- Ollama как локальный сервер моделей;
- Qwen и любые модели, доступные через Ollama;
- цикл агент → tool → результат → агент;
- встроенные инструменты для чтения, записи и просмотра файлов;
- MCP-клиенты через stdio;
- отдельный конфиг для будущих адаптеров (Roblox, 1С и т.д.).

## Установка

Нужны Rust и Ollama.

```bash
ollama serve
ollama pull qwen2.5:7b
cp harness.toml.example harness.toml
cargo run -- models
```

## Первый запуск

```bash
cargo run -- run "посмотри файлы проекта и кратко объясни его структуру"
```

По умолчанию рабочая директория — текущая. Файловые инструменты не позволяют агенту выйти за её пределы.

## MCP

MCP-сервер подключается в `harness.toml`:

```toml
[[mcp]]
name = "example"
command = "npx"
args = ["some-mcp-server"]
```

Harness запускает сервер через stdio, выполняет `initialize`, получает `tools/list` и затем может вызывать `tools/call`.

MCP является расширением поверх ядра, а не частью конкретного типа проекта.

## Архитектура

```text
LLM (Ollama)
      │
      ▼
 Rust Harness
 ┌────┼──────────────┐
 │    │              │
agent native tools  MCP client
 │    │              │
 │    └── filesystem └── external MCP servers
 │
 └── task loop
```

Следующие слои можно добавлять независимо: Git, shell/test runner, память, планировщик, project adapters и более строгий structured tool calling.
