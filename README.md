# Bluetooth Beacon Consumer

> Rust passive BLE monitor for Govee weather sensors; logs readings and exposes a local HTTP API.

## Summary

Bluetooth Beacon Consumer is a Rust application that passively listens for BLE
advertisements from Govee weather sensors, decodes their temperature and humidity
payloads, and writes one CSV per sensor per month under `sensor_logs/`. A companion
dashboard binary serves a JSON API plus a built React frontend on `127.0.0.1:3000`
for live and historical analysis.

The collector handles two Govee payload layouts (A and B), rate-limits per-device
writes to once every 60 seconds, and archives CSVs older than three months as
`.csv.gz`. The dashboard reads both plain and archived files and supports
filtering and offset paging for large histories.

## Capabilities

- **Passive BLE collection** — listens for Govee advertisements and decodes layout A and B payloads.
- **Per-sensor monthly CSV logs** — one file per device per month under `sensor_logs/`, file-locked against concurrent writes.
- **Per-device rate limiting** — caps each sensor's write cadence to once every 60 seconds.
- **Automatic archival** — gzips CSV files older than three months on new-file initialization.
- **Local HTTP API** — Axum-based JSON API exposes sensor inventory and historical rows on `127.0.0.1:3000`.
- **Bundled dashboard frontend** — React + Vite UI with live polling, paged history, charts, and time filters.

## Status

Two Rust binaries (`bluetooth-beacon-consumer`, `dashboard`) plus a `ui/` React
frontend, edition 2024. Requires a host Bluetooth adapter for the collector.

## Prerequisites

- Rust toolchain (edition 2024; stable recommended)
- Node.js 18+ for building the frontend
- A Bluetooth adapter enabled on the host for the collector runtime

## Setup

Build the UI once before running the dashboard:

```bash
cd ui
npm ci
npm run build
```

## Usage

Run the collector (writes sensor logs):

```bash
cargo run
```

Output files are written to:

```text
sensor_logs/YYYY_MM_<DeviceName>_<Address>.csv
```

Run the dashboard / API:

```bash
cargo run --bin dashboard
```

Then open `http://127.0.0.1:3000`.

### Frontend behavior

- Sensor selection driven by `/api/sensors`.
- Initial load from `/api/sensors/:id/data?limit=10000&offset=0`.
- Live polling every 10s using `since=<latest_timestamp>`.
- "Load Full History" enables archive inclusion (`all=true`).
- "Load More Rows" follows `next_offset` paging.
- Time filters: `24h`, `7d`, `30d`, `All` (client-side filter).

## HTTP API

### `GET /api/sensors`

Returns available sensor IDs and archive availability.

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

- Unknown sensor ID returns `404`.
- The timestamp column is always returned as a string.
- Non-finite numbers (for example `NaN`) remain strings in JSON.
- Malformed or unreadable files produce `partial=true` with warning text.

### Auth posture

The service is auth-neutral — no built-in login flow — and is commonly run behind
gateway-managed auth in some environments. Compatibility rules:

- Preserve route and response contracts used by gateway and clients.
- Do not introduce mandatory app-level auth by default.
- Do not add redirect-based login behavior to API handlers.
- Remain compatible with forwarded `Authorization` headers.

## Architecture

The collector writes the CSV header once per file. Writes are file-locked to
avoid concurrent write corruption. Archive policy: a file's month must be
older than three months from the current month, and archival happens during
new-file initialization.

## Testing

```bash
cargo fmt
cargo check
cargo test
```

Coverage includes:

- Govee payload decode/validation paths
- Append and archive file behaviors
- API-level behavior (404, paging, filtering, archive inclusion, warnings)

For manual end-to-end validation, see the production-style runbook in
[MANUAL_TESTS.md](./MANUAL_TESTS.md).
