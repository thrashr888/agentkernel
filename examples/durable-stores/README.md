# Durable Store Templates

Starter payload templates for creating durable stores via the HTTP API.

## Create Stores

```bash
curl -X POST http://localhost:18888/stores \
  -H "Content-Type: application/json" \
  -d @examples/durable-stores/sqlite-store.json
```

```bash
curl -X POST http://localhost:18888/stores \
  -H "Content-Type: application/json" \
  -d @examples/durable-stores/postgres-store.json
```

```bash
curl -X POST http://localhost:18888/stores \
  -H "Content-Type: application/json" \
  -d @examples/durable-stores/mysql-store.json
```

```bash
curl -X POST http://localhost:18888/stores \
  -H "Content-Type: application/json" \
  -d @examples/durable-stores/redis-store.json
```

## Execute Operations

SQLite/Postgres (query):

```bash
curl -X POST http://localhost:18888/stores/<store-id>/query \
  -H "Content-Type: application/json" \
  -d '{"sql":"SELECT 1 as ok","params":[]}'
```

SQLite/Postgres (execute):

```bash
curl -X POST http://localhost:18888/stores/<store-id>/execute \
  -H "Content-Type: application/json" \
  -d '{"sql":"CREATE TABLE IF NOT EXISTS items(id INTEGER PRIMARY KEY, name TEXT)","params":[]}'
```

Redis (command):

```bash
curl -X POST http://localhost:18888/stores/<store-id>/command \
  -H "Content-Type: application/json" \
  -d '{"command":["PING"]}'
```
