# Manual End-to-End Test Plan

This runbook validates collector, API, and dashboard behavior from a production-user perspective.
It is written so a human or user-agent can execute it step-by-step and record pass/fail outcomes.

## 1) Scope

Validated components:

- BLE collector (`cargo run`)
- dashboard/API (`cargo run --bin dashboard`)
- frontend UX served by dashboard

Validated API endpoints:

- `GET /api/sensors`
- `GET /api/sensors/:id/data`

## 2) Test Environment

- OS: Windows/Linux/macOS (commands below include PowerShell examples)
- Rust + Node installed
- Project root checked out
- Clean test folder:
  - remove old test fixtures from `sensor_logs/`

## 3) Build and Start

1. Build frontend:
   - `cd ui && npm ci && npm run build`
2. Start dashboard:
   - `cargo run --bin dashboard`
3. (For collector tests) start collector in separate terminal:
   - `cargo run`

Expected:

- Dashboard listens on `http://127.0.0.1:3000`
- UI loads with no JS console errors

## 4) Controlled Fixture Setup (API/UI tests)

Run from repo root (PowerShell):

```powershell
New-Item -ItemType Directory -Force sensor_logs | Out-Null
Remove-Item sensor_logs\* -Force -ErrorAction SilentlyContinue

# Current month file
@'
timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm
2026-03-11 10:00:00.000,alpha,A4,20.0,68.0,40.0,90,-60
2026-03-11 10:01:00.000,alpha,A4,20.1,68.2,NaN,90,-59
2026-03-11 10:02:00.000,alpha,A4,20.2,68.4,41.0,89,-58
'@ | Set-Content sensor_logs\2026_03_sensor_alpha.csv

# Another sensor current month
@'
timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm
2026-03-11 10:00:00.000,beta,B5,19.5,67.1,50.0,75,-66
'@ | Set-Content sensor_logs\2026_03_sensor_beta.csv

# Malformed historical CSV (for partial warning test)
@'
timestamp,device_name,humidity_pct
"bad
'@ | Set-Content sensor_logs\2025_11_sensor_alpha.csv

# Valid archived CSV.GZ for alpha
$csv = @'
timestamp,device_name,device_address,temperature_c,temperature_f,humidity_pct,battery_pct,rssi_dbm
2025-12-15 08:00:00.000,alpha,A4,18.0,64.4,45.0,91,-61
'@
$bytes = [System.Text.Encoding]::UTF8.GetBytes($csv)
$fs = [System.IO.File]::Create("sensor_logs\2025_12_sensor_alpha.csv.gz")
$gz = New-Object System.IO.Compression.GzipStream($fs,[System.IO.Compression.CompressionMode]::Compress)
$gz.Write($bytes,0,$bytes.Length); $gz.Close(); $fs.Close()
```

## 5) API Validation Matrix

Record each test as `PASS`/`FAIL` with evidence (response snippet/screenshot).

### A. Sensor Discovery Endpoint (`GET /api/sensors`)

1. `A1 - Basic listing`
   - Step: `Invoke-RestMethod http://127.0.0.1:3000/api/sensors`
   - Expect:
     - HTTP 200
     - includes `sensor_alpha` and `sensor_beta`
     - sorted ascending by `id`
2. `A2 - Archive flag correctness`
   - Expect:
     - `sensor_alpha.has_archives == true`
     - `sensor_beta.has_archives == false`
3. `A3 - Empty logs directory behavior`
   - Step: temporarily empty `sensor_logs/`, call endpoint again
   - Expect: `[]`, HTTP 200

### B. Sensor Data Endpoint (`GET /api/sensors/:id/data`)

4. `B1 - Unknown sensor returns 404`
   - Step: call `/api/sensors/does_not_exist/data`
   - Expect: HTTP 404
5. `B2 - Default fetch excludes archives`
   - Step: `/api/sensors/sensor_alpha/data`
   - Expect:
     - HTTP 200
     - only current-month rows returned
     - `has_archives == true`
6. `B3 - all=true includes archives`
   - Step: `/api/sensors/sensor_alpha/data?all=true`
   - Expect:
     - rows include current and archived data
7. `B4 - since filter inclusive`
   - Step: `/api/sensors/sensor_alpha/data?since=2026-03-11%2010:01:00.000`
   - Expect:
     - row with `10:01:00.000` included
     - older rows excluded
8. `B5 - since RFC3339 accepted`
   - Step: `/api/sensors/sensor_alpha/data?all=true&since=2025-12-15T08:00:00Z`
   - Expect: archived boundary row included
9. `B6 - limit clamp low`
   - Step: `/api/sensors/sensor_alpha/data?limit=0`
   - Expect: exactly 1 row returned (clamped to 1)
10. `B7 - limit normal`
    - Step: `/api/sensors/sensor_alpha/data?limit=2`
    - Expect:
      - `rows.length == 2`
      - `partial == true`
      - `next_offset == 2`
11. `B8 - offset paging`
    - Step: `/api/sensors/sensor_alpha/data?limit=2&offset=2`
    - Expect:
      - returns remaining rows after first page
      - `next_offset` null when no more data
12. `B9 - offset beyond end`
    - Step: `/api/sensors/sensor_alpha/data?offset=999999`
    - Expect:
      - `rows == []`
      - HTTP 200
13. `B10 - warning for truncated results`
    - Step: `/api/sensors/sensor_alpha/data?limit=1`
    - Expect:
      - `partial == true`
      - `warnings` contains truncation message
14. `B11 - warning for unreadable/malformed historical file`
    - Step: `/api/sensors/sensor_alpha/data?all=true&limit=100`
    - Expect:
      - `partial == true`
      - `warnings` contains `Could not read ...`
15. `B12 - NaN remains string`
    - Step: request data and inspect second row `humidity_pct`
    - Expect:
      - value is string `"NaN"` (not JSON number)
16. `B13 - timestamp remains string`
    - Expect:
      - first column values are strings
17. `B14 - numeric coercion for non-first columns`
    - Expect:
      - numeric fields are numbers when finite (for example `temperature_c`)
18. `B15 - unknown query params ignored safely`
    - Step: `/api/sensors/sensor_alpha/data?foo=bar&limit=2`
    - Expect: same behavior as `limit=2` call
19. `B16 - path traversal-like id rejected`
    - Step: `/api/sensors/%2E%2E%2Fsecret/data`
    - Expect: HTTP 404
20. `B17 - CORS preflight`
    - Step: send OPTIONS with `Origin` + `Access-Control-Request-Method: GET`
    - Expect: permissive CORS headers present

## 6) UI End-to-End Validation Matrix

Open `http://127.0.0.1:3000`.

1. `U1 - Sensor selector`
   - Expect both sensors shown; alpha marked as having history.
2. `U2 - Initial load state`
   - Expect loading indicator then populated dashboard.
3. `U3 - Summary cards`
   - Expect cards for numeric columns with latest values.
4. `U4 - Charts render`
   - Expect one chart per numeric metric; tooltips show readable values.
5. `U5 - Time range buttons`
   - Click `24h`, `7d`, `30d`, `All`
   - Expect row/chart data filter changes accordingly.
6. `U6 - Load Full History button`
   - On alpha before all history, button visible.
   - Click it; expect archived data appears and badge says full history loaded.
7. `U7 - Load More Rows button (paging)`
   - Force truncated response (small limit in backend fixture if needed).
   - Expect button appears when `next_offset` exists.
   - Clicking appends additional rows without duplicates.
8. `U8 - Warning banner surfacing`
   - With malformed file present, expect info banner with warning text.
9. `U9 - Error banner handling`
   - Temporarily stop dashboard API or break endpoint request.
   - Expect error banner, no crash.
10. `U10 - Table sort`
    - Click column header multiple times.
    - Expect ascending/descending toggles and stable rendering.
11. `U11 - Table pagination`
    - With >50 rows, expect page controls and correct page counts.
12. `U12 - Live polling update`
    - Append a newer CSV row to selected sensor.
    - Wait ~10s.
    - Expect new row appears without manual refresh.
13. `U13 - Empty-state handling`
    - Clear `sensor_logs/`, reload.
    - Expect "No sensors found in sensor_logs/".
14. `U14 - Responsive behavior`
    - Test desktop and mobile widths.
    - Expect no broken layout or clipped controls.

## 7) Collector Runtime Validation Matrix

These require a BLE adapter and an active Govee sensor broadcast.

1. `C1 - Collector startup`
   - `cargo run`
   - Expect startup banner and scan begins.
2. `C2 - Device detection`
   - Expect readings from Govee sensor appear in logs.
3. `C3 - File creation format`
   - Expect file name: `YYYY_MM_<sanitized_device>_<sanitized_address>.csv`
4. `C4 - Header written once`
   - After multiple writes, verify only one header row.
5. `C5 - Write throttling`
   - Confirm same device is not logged more than once per 60s.
6. `C6 - Battery/rssi optional handling`
   - Missing values result in empty CSV fields, not malformed row.
7. `C7 - Archive sweep behavior`
   - Place an old CSV (`YYYY_MM`) older than 3 months.
   - Trigger a new file creation event.
   - Expect old file compressed to `.csv.gz` and original removed.

## 8) OATH/OAuth Compatibility Validation

If deployed behind gateway auth, run these smoke checks:

1. `O1 - Authorization header pass-through safety`
   - Call API with `Authorization: Bearer test-token`.
   - Expect same API behavior (no app-level auth failure by default).
2. `O2 - No redirect-based auth challenge`
   - API should not return HTML login redirects.
   - Expect JSON/HTTP status behavior only.
3. `O3 - Endpoint contract stability under gateway`
   - Through gateway route, verify `/api/sensors` and `/api/sensors/:id/data` payload schema unchanged.
4. `O4 - CORS and preflight at edge`
   - Confirm gateway + app preserve required CORS headers for browser use.

## 9) Exit Criteria

Release candidate is acceptable when:

- all API tests `A1..B17` pass
- all UI tests `U1..U14` pass
- all collector tests `C1..C7` pass (or documented N/A in non-BLE env)
- all OATH/OAuth compatibility tests `O1..O4` pass in gated environment
- no critical UI/API regression observed during 30-minute soak run

## 10) Evidence to capture

- command outputs for each API test
- screenshots for key UI states (initial, full history, warning banner, error banner)
- sample CSV before/after archive sweep
- short test summary table with pass/fail and defect references
