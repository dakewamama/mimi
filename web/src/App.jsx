import React, { useCallback, useEffect, useState } from "react";

const API = import.meta.env.VITE_MIMI_API || "/api";
const THRESHOLD = 0.03;
const POLL_MS = 2000;
// What to print in the error message. When API is a proxy path like "/api",
// telling someone to curl "/api/health" is not a runnable command.
const AGENT_HINT = API.startsWith("/") ? "http://localhost:8080" : API;

const pct = (v) => `${(v * 100).toFixed(1)}%`;
const signed = (v) => `${v >= 0 ? "+" : ""}${(v * 100).toFixed(1)}`;
const clamp = (v) => Math.max(0, Math.min(1, v));
const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour12: false });

const money = (v) => {
  if (v == null) return null;
  if (v >= 1e9) return `$${(v / 1e9).toFixed(1)}B`;
  if (v >= 1e6) return `$${(v / 1e6).toFixed(1)}M`;
  if (v >= 1e3) return `$${Math.round(v / 1e3)}K`;
  return `$${Math.round(v)}`;
};

function Crest({ src, abbr }) {
  const [failed, setFailed] = useState(false);
  if (src && !failed) {
    return <img className="crest" src={src} alt="" onError={() => setFailed(true)} />;
  }
  return <div className="crest">{(abbr || "—").slice(0, 4).toUpperCase()}</div>;
}

// The signature element. One probability rail from 0 to 100%: a hollow ring for
// the on-chain price, a solid bubble for the sharp fair price, and a filled
// span between them. The gap IS the product, so the gap is what gets drawn.
function GapRail({ fair, venue }) {
  const a = clamp(Math.min(fair, venue));
  const b = clamp(Math.max(fair, venue));
  const short = fair < venue;
  return (
    <div className={`rail ${short ? "sell" : ""}`}>
      <div className="track" />
      <div className="span" style={{ left: `${a * 100}%`, width: `${(b - a) * 100}%` }} />
      <div className="knob venue" style={{ left: `${clamp(venue) * 100}%` }} title={`Jupiter ${pct(venue)}`} />
      <div className="knob fair" style={{ left: `${clamp(fair) * 100}%` }} title={`TxLINE ${pct(fair)}`} />
      <span className="scale l">0%</span>
      <span className="scale r">100%</span>
    </div>
  );
}

function Prices({ fair, venue, gap, label, side }) {
  return (
    <div className="prices">
      <div className="px">
        <div className="k">Sharp</div>
        <div className="v">{pct(fair)}</div>
      </div>
      <div className="px">
        <div className="k">On-chain</div>
        <div className="v">{pct(venue)}</div>
      </div>
      <div className={`px gap ${gap < 0 ? "neg" : "pos"}`}>
        <div className="k">{label}</div>
        <div className="v">{signed(gap)}</div>
      </div>
      {side && <span className={`side ${side}`}>{side.toUpperCase()}</span>}
    </div>
  );
}

function MatchCard({ m }) {
  return (
    <div className={`card ${m.is_signal ? "fired" : ""} ${m.gap < 0 ? "sell" : ""}`}>
      <Crest src={m.image_url} abbr={m.abbreviation} />
      <div>
        <div className="team">
          {m.team_name} <span className="vs">vs {m.opponent || "—"}</span>
        </div>
        <div className="meta">
          {m.in_running && <span className="pill live">in play</span>}
          {money(m.volume) && <span>{money(m.volume)} traded</span>}
          <span>{clock(m.ts_millis)}</span>
        </div>
        <GapRail fair={m.txline} venue={m.jupiter} />
      </div>
      <Prices fair={m.txline} venue={m.jupiter} gap={m.gap} label="Gap" />
    </div>
  );
}

function SignalCard({ s }) {
  return (
    <div className={`card fired ${s.side === "Sell" ? "sell" : ""}`}>
      <Crest src={s.image_url} abbr={s.abbreviation} />
      <div>
        <div className="team">
          {s.team_name} <span className="vs">vs {s.opponent || "—"}</span>
        </div>
        <div className="meta">
          {s.event_title && <span>{s.event_title}</span>}
          {s.in_running && <span className="pill live">in play</span>}
          <span>{clock(s.ts_millis)}</span>
        </div>
        <GapRail fair={s.fair} venue={s.venue} />
      </div>
      <Prices fair={s.fair} venue={s.venue} gap={s.edge} label="Edge" side={s.side} />
    </div>
  );
}

export default function App() {
  const [matches, setMatches] = useState([]);
  const [signals, setSignals] = useState([]);
  const [status, setStatus] = useState(null);
  const [err, setErr] = useState(null);

  const poll = useCallback(async () => {
    try {
      const get = async (path) => {
        const r = await fetch(`${API}${path}`);
        if (!r.ok) throw new Error(`${path} returned ${r.status}`);
        return r.json();
      };
      const [m, s, st] = await Promise.all([get("/matches"), get("/signals"), get("/status")]);
      setMatches(m);
      setSignals(s);
      setStatus(st);
      setErr(null);
    } catch (e) {
      setStatus(null);
      setErr(e.message);
    }
  }, []);

  useEffect(() => {
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, [poll]);

  const online = status != null;
  const live = matches.filter((m) => m.in_running).length;
  const widest = matches.length
    ? matches.reduce((a, b) => (Math.abs(b.gap) > Math.abs(a.gap) ? b : a)).gap
    : null;

  return (
    <div className="wrap">
      <header className="top">
        <div>
          <img className="mark" src="/mimi-logo.png" alt="mimi" />
          <div className="tag">first to the loose price</div>
        </div>
        <div className="status">
          <span className={`chip ${online ? "on" : "off"}`}>
            <i className="dot" />
            {online ? (status.stream_ok ? "reading the line" : "agent up, stream idle") : "agent unreachable"}
          </span>
          <span className="chip">
            fires at <b>{pct(THRESHOLD)}</b>
          </span>
        </div>
      </header>

      {err && (
        <div className="err">
          <b>Can't reach the agent.</b> {err}
          <br />
          Start it with <code>cargo run</code> from the repo root, then check{" "}
          <code>curl {AGENT_HINT}/health</code> returns <code>ok</code>.
          {API.startsWith("/") && (
            <>
              {" "}
              This page proxies <code>{API}</code> to <code>{AGENT_HINT}</code> through Vite, so the
              dev server must be running too.
            </>
          )}
        </div>
      )}

      <div className="stats">
        <div className="stat">
          <div className="k">Markets tracked</div>
          <div className="v">{matches.length}</div>
        </div>
        <div className="stat">
          <div className="k">In play</div>
          <div className="v">{live}</div>
        </div>
        <div className="stat hot">
          <div className="k">Signals caught</div>
          <div className="v">{signals.length}</div>
        </div>
        <div className="stat">
          <div className="k">Widest gap</div>
          <div className="v">{widest == null ? "—" : signed(widest)}</div>
        </div>
      </div>

      <div className="section">
        <h2>Signals</h2>
        {signals.length > 0 && <span className="count">{signals.length}</span>}
        <span className="note">the sharp line moved and the chain hasn't caught up</span>
      </div>
      {signals.length ? (
        signals.slice(0, 12).map((s, i) => <SignalCard key={`${s.ts_millis}-${s.market_id}-${i}`} s={s} />)
      ) : (
        <div className="empty">
          <b>Nothing loose yet.</b>
          Both venues agree within {pct(THRESHOLD)}. Mimi fires the moment they don't.
        </div>
      )}

      <div className="section">
        <h2>Watching</h2>
        <span className="note">every fixture matched across both venues, widest gap first</span>
      </div>
      {matches.length ? (
        matches.map((m) => <MatchCard key={m.key} m={m} />)
      ) : (
        <div className="empty">
          <b>No fixture matched on both sides yet.</b>
          Mimi needs the same game priced by TxLINE and listed on Jupiter Predict. It rebuilds the
          catalog every 60 seconds.
        </div>
      )}

      <footer>
        <span>TxLINE sharp line against Jupiter Predict, de-vigged to fair probability.</span>
        <span>
          <code>{API}</code> every {POLL_MS / 1000}s
        </span>
      </footer>
    </div>
  );
}