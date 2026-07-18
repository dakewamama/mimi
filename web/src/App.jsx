import React, { useEffect, useState, useCallback } from "react";

const API = import.meta.env.VITE_MIMI_API || "/api";
const THRESHOLD = 0.03;
const POLL_MS = 2000;

const pct = (v) => `${(v * 100).toFixed(2)}%`;
const signed = (v) => `${v >= 0 ? "+" : ""}${(v * 100).toFixed(2)}%`;
const money = (v) =>
  v == null ? null : v >= 1e6 ? `$${(v / 1e6).toFixed(1)}M` : `$${(v / 1e3).toFixed(0)}K`;
const clock = (ms) => new Date(ms).toLocaleTimeString();

function Flag({ src, abbr, color }) {
  if (src) return <img className="flag" src={src} alt={abbr || ""} />;
  return (
    <div className="flag ph" style={color ? { background: color + "22" } : undefined}>
      {(abbr || "??").toUpperCase()}
    </div>
  );
}

function Row({ m }) {
  const strength = Math.min(Math.abs(m.gap) / THRESHOLD, 1) * 100;
  const neg = m.gap < 0;
  return (
    <div className={`card ${m.is_signal ? "signal" : ""} ${m.is_signal && neg ? "sell" : ""}`}>
      <Flag src={m.image_url} abbr={m.abbreviation} color={m.color} />
      <div>
        <div className="team">
          {m.team_name} <span style={{ color: "var(--muted)", fontWeight: 400 }}>vs {m.opponent}</span>
        </div>
        <div className="meta">
          {m.competition && <span className="pill">{m.competition}</span>}
          {m.sport && <span className="pill">{m.sport}</span>}
          {m.in_running && <span className="pill live">live</span>}
          {money(m.volume) && <span>vol {money(m.volume)}</span>}
          <span>{clock(m.ts_millis)}</span>
        </div>
        <div className={`bar ${neg ? "neg" : ""}`}>
          <i style={{ width: `${strength}%` }} />
        </div>
      </div>
      <div className="prices">
        <div className="px">
          <div className="k">TxLINE</div>
          <div className="v">{pct(m.txline)}</div>
        </div>
        <div className="px">
          <div className="k">Jupiter</div>
          <div className="v">{pct(m.jupiter)}</div>
        </div>
        <div className={`px gap ${neg ? "neg" : "pos"}`}>
          <div className="k">Gap</div>
          <div className="v">{signed(m.gap)}</div>
        </div>
      </div>
    </div>
  );
}

function SignalRow({ s }) {
  return (
    <div className={`card signal ${s.side === "Sell" ? "sell" : ""}`}>
      <Flag src={s.image_url} abbr={s.abbreviation} color={s.color} />
      <div>
        <div className="team">{s.team_name}</div>
        <div className="meta">
          <span>{s.event_title}</span>
          {s.competition && <span className="pill">{s.competition}</span>}
          <span>{clock(s.ts_millis)}</span>
        </div>
      </div>
      <div className="prices">
        <div className="px">
          <div className="k">TxLINE</div>
          <div className="v">{pct(s.fair)}</div>
        </div>
        <div className="px">
          <div className="k">Jupiter</div>
          <div className="v">{pct(s.venue)}</div>
        </div>
        <div className={`px gap ${s.edge < 0 ? "neg" : "pos"}`}>
          <div className="k">Edge</div>
          <div className="v">{signed(s.edge)}</div>
        </div>
        <span className={`side ${s.side}`}>{s.side.toUpperCase()}</span>
      </div>
    </div>
  );
}

export default function App() {
  const [matches, setMatches] = useState([]);
  const [signals, setSignals] = useState([]);
  const [online, setOnline] = useState(false);
  const [err, setErr] = useState(null);

  const poll = useCallback(async () => {
    try {
      const [m, s] = await Promise.all([
        fetch(`${API}/matches`).then((r) => r.json()),
        fetch(`${API}/signals`).then((r) => r.json()),
      ]);
      setMatches(m);
      setSignals(s);
      setOnline(true);
      setErr(null);
    } catch (e) {
      setOnline(false);
      setErr(`cannot reach agent at ${API}`);
    }
  }, []);

  useEffect(() => {
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, [poll]);

  const live = matches.filter((m) => m.in_running).length;

  return (
    <div className="wrap">
      <header className="top">
        <div>
          <div className="brand">MIMI</div>
          <div className="tag">first to the loose price</div>
        </div>
        <div className="status">
          <span>
            <i className={`dot ${online ? "on" : "off"}`} />
            {online ? "agent connected" : "agent offline"}
          </span>
          <span>threshold {pct(THRESHOLD)}</span>
        </div>
      </header>

      {err && <div className="err">{err}</div>}

      <div className="stats">
        <div className="stat">
          <div className="k">Tracked markets</div>
          <div className="v">{matches.length}</div>
        </div>
        <div className="stat">
          <div className="k">In running</div>
          <div className="v">{live}</div>
        </div>
        <div className="stat">
          <div className="k">Signals caught</div>
          <div className="v">{signals.length}</div>
        </div>
        <div className="stat">
          <div className="k">Widest gap</div>
          <div className="v">
            {matches.length
              ? signed(matches.reduce((a, b) => (Math.abs(b.gap) > Math.abs(a.gap) ? b : a)).gap)
              : "—"}
          </div>
        </div>
      </div>

      <h2 className="section">Signals</h2>
      {signals.length ? (
        signals.slice(0, 12).map((s, i) => <SignalRow key={`${s.ts_millis}-${i}`} s={s} />)
      ) : (
        <div className="empty">
          No divergence past threshold yet. Markets in line across both venues.
        </div>
      )}

      <h2 className="section">Tracked markets</h2>
      {matches.length ? (
        matches.map((m) => <Row key={m.key} m={m} />)
      ) : (
        <div className="empty">Waiting for the agent to match a fixture across both venues.</div>
      )}

      <footer>
        TxLINE sharp line vs Jupiter Predict on-chain price. Polling {API} every {POLL_MS / 1000}s.
      </footer>
    </div>
  );
}
