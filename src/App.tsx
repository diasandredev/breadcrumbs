import { useEffect, useState } from "preact/hooks";
import type { JSX } from "preact";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Tab = "timeline" | "reports" | "settings";

interface Commit {
  hash: string;
  message: string;
  author?: string;
  committedAtMs: number;
  projectName?: string;
}

interface Session {
  id: string;
  projectName: string;
  projectPath: string;
  title: string;
  agent?: string;
  modelProvider?: string;
  modelId?: string;
  tokensInput: number;
  tokensOutput: number;
  tokensReasoning: number;
  cost: number;
  additions: number;
  deletions: number;
  filesChanged: number;
  messageCount: number;
  startedAtMs: number;
  endedAtMs: number;
  commits: Commit[];
}

interface TimelineData {
  sessions: Session[];
  standaloneCommits: Commit[];
}

const TABS: { id: Tab; label: string }[] = [
  { id: "timeline", label: "Timeline" },
  { id: "reports", label: "Reports" },
  { id: "settings", label: "Settings" },
];

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtDur(ms: number): string {
  const m = Math.round(ms / 60000);
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}`;
}

function fmtTok(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function dayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function dayLabel(key: string): string {
  const today = dayKey(Date.now());
  const yesterday = dayKey(Date.now() - 86_400_000);
  if (key === today) return "Today";
  if (key === yesterday) return "Yesterday";
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, m - 1, d).toLocaleDateString([], {
    weekday: "long",
    month: "short",
    day: "numeric",
  });
}

function SessionCard({ s }: { s: Session }) {
  const diff = s.additions + s.deletions > 0;
  return (
    <div class="row">
      <div class="card-title" title={s.title}>
        {s.title}
      </div>
      <div class="card-meta">
        <span class="badge">{s.projectName}</span>
        {s.modelId && <span class="dim">{s.modelId}</span>}
        {s.agent && <span class="dim">{s.agent}</span>}
      </div>
      <div class="card-stats">
        <span class="dim">
          {fmtTime(s.startedAtMs)}–{fmtTime(s.endedAtMs)} ·{" "}
          {fmtDur(s.endedAtMs - s.startedAtMs)}
        </span>
        <span class="dim">
          {s.messageCount} msgs · {fmtTok(s.tokensInput)}→
          {fmtTok(s.tokensOutput)} tok
          {diff && (
            <>
              {" · "}
              <span class="add">+{s.additions}</span>
              {"/"}
              <span class="del">−{s.deletions}</span>
            </>
          )}
        </span>
      </div>
      {s.commits.length > 0 && (
        <ul class="commit-list">
          {s.commits.map((c) => (
            <li key={c.hash} class="dim">
              <code>{c.hash.slice(0, 7)}</code> {c.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function CommitRow({ c }: { c: Commit }) {
  return (
    <div class="row commit-card">
      <span class="dim">
        <code>{c.hash.slice(0, 7)}</code>
      </span>
      <span class="commit-msg" title={c.message}>
        {c.message}
      </span>
      <span class="badge">{c.projectName}</span>
    </div>
  );
}

interface AiSettings {
  enabled: boolean;
  url: string;
  model: string;
}

type RangeId = "today" | "week" | "last7" | "month";

const RANGES: { id: RangeId; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "week", label: "This week" },
  { id: "last7", label: "Last 7 days" },
  { id: "month", label: "This month" },
];

function rangeBounds(id: RangeId): { from: number; to: number; title: string } {
  const now = new Date();
  const to = now.getTime();
  const startOfToday = new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate()
  ).getTime();
  const fmt = (d: Date) =>
    d.toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" });

  switch (id) {
    case "today":
      return {
        from: startOfToday,
        to,
        title: `Daily Report — ${fmt(new Date())}`,
      };
    case "week": {
      const dow = (now.getDay() + 6) % 7; // Monday = 0
      const monday = startOfToday - dow * 86_400_000;
      return {
        from: monday,
        to,
        title: `Weekly Recap — ${fmt(new Date(monday))} to ${fmt(now)}`,
      };
    }
    case "last7":
      return {
        from: startOfToday - 6 * 86_400_000,
        to,
        title: `Recap — ${fmt(new Date(startOfToday - 6 * 86_400_000))} to ${fmt(now)}`,
      };
    case "month":
      return {
        from: new Date(now.getFullYear(), now.getMonth(), 1).getTime(),
        to,
        title: `Monthly Recap — ${now.toLocaleDateString([], {
          month: "long",
          year: "numeric",
        })}`,
      };
  }
}

export function App(): JSX.Element {
  const [tab, setTab] = useState<Tab>("timeline");
  const [data, setData] = useState<TimelineData | null>(null);
  const [syncing, setSyncing] = useState(false);

  async function load() {
    try {
      const d = await invoke<TimelineData>("get_timeline", { days: 30 });
      setData(d);
    } catch (e) {
      console.error(e);
      setData({ sessions: [], standaloneCommits: [] });
    }
  }

  async function sync() {
    if (syncing) return;
    setSyncing(true);
    try {
      await invoke("sync_now");
      await load();
    } catch (e) {
      console.error(e);
    } finally {
      setSyncing(false);
    }
  }

  useEffect(() => {
    load();
    let unlisten: (() => void) | undefined;
    listen("sync-done", () => load()).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  const groups = new Map<string, { sessions: Session[]; commits: Commit[] }>();
  for (const s of data?.sessions ?? []) {
    const k = dayKey(s.endedAtMs);
    if (!groups.has(k)) groups.set(k, { sessions: [], commits: [] });
    groups.get(k)!.sessions.push(s);
  }
  for (const c of data?.standaloneCommits ?? []) {
    const k = dayKey(c.committedAtMs);
    if (!groups.has(k)) groups.set(k, { sessions: [], commits: [] });
    groups.get(k)!.commits.push(c);
  }

  const isEmpty =
    data !== null &&
    data.sessions.length === 0 &&
    data.standaloneCommits.length === 0;

  return (
    <div class="panel">
      <header class="panel-header">
        <div class="brand-row">
          <div class="brand">
            <span class="brand-dot" />
            Breadcrumbs
          </div>
          {tab === "timeline" && (
            <button class="sync-btn" onClick={sync} disabled={syncing}>
              {syncing ? "Syncing…" : "Sync now"}
            </button>
          )}
        </div>
        <nav class="segmented">
          {TABS.map((t) => (
            <button
              key={t.id}
              class={`seg-btn${tab === t.id ? " active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <main class="panel-body">
        {tab === "timeline" &&
          (data === null ? (
            <p class="hint-center">Loading…</p>
          ) : isEmpty ? (
            <p class="hint-center">No sessions or commits yet — hit Sync.</p>
          ) : (
            [...groups.entries()].map(([key, g]) => (
              <section key={key} class="day-group">
                <h3 class="day-label">{dayLabel(key)}</h3>
                <div class="list">
                  {g.sessions.map((s) => (
                    <SessionCard key={s.id} s={s} />
                  ))}
                  {g.commits.map((c) => (
                    <CommitRow key={c.hash} c={c} />
                  ))}
                </div>
              </section>
            ))
          ))}

        {tab === "reports" && <ReportsTab />}

        {tab === "settings" && <SettingsTab />}
      </main>
    </div>
  );
}

function ReportsTab(): JSX.Element {
  const [range, setRange] = useState<RangeId>("today");
  const [report, setReport] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [enhancing, setEnhancing] = useState(false);

  async function load(r: RangeId) {
    const b = rangeBounds(r);
    try {
      const md = await invoke<string>("generate_report", {
        fromMs: b.from,
        toMs: b.to,
        title: b.title,
      });
      setReport(md);
    } catch (e) {
      console.error(e);
      setReport("Failed to generate report.");
    }
  }

  useEffect(() => {
    load(range);
  }, [range]);

  async function copy() {
    await navigator.clipboard.writeText(report);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  async function enhance() {
    if (enhancing) return;
    setEnhancing(true);
    try {
      const b = rangeBounds(range);
      const md = await invoke<string>("enhance_report", {
        fromMs: b.from,
        toMs: b.to,
        title: b.title,
      });
      if (md) setReport(md);
    } catch (e) {
      console.error(e);
      alert(String(e));
    } finally {
      setEnhancing(false);
    }
  }

  return (
    <div class="reports">
      <div class="range-row">
        {RANGES.map((r) => (
          <button
            key={r.id}
            class={`chip${range === r.id ? " active" : ""}`}
            onClick={() => setRange(r.id)}
          >
            {r.label}
          </button>
        ))}
      </div>
      <pre class="report-preview">{report}</pre>
      <div class="report-actions">
        <button class="sync-btn" onClick={enhance} disabled={enhancing}>
          {enhancing ? "Thinking…" : "✦ Enhance with AI"}
        </button>
        <button class="sync-btn" onClick={copy} disabled={!report}>
          {copied ? "Copied ✓" : "Copy markdown"}
        </button>
      </div>
    </div>
  );
}

function SettingsTab(): JSX.Element {
  const [cfg, setCfg] = useState<AiSettings | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [saved, setSaved] = useState(false);
  const [connError, setConnError] = useState("");

  useEffect(() => {
    invoke<AiSettings>("get_settings")
      .then(setCfg)
      .catch((e) => console.error(e));
  }, []);

  useEffect(() => {
    if (!cfg?.url) return;
    setConnError("");
    invoke<string[]>("list_ollama_models", { url: cfg.url })
      .then((m) => {
        setModels(m);
        if (m.length === 0) setConnError("No models installed in Ollama.");
      })
      .catch((e) => {
        setModels([]);
        setConnError(String(e));
      });
  }, [cfg?.url]);

  if (!cfg) return <p class="hint-center">Loading…</p>;

  function save() {
    if (!cfg) return;
    invoke("set_ai_settings", { settings: cfg })
      .then(() => {
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      })
      .catch((e) => console.error(e));
  }

  return (
    <div class="settings">
      <section class="setting-block">
        <div class="setting-row toggle-row">
          <span>AI report summaries</span>
          <label class="switch">
            <input
              type="checkbox"
              checked={cfg.enabled}
              onChange={(e) =>
                setCfg({
                  ...cfg,
                  enabled: (e.target as HTMLInputElement).checked,
                })
              }
            />
            <span class="knob" />
          </label>
        </div>
        <p class="setting-hint">
          When off, reports are fully deterministic. Everything stays local.
        </p>
      </section>

      <section class="setting-block">
        <span class="setting-label">Ollama URL</span>
        <input
          type="text"
          class="text-input"
          value={cfg.url}
          placeholder="http://localhost:11434"
          onChange={(e) =>
            setCfg({ ...cfg, url: (e.target as HTMLInputElement).value })
          }
        />
        <p class={`setting-hint${connError ? " err" : ""}`}>
          {connError || `${models.length} model(s) available`}
        </p>
      </section>

      <section class="setting-block">
        <span class="setting-label">Model</span>
        <select
          class="text-input"
          disabled={models.length === 0}
          value={cfg.model}
          onChange={(e) =>
            setCfg({ ...cfg, model: (e.target as HTMLSelectElement).value })
          }
        >
          {models.length === 0 && <option>No models</option>}
          {models.map((m) => (
            <option key={m} value={m} selected={m === cfg.model}>
              {m}
            </option>
          ))}
        </select>
      </section>

      <div class="report-actions">
        <button class="sync-btn primary" onClick={save}>
          {saved ? "Saved ✓" : "Save settings"}
        </button>
      </div>
    </div>
  );
}

export default App;
