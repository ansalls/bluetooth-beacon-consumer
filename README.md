# Bluetooth Beacon Consumer

BLE passive monitor for Govee weather sensors with:

- real-time CSV logging (`sensor_logs/`)
- automatic archival of old monthly files (`.csv.gz`)
- a local dashboard and JSON API for live/historical analysis

## Current State

The project currently ships two Rust binaries and one frontend:

- `src/main.rs` (`cargo run`)
  - BLE collector/logger
  - Govee payload parsing (layout A/B)
  - one CSV per sensor per month
  - per-device write rate limiting (60s)
  - archive sweep for files older than 3 months
- `src/bin/dashboard.rs` (`cargo run --bin dashboard`)
  - serves API + built frontend on `127.0.0.1:3000`
  - reads both `.csv` and `.csv.gz`
  - supports filtering and paging for large history
- `ui/` (React + Vite)
  - live polling every 10s
  - load full history + load more paging
  - chart cards + summary cards + sortable/paged table

## Requirements

- Rust toolchain (edition 2024; stable recommended)
- Node.js 18+ (for frontend build)
- Bluetooth adapter enabled for collector runtime

## Quick Start

### 1) Build the UI

```bash
cd ui
npm ci
npm run build
```

### 2) Run the collector (writes sensor logs)

```bash
cargo run
```

Output files are written to:

```text
sensor_logs/YYYY_MM_<DeviceName>_<Address>.csv
```

### 3) Run dashboard/API

```bash
cargo run --bin dashboard
```

Open:

```text
http://127.0.0.1:3000
```

## API

### `GET /api/sensors`

Returns available sensor IDs and archive availability.

Example:

```json
[
  { "id": "GVH5075_A4_C1_38_52_90_85", "has_archives": true },
  { "id": "GVH5102_DA_01_02_03_04_05", "has_archives": false }
]
```

### `GET /api/sensors/:id/data`

Query params:

- `all=true|false` (default `false`): include archives
- `since=<timestamp>`: inclusive timestamp filter
- `limit=<n>`: server row cap per response (`1..50000`, default `10000`)
- `offset=<n>`: row offset for paging

Response:

```json
{
  "columns": ["timestamp", "temperature_c", "humidity_pct"],
  "rows": [["2026-03-11 10:00:00.000", 21.5, 45.2]],
  "has_archives": true,
  "partial": false,
  "next_offset": null,
  "warnings": []
}
```

Notes:

- unknown sensor ID returns `404`
- timestamp column is always returned as string
- non-finite numbers (for example `NaN`) remain strings in JSON
- malformed/unreadable files produce `partial=true` with warning text

## Frontend behavior

- Sensor selection from `/api/sensors`
- Initial load from `/api/sensors/:id/data?limit=10000&offset=0`
- Live polling every 10s using `since=<latest_timestamp>`
- "Load Full History" enables archive inclusion (`all=true`)
- "Load More Rows" follows `next_offset` paging
- Time filters: `24h`, `7d`, `30d`, `All` (client-side filter)

## Logging and archival behavior

- Collector writes CSV header once per file.
- Writes are file-locked to avoid concurrent write corruption.
- Archive policy: file month must be older than 3 months from current month.
- Archival happens during new-file initialization.

## Tests

Run:

```bash
cargo fmt
cargo check
cargo test
```

Coverage includes:

- Govee payload decode/validation paths
- append and archive file behaviors
- API-level behavior (404, paging, filtering, archive inclusion, warnings)

## OATH/OAuth compatibility

This service is commonly run behind gateway-managed auth in some environments.
Current app behavior is auth-neutral (no built-in login flow). Compatibility rules:

- preserve route and response contracts used by gateway and clients
- do not introduce mandatory app-level auth by default
- do not add redirect-based login behavior to API handlers
- remain compatible with forwarded `Authorization` headers

## Manual end-to-end validation

See the full production-style test runbook:

- [MANUAL_TESTS.md](./MANUAL_TESTS.md)
