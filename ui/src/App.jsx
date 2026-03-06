import { useState, useEffect, useMemo, useCallback } from 'react';
import {
  LineChart, Line, XAxis, YAxis, CartesianGrid,
  Tooltip, ResponsiveContainer, Legend,
} from 'recharts';

// ── Constants ─────────────────────────────────────────────────────────────────

const CHART_COLORS = [
  '#2563EB', '#059669', '#DC2626', '#D97706', '#7C3AED', '#0891B2', '#DB2777',
];

// Well-known column metadata for sensor-type hints; falls back gracefully for
// unknown columns so any CSV schema renders correctly.
const COLUMN_META = {
  temperature_c: { label: 'Temperature',    unit: '°C',  color: '#DC2626', decimals: 1 },
  temperature_f: { label: 'Temperature',    unit: '°F',  color: '#F97316', decimals: 1 },
  humidity_pct:  { label: 'Humidity',       unit: '%',   color: '#2563EB', decimals: 1 },
  battery_pct:   { label: 'Battery',        unit: '%',   color: '#059669', decimals: 0 },
  rssi_dbm:      { label: 'Signal Strength',unit: ' dBm',color: '#7C3AED', decimals: 0 },
};

const TIME_RANGES = [
  { label: '24h', ms: 24 * 3600_000 },
  { label: '7d',  ms: 7 * 24 * 3600_000 },
  { label: '30d', ms: 30 * 24 * 3600_000 },
  { label: 'All', ms: null },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

function colLabel(col) {
  return COLUMN_META[col]?.label ?? col.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
}
function colUnit(col) { return COLUMN_META[col]?.unit ?? ''; }
function colDecimals(col) { return COLUMN_META[col]?.decimals ?? 2; }
function colColor(col, idx) { return COLUMN_META[col]?.color ?? CHART_COLORS[idx % CHART_COLORS.length]; }

function formatValue(val, col) {
  if (val == null || val === '') return '—';
  if (typeof val === 'number') return `${val.toFixed(colDecimals(col))}${colUnit(col)}`;
  return val;
}

// Parse "2026-03-05 17:24:30" → Date
function parseTimestamp(ts) {
  return ts ? new Date(ts.replace(' ', 'T')) : null;
}

function fmtAxisTick(ts) {
  const d = parseTimestamp(ts);
  if (!d || isNaN(d)) return ts;
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function fmtTooltipLabel(ts) {
  const d = parseTimestamp(ts);
  if (!d || isNaN(d)) return ts;
  return d.toLocaleString();
}

// Strip trailing MAC address from sensor ID for display.
// "GVH5075_9085_A4_C1_38_52_90_85" → "GVH5075_9085"
function sensorDisplayName(id) {
  return id.replace(/_[0-9A-Fa-f]{2}(_[0-9A-Fa-f]{2}){5}$/, '');
}

async function apiFetch(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${url}`);
  return res.json();
}

// ── Sub-components ────────────────────────────────────────────────────────────

function SummaryCard({ col, value, idx }) {
  const color = colColor(col, idx);
  return (
    <div className="card summary-card" style={{ borderTop: `3px solid ${color}` }}>
      <div className="summary-label">{colLabel(col)}{colUnit(col) ? ` (${colUnit(col).trim()})` : ''}</div>
      <div className="summary-value" style={{ color }}>
        {value != null ? value.toFixed(colDecimals(col)) : '—'}
      </div>
    </div>
  );
}

function CustomTooltip({ active, payload, label }) {
  if (!active || !payload?.length) return null;
  return (
    <div className="chart-tooltip">
      <div className="tooltip-ts">{fmtTooltipLabel(label)}</div>
      {payload.map(p => (
        <div key={p.dataKey} className="tooltip-row">
          <span className="tooltip-dot" style={{ background: p.color }} />
          <span className="tooltip-key">{colLabel(p.dataKey)}</span>
          <span className="tooltip-val">{formatValue(p.value, p.dataKey)}</span>
        </div>
      ))}
    </div>
  );
}

function MetricChart({ col, data, idx }) {
  const color = colColor(col, idx);
  const values = data.map(d => d[col]).filter(v => v != null);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const pad = Math.max((max - min) * 0.12, 0.5);
  const useDots = data.length <= 150;

  return (
    <div className="card chart-card">
      <div className="chart-card-header">
        <span className="chart-title">{colLabel(col)}</span>
        <span className="chart-unit" style={{ color }}>{colUnit(col)}</span>
      </div>
      <ResponsiveContainer width="100%" height={200}>
        <LineChart data={data} margin={{ top: 4, right: 16, left: 0, bottom: 0 }}>
          <CartesianGrid strokeDasharray="3 3" stroke="#E2E8F0" vertical={false} />
          <XAxis
            dataKey="timestamp"
            tickFormatter={fmtAxisTick}
            tick={{ fontSize: 11, fill: '#94A3B8' }}
            tickLine={false}
            axisLine={false}
            interval="preserveStartEnd"
          />
          <YAxis
            domain={[min - pad, max + pad]}
            tick={{ fontSize: 11, fill: '#94A3B8' }}
            tickLine={false}
            axisLine={false}
            tickFormatter={v => v.toFixed(colDecimals(col))}
            width={52}
          />
          <Tooltip content={<CustomTooltip />} />
          <Line
            type="monotone"
            dataKey={col}
            stroke={color}
            strokeWidth={2}
            dot={useDots ? { r: 2.5, fill: color, strokeWidth: 0 } : false}
            activeDot={{ r: 5, strokeWidth: 0 }}
            connectNulls
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

function DataTable({ columns, rows, numericCols }) {
  const PAGE = 50;
  const [page, setPage] = useState(0);
  const [sortCol, setSortCol] = useState(null);
  const [asc, setAsc] = useState(false);

  useEffect(() => setPage(0), [rows]);

  const sorted = useMemo(() => {
    const arr = [...rows];
    if (!sortCol) return arr.reverse();
    arr.sort((a, b) => {
      const av = a[sortCol], bv = b[sortCol];
      const cmp = typeof av === 'number' ? av - bv : String(av ?? '').localeCompare(String(bv ?? ''));
      return asc ? cmp : -cmp;
    });
    return arr;
  }, [rows, sortCol, asc]);

  const pages = Math.ceil(sorted.length / PAGE);
  const visible = sorted.slice(page * PAGE, (page + 1) * PAGE);

  function onSort(col) {
    if (sortCol === col) setAsc(a => !a);
    else { setSortCol(col); setAsc(true); }
  }

  const displayCols = columns.filter(c => c !== 'device_address');

  return (
    <div className="card table-card">
      <div className="table-header">
        <span className="chart-title">All Readings</span>
        <span className="table-count">{rows.length.toLocaleString()} rows</span>
      </div>
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              {displayCols.map(col => (
                <th key={col} onClick={() => onSort(col)} className={sortCol === col ? 'col-sorted' : ''}>
                  {colLabel(col)}
                  {sortCol === col ? (asc ? ' ↑' : ' ↓') : ''}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, i) => (
              <tr key={i}>
                {displayCols.map(col => (
                  <td key={col} className={numericCols.includes(col) ? 'td-num' : ''}>
                    {formatValue(row[col], col)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {pages > 1 && (
        <div className="pagination">
          <button onClick={() => setPage(p => Math.max(0, p - 1))} disabled={page === 0}>← Prev</button>
          <span className="page-indicator">Page {page + 1} of {pages}</span>
          <button onClick={() => setPage(p => Math.min(pages - 1, p + 1))} disabled={page === pages - 1}>Next →</button>
        </div>
      )}
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

export default function App() {
  const [sensors, setSensors]         = useState([]);
  const [selectedId, setSelectedId]   = useState(null);
  const [rawData, setRawData]         = useState(null);
  const [loading, setLoading]         = useState(false);
  const [error, setError]             = useState(null);
  const [timeRange, setTimeRange]     = useState('7d');
  const [allLoaded, setAllLoaded]     = useState(false);

  // Fetch sensor list on mount.
  useEffect(() => {
    apiFetch('/api/sensors')
      .then(data => {
        setSensors(data);
        if (data.length) setSelectedId(data[0].id);
      })
      .catch(e => setError(e.message));
  }, []);

  // Fetch data when sensor selection or allLoaded changes.
  useEffect(() => {
    if (!selectedId) return;
    setLoading(true);
    setError(null);
    const url = `/api/sensors/${encodeURIComponent(selectedId)}/data${allLoaded ? '?all=true' : ''}`;
    apiFetch(url)
      .then(({ columns, rows: rawRows, has_archives }) => {
        // Convert array rows → objects keyed by column name.
        const rows = rawRows.map(row => {
          const obj = {};
          columns.forEach((col, i) => { obj[col] = row[i]; });
          return obj;
        });
        setRawData({ columns, rows, has_archives });
        setLoading(false);
      })
      .catch(e => { setError(e.message); setLoading(false); });
  }, [selectedId, allLoaded]);

  // Reset archive state when switching sensors.
  useEffect(() => { setAllLoaded(false); setRawData(null); }, [selectedId]);

  const { filteredRows, numericCols, latestRow } = useMemo(() => {
    if (!rawData) return { filteredRows: [], numericCols: [], latestRow: null };
    const { columns, rows } = rawData;

    const numericCols = columns.filter(col => {
      if (col === 'timestamp') return false;
      const samples = rows.slice(0, 20).map(r => r[col]).filter(v => v != null);
      return samples.length > 0 && samples.every(v => typeof v === 'number');
    });

    const rangeMs = TIME_RANGES.find(r => r.label === timeRange)?.ms ?? null;
    const cutoff  = rangeMs ? Date.now() - rangeMs : null;

    const filteredRows = cutoff
      ? rows.filter(r => {
          const d = parseTimestamp(r.timestamp);
          return d && !isNaN(d) && d.getTime() >= cutoff;
        })
      : rows;

    const latestRow = rows[rows.length - 1] ?? null;
    return { filteredRows, numericCols, latestRow };
  }, [rawData, timeRange]);

  const sensorInfo   = sensors.find(s => s.id === selectedId);
  const hasArchives  = (sensorInfo?.has_archives ?? rawData?.has_archives) && !allLoaded;

  return (
    <div className="app">
      {/* ── Header ── */}
      <header className="app-header">
        <div className="header-inner">
          <div className="brand">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M5 12.55a11 11 0 0 1 14.08 0"/><path d="M1.42 9a16 16 0 0 1 21.16 0"/>
              <path d="M8.53 16.11a6 6 0 0 1 6.95 0"/><circle cx="12" cy="20" r="1" fill="currentColor"/>
            </svg>
            <span>Sensor Dashboard</span>
          </div>
          <div className="header-controls">
            {sensors.length > 0 && (
              <div className="sensor-select-wrap">
                <label className="sensor-select-label">Sensor</label>
                <select
                  className="sensor-select"
                  value={selectedId ?? ''}
                  onChange={e => setSelectedId(e.target.value)}
                >
                  {sensors.map(s => (
                    <option key={s.id} value={s.id}>
                      {sensorDisplayName(s.id)}{s.has_archives ? '  ·  has history' : ''}
                    </option>
                  ))}
                </select>
              </div>
            )}
          </div>
        </div>
      </header>

      {/* ── Main ── */}
      <main className="app-main">
        {error && <div className="banner banner-error">⚠ {error}</div>}

        {!selectedId && !loading && (
          <div className="empty-state">No sensors found in <code>sensor_logs/</code></div>
        )}

        {selectedId && !rawData && loading && (
          <div className="empty-state">
            <div className="spinner" />
            Loading sensor data…
          </div>
        )}

        {rawData && (
          <>
            {/* ── Toolbar ── */}
            <div className="toolbar">
              <div className="time-range-group">
                {TIME_RANGES.map(r => (
                  <button
                    key={r.label}
                    className={`range-btn${timeRange === r.label ? ' active' : ''}`}
                    onClick={() => setTimeRange(r.label)}
                  >
                    {r.label}
                  </button>
                ))}
              </div>
              <div className="toolbar-right">
                {loading && <span className="loading-pill">Refreshing…</span>}
                {hasArchives && (
                  <button className="btn-archive" onClick={() => setAllLoaded(true)}>
                    Load Full History
                  </button>
                )}
                {allLoaded && (
                  <span className="pill pill-green">Full history loaded</span>
                )}
                <span className="toolbar-count">
                  {filteredRows.length.toLocaleString()} readings
                </span>
              </div>
            </div>

            {/* ── Summary cards ── */}
            {latestRow && numericCols.length > 0 && (
              <div className="summary-grid">
                {numericCols.map((col, i) => (
                  <SummaryCard key={col} col={col} value={latestRow[col]} idx={i} />
                ))}
              </div>
            )}

            {/* ── Charts ── */}
            {filteredRows.length > 0 && (
              <div className="charts-grid">
                {numericCols.map((col, i) => (
                  <MetricChart key={col} col={col} data={filteredRows} idx={i} />
                ))}
              </div>
            )}

            {filteredRows.length === 0 && (
              <div className="banner banner-info">
                No readings in the selected time range. Try "All" or load full history.
              </div>
            )}

            {/* ── Table ── */}
            <DataTable
              columns={rawData.columns}
              rows={filteredRows}
              numericCols={numericCols}
            />
          </>
        )}
      </main>

      <footer className="app-footer">
        Sensor Dashboard · data from <code>sensor_logs/</code>
      </footer>
    </div>
  );
}
