import { useEffect, useState } from "preact/hooks";
import type { JSX } from "preact";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Tab = "diary" | "projects" | "reports" | "settings";

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
  isMultiday: boolean;
  daySpan: number;
  startedDate: string;
  endedDate: string;
  note?: string;
  commits: Commit[];
}

interface TimelineData {
  sessions: Session[];
  standaloneCommits: Commit[];
}

interface AiSettings {
  enabled: boolean;
  provider: string; // "gateway" | "ollama"
  url: string;
  auth_mode: string; // "token_file" | "api_key"
  token_file?: string;
  api_key?: string;
  model: string;
  reasoning?: string;
}

type RangeId = "today" | "week" | "last7" | "month";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function parseYmd(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function addDays(s: string, n: number): string {
  const d = parseYmd(s);
  d.setDate(d.getDate() + n);
  return ymd(d);
}

function fmtDay(s: string): string {
  return parseYmd(s).toLocaleDateString([], {
    weekday: "long",
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

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

function level(n: number): number {
  if (n <= 0) return 0;
  if (n <= 1) return 1;
  if (n <= 3) return 2;
  if (n <= 6) return 3;
  return 4;
}

function currentStreak(counts: Record<string, number>): number {
  let n = 0;
  let cur = ymd(new Date());
  if (!counts[cur]) cur = addDays(cur, -1);
  while (counts[cur]) {
    n++;
    cur = addDays(cur, -1);
  }
  return n;
}

export function App(): JSX.Element {
  const [tab, setTab] = useState<Tab>("diary");
  const [data, setData] = useState<TimelineData | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [selectedDay, setSelectedDay] = useState<string>(ymd(new Date()));
  const [projectFilter, setProjectFilter] = useState<string>("");
  const [showCommits, setShowCommits] = useState<boolean>(true);

  async function load() {
    try {
      const d = await invoke<TimelineData>("get_timeline", { days: 365 });
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

  // Filtered entries
  const visibleSessions = (data?.sessions ?? []).filter(
    (s) => !projectFilter || s.projectName === projectFilter
  );

  // Counts by day for heatmap
  const countsByDay: Record<string, number> = {};
  for (const s of visibleSessions) {
    countsByDay[s.endedDate] = (countsByDay[s.endedDate] || 0) + 1;
    if (s.isMultiday && s.startedDate !== s.endedDate) {
      countsByDay[s.startedDate] = (countsByDay[s.startedDate] || 0) + 1;
    }
  }
  for (const c of data?.standaloneCommits ?? []) {
    const cd = ymd(new Date(c.committedAtMs));
    countsByDay[cd] = (countsByDay[cd] || 0) + 1;
  }

  // Projects list
  const projects = Array.from(
    new Set((data?.sessions ?? []).map((s) => s.projectName))
  ).sort();

  // Metrics totals
  const totalTokens = visibleSessions.reduce(
    (acc, s) => acc + s.tokensInput + s.tokensOutput,
    0
  );
  const streak = currentStreak(countsByDay);
  const activeDaysCount = Object.keys(countsByDay).length;

  return (
    <div class="app-layout">
      {/* Sidebar */}
      <aside class="sidebar">
        <div class="brand">
          <span class="brand-dot" />
          <div class="brand-text">
            <span class="brand-name">Breadcrumbs</span>
            <span class="brand-sub">Local Dev Diary</span>
          </div>
        </div>

        <nav class="nav-tabs">
          <button
            class={`nav-btn ${tab === "diary" ? "active" : ""}`}
            onClick={() => setTab("diary")}
          >
            <span class="nav-icon">📖</span> Dev Diary
          </button>
          <button
            class={`nav-btn ${tab === "projects" ? "active" : ""}`}
            onClick={() => setTab("projects")}
          >
            <span class="nav-icon">📁</span> Projects
          </button>
          <button
            class={`nav-btn ${tab === "reports" ? "active" : ""}`}
            onClick={() => setTab("reports")}
          >
            <span class="nav-icon">📊</span> Reports
          </button>
          <button
            class={`nav-btn ${tab === "settings" ? "active" : ""}`}
            onClick={() => setTab("settings")}
          >
            <span class="nav-icon">⚙️</span> Settings
          </button>
        </nav>

        <div class="sidebar-spacer" />

        <div class="sidebar-foot">
          <button class="sync-action-btn" onClick={sync} disabled={syncing}>
            <span class={`sync-icon ${syncing ? "spin" : ""}`}>↻</span>
            {syncing ? "Syncing..." : "Sync Agents"}
          </button>
          <div class="app-tag">OpenCode + Claude Code</div>
        </div>
      </aside>

      {/* Main Content Area */}
      <main class="main-content">
        {tab === "diary" && (
          <div class="diary-view">
            {/* KPI Totals */}
            <header class="totals-strip">
              <div class="kpi-card">
                <span class="kpi-val">{visibleSessions.length}</span>
                <span class="kpi-lbl">Sessions</span>
              </div>
              <div class="kpi-card">
                <span class="kpi-val">{activeDaysCount}</span>
                <span class="kpi-lbl">Active Days</span>
              </div>
              <div class="kpi-card">
                <span class="kpi-val">{streak}</span>
                <span class="kpi-lbl">Day Streak</span>
              </div>
              <div class="kpi-card">
                <span class="kpi-val">{fmtTok(totalTokens)}</span>
                <span class="kpi-lbl">Tokens</span>
              </div>
            </header>

            {/* 52-Week Contribution Heatmap */}
            <HeatmapSection
              counts={countsByDay}
              selectedDay={selectedDay}
              onSelectDay={setSelectedDay}
            />

            {/* Day Toolbar */}
            <div class="day-toolbar">
              <div class="day-navigator">
                <button
                  class="btn-icon"
                  title="Previous Day"
                  onClick={() => setSelectedDay(addDays(selectedDay, -1))}
                >
                  ‹
                </button>
                <span class="day-title">{fmtDay(selectedDay)}</span>
                <button
                  class="btn-icon"
                  title="Next Day"
                  onClick={() => setSelectedDay(addDays(selectedDay, 1))}
                >
                  ›
                </button>
                <button
                  class="btn-text-ghost"
                  onClick={() => setSelectedDay(ymd(new Date()))}
                >
                  Today
                </button>
              </div>

              <div class="toolbar-spacer" />

              <div class="toolbar-filters">
                <select
                  class="select-filter"
                  value={projectFilter}
                  onChange={(e) =>
                    setProjectFilter((e.target as HTMLSelectElement).value)
                  }
                >
                  <option value="">All Projects</option>
                  {projects.map((p) => (
                    <option key={p} value={p}>
                      {p}
                    </option>
                  ))}
                </select>

                <button
                  class={`btn-toggle ${showCommits ? "active" : ""}`}
                  onClick={() => setShowCommits(!showCommits)}
                >
                  Commits: {showCommits ? "on" : "off"}
                </button>
              </div>
            </div>

            {/* Daily Timeline */}
            <DayTimeline
              day={selectedDay}
              sessions={visibleSessions}
              standaloneCommits={data?.standaloneCommits ?? []}
              showCommits={showCommits}
              onNoteSaved={load}
            />
          </div>
        )}

        {tab === "projects" && (
          <ProjectsView
            sessions={data?.sessions ?? []}
            onSelectProject={(p) => {
              setProjectFilter(p);
              setTab("diary");
            }}
          />
        )}

        {tab === "reports" && <ReportsStudio sessions={data?.sessions ?? []} />}

        {tab === "settings" && <SettingsView />}
      </main>
    </div>
  );
}

// ---------------------------------------------------------
// Component: HeatmapSection (52 Weeks Contribution Grid)
// ---------------------------------------------------------
function HeatmapSection({
  counts,
  selectedDay,
  onSelectDay,
}: {
  counts: Record<string, number>;
  selectedDay: string;
  onSelectDay: (d: string) => void;
}) {
  const today = new Date();
  const todayYmd = ymd(today);

  // Compute 52 weeks back starting on Sunday
  const start = new Date(today);
  start.setDate(start.getDate() - 52 * 7);
  start.setDate(start.getDate() - start.getDay());

  const weeks: { ds: string; n: number; future: boolean }[][] = [];
  const cur = new Date(start);

  for (let w = 0; w < 53; w++) {
    const days = [];
    for (let dow = 0; dow < 7; dow++) {
      const ds = ymd(cur);
      days.push({ ds, n: counts[ds] || 0, future: ds > todayYmd });
      cur.setDate(cur.getDate() + 1);
    }
    weeks.push(days);
  }

  let prevM = -1;
  const monthLabels = weeks.map((days) => {
    const m = parseYmd(days[0].ds).getMonth();
    if (m !== prevM) {
      prevM = m;
      return MONTHS[m];
    }
    return "";
  });

  return (
    <section class="heatmap-panel">
      <div class="heatmap-header">
        <span class="heatmap-title">Annual Activity</span>
        <span class="heatmap-sub">52-week timeline</span>
      </div>
      <div class="cal-months-row">
        {monthLabels.map((lbl, i) => (
          <span key={i} class="cal-month-lbl">
            {lbl}
          </span>
        ))}
      </div>
      <div class="cal-grid-row">
        <div class="cal-dow-col">
          <span></span>
          <span>Mon</span>
          <span></span>
          <span>Wed</span>
          <span></span>
          <span>Fri</span>
          <span></span>
        </div>
        <div class="cal-weeks-grid">
          {weeks.map((days, wIdx) => (
            <div key={wIdx} class="cal-week-col">
              {days.map((d) => {
                const lv = level(d.n);
                const isSel = d.ds === selectedDay;
                const isToday = d.ds === todayYmd;
                const tip = `${fmtDay(d.ds)} — ${d.n} session(s)`;
                return (
                  <button
                    key={d.ds}
                    disabled={d.future}
                    class={`cal-cell l${lv} ${isSel ? "selected" : ""} ${
                      isToday ? "today" : ""
                    } ${d.future ? "future" : ""}`}
                    title={tip}
                    onClick={() => onSelectDay(d.ds)}
                  />
                );
              })}
            </div>
          ))}
        </div>
      </div>
      <div class="heatmap-legend">
        <span>Less</span>
        <i class="cal-cell l0" />
        <i class="cal-cell l1" />
        <i class="cal-cell l2" />
        <i class="cal-cell l3" />
        <i class="cal-cell l4" />
        <span>More</span>
      </div>
    </section>
  );
}

// ---------------------------------------------------------
// Component: DayTimeline (Renders Sessions & Standalone Commits)
// ---------------------------------------------------------
function DayTimeline({
  day,
  sessions,
  standaloneCommits,
  showCommits,
  onNoteSaved,
}: {
  day: string;
  sessions: Session[];
  standaloneCommits: Commit[];
  showCommits: boolean;
  onNoteSaved: () => void;
}) {
  // Filter sessions that were either started or ended or active on this day
  const daySessions = sessions.filter(
    (s) => s.endedDate === day || s.startedDate === day
  );

  const dayCommits = standaloneCommits.filter(
    (c) => ymd(new Date(c.committedAtMs)) === day
  );

  if (daySessions.length === 0 && (!showCommits || dayCommits.length === 0)) {
    return (
      <div class="empty-state">
        <div class="empty-icon">☕</div>
        <h3>No activity recorded</h3>
        <p>No OpenCode or Claude Code sessions logged on {fmtDay(day)}.</p>
      </div>
    );
  }

  return (
    <div class="timeline-container">
      {daySessions.map((s) => (
        <EnhancedSessionCard
          key={s.id}
          session={s}
          currentDay={day}
          onNoteSaved={onNoteSaved}
        />
      ))}

      {showCommits &&
        dayCommits.map((c) => (
          <div key={c.hash} class="commit-timeline-row">
            <span class="commit-dot" />
            <div class="commit-card-body">
              <span class="commit-time">{fmtTime(c.committedAtMs)}</span>
              <code class="commit-hash">{c.hash.slice(0, 7)}</code>
              <span class="commit-message">{c.message}</span>
              {c.projectName && (
                <span class="tag-badge project">{c.projectName}</span>
              )}
            </div>
          </div>
        ))}
    </div>
  );
}

// ---------------------------------------------------------
// Component: EnhancedSessionCard (Clear Multi-Day Handling)
// ---------------------------------------------------------
function EnhancedSessionCard({
  session: s,
  currentDay,
  onNoteSaved,
}: {
  session: Session;
  currentDay: string;
  onNoteSaved: () => void;
}) {
  const [editingNote, setEditingNote] = useState(false);
  const [noteText, setNoteText] = useState(s.note || "");
  const [savingNote, setSavingNote] = useState(false);

  const isContinued = s.isMultiday && s.startedDate !== currentDay;
  const isStarting = s.isMultiday && s.startedDate === currentDay;

  const agentLabel = s.agent === "claude-code" ? "Claude Code" : "OpenCode";
  const agentClass = s.agent === "claude-code" ? "claude" : "opencode";

  async function saveNote() {
    setSavingNote(true);
    try {
      await invoke("set_note", {
        sessionId: s.id,
        note: noteText,
      });
      setEditingNote(false);
      onNoteSaved();
    } catch (e) {
      console.error(e);
    } finally {
      setSavingNote(false);
    }
  }

  return (
    <article class="session-card">
      <div class="session-card-header">
        <div class="header-tags">
          <span class="tag-badge project">{s.projectName}</span>
          <span class={`tag-badge agent ${agentClass}`}>{agentLabel}</span>
          {s.modelId && <span class="tag-badge model">{s.modelId}</span>}
          {s.isMultiday && (
            <span class="tag-badge multiday" title={`Active over ${s.daySpan} calendar days`}>
              ✦ Multi-day ({s.daySpan}d)
            </span>
          )}
        </div>

        <div class="header-actions">
          <button
            class="action-subtle-btn"
            onClick={() => setEditingNote(!editingNote)}
          >
            {s.note ? "✎ Edit Note" : "+ Add Note"}
          </button>
        </div>
      </div>

      <h4 class="session-title">{s.title}</h4>

      {/* Multi-Day Detailed Timeline Banner */}
      {s.isMultiday ? (
        <div class="multiday-notice">
          <span class="md-icon">⏱</span>
          <span class="md-text">
            {isContinued ? (
              <>
                <strong>Continued session:</strong> started on {s.startedDate} (
                {fmtTime(s.startedAtMs)}) and updated on {s.endedDate} (
                {fmtTime(s.endedAtMs)}).
              </>
            ) : isStarting ? (
              <>
                <strong>Started on this day:</strong> active until {s.endedDate} (
                {s.daySpan} days span).
              </>
            ) : (
              <>
                <strong>Multi-day cycle:</strong> {s.startedDate} → {s.endedDate}{" "}
                ({s.daySpan} days span).
              </>
            )}
          </span>
        </div>
      ) : (
        <div class="session-time-row">
          <span class="time-range">
            {fmtTime(s.startedAtMs)} – {fmtTime(s.endedAtMs)}
          </span>
          <span class="duration">({fmtDur(s.endedAtMs - s.startedAtMs)})</span>
        </div>
      )}

      {/* Metrics Row */}
      <div class="session-metrics">
        <span class="metric-chip">
          <strong>{s.messageCount}</strong> msgs
        </span>
        <span class="metric-chip">
          <strong>{fmtTok(s.tokensInput)}</strong> in →{" "}
          <strong>{fmtTok(s.tokensOutput)}</strong> out
        </span>
        {s.additions + s.deletions > 0 && (
          <span class="metric-chip diff">
            <span class="diff-add">+{s.additions}</span>
            <span class="diff-sep">/</span>
            <span class="diff-del">−{s.deletions}</span> across{" "}
            <strong>{s.filesChanged}</strong> files
          </span>
        )}
      </div>

      {/* User Note Display */}
      {s.note && !editingNote && (
        <div class="session-user-note">
          <span class="note-pin">📌</span>
          <p class="note-text">{s.note}</p>
        </div>
      )}

      {/* Inline Note Editor */}
      {editingNote && (
        <div class="inline-note-editor">
          <textarea
            class="note-textarea"
            placeholder="Add context or observations about this session..."
            value={noteText}
            onInput={(e) => setNoteText((e.target as HTMLTextAreaElement).value)}
          />
          <div class="note-editor-actions">
            <button
              class="btn-primary-sm"
              onClick={saveNote}
              disabled={savingNote}
            >
              {savingNote ? "Saving..." : "Save Note"}
            </button>
            <button
              class="btn-ghost-sm"
              onClick={() => setEditingNote(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Associated Git Commits */}
      {s.commits.length > 0 && (
        <div class="session-commits-section">
          <span class="commits-heading">Git Commits:</span>
          <ul class="session-commit-list">
            {s.commits.map((c) => (
              <li key={c.hash} class="commit-item">
                <code class="commit-hash">{c.hash.slice(0, 7)}</code>
                <span class="commit-msg">{c.message}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </article>
  );
}

// ---------------------------------------------------------
// Component: ProjectsView (Directory & Activity Overview)
// ---------------------------------------------------------
function ProjectsView({
  sessions,
  onSelectProject,
}: {
  sessions: Session[];
  onSelectProject: (p: string) => void;
}) {
  const byProj: Record<
    string,
    { sessions: Session[]; totalTokens: number; lastActive: number }
  > = {};

  for (const s of sessions) {
    if (!byProj[s.projectName]) {
      byProj[s.projectName] = { sessions: [], totalTokens: 0, lastActive: 0 };
    }
    byProj[s.projectName].sessions.push(s);
    byProj[s.projectName].totalTokens += s.tokensInput + s.tokensOutput;
    byProj[s.projectName].lastActive = Math.max(
      byProj[s.projectName].lastActive,
      s.endedAtMs
    );
  }

  const projList = Object.entries(byProj).sort(
    (a, b) => b[1].lastActive - a[1].lastActive
  );

  return (
    <div class="projects-view">
      <header class="section-header">
        <h2>Monitored Projects</h2>
        <span class="section-sub">
          Auto-discovered from OpenCode and Claude Code sessions
        </span>
      </header>

      <div class="projects-grid">
        {projList.map(([name, info]) => (
          <div
            key={name}
            class="project-card"
            onClick={() => onSelectProject(name)}
          >
            <div class="project-card-top">
              <h3 class="project-name">{name}</h3>
              <span class="badge-count">{info.sessions.length} sessions</span>
            </div>
            <p class="project-path">
              {info.sessions[0]?.projectPath || "Local workspace"}
            </p>
            <div class="project-card-stats">
              <span>{fmtTok(info.totalTokens)} tokens</span>
              <span>
                Last active: {new Date(info.lastActive).toLocaleDateString()}
              </span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------------------------------------------------------
// Component: ReportsStudio (Daily Slack Card + Export Options)
// ---------------------------------------------------------
function ReportsStudio({ sessions: _sessions }: { sessions: Session[] }) {
  const [range, setRange] = useState<RangeId>("today");
  const [report, setReport] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [enhancing, setEnhancing] = useState(false);

  function boundsForRange(r: RangeId): { from: number; to: number; title: string } {
    const now = new Date();
    const to = now.getTime();
    const startOfToday = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate()
    ).getTime();
    const fmt = (d: Date) =>
      d.toLocaleDateString([], { month: "short", day: "numeric", year: "numeric" });

    switch (r) {
      case "today":
        return {
          from: startOfToday,
          to,
          title: `Daily Report — ${fmt(new Date())}`,
        };
      case "week": {
        const dow = (now.getDay() + 6) % 7;
        const monday = startOfToday - dow * 86_400_000;
        return {
          from: monday,
          to,
          title: `Weekly Standup — ${fmt(new Date(monday))} to ${fmt(now)}`,
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

  async function loadReport(r: RangeId) {
    const b = boundsForRange(r);
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
    loadReport(range);
  }, [range]);

  async function copyMarkdown() {
    await navigator.clipboard.writeText(report);
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  }

  async function enhance() {
    if (enhancing) return;
    setEnhancing(true);
    try {
      const b = boundsForRange(range);
      const enhancedMd = await invoke<string>("enhance_report", {
        fromMs: b.from,
        toMs: b.to,
        title: b.title,
      });
      if (enhancedMd) setReport(enhancedMd);
    } catch (e) {
      console.error(e);
      alert(String(e));
    } finally {
      setEnhancing(false);
    }
  }

  return (
    <div class="reports-studio">
      <header class="section-header">
        <h2>Reports & Standup Studio</h2>
        <span class="section-sub">
          Generate clean Markdown and Slack/Discord ready recaps
        </span>
      </header>

      <div class="range-selector">
        <button
          class={`range-chip ${range === "today" ? "active" : ""}`}
          onClick={() => setRange("today")}
        >
          Today
        </button>
        <button
          class={`range-chip ${range === "week" ? "active" : ""}`}
          onClick={() => setRange("week")}
        >
          This Week
        </button>
        <button
          class={`range-chip ${range === "last7" ? "active" : ""}`}
          onClick={() => setRange("last7")}
        >
          Last 7 Days
        </button>
        <button
          class={`range-chip ${range === "month" ? "active" : ""}`}
          onClick={() => setRange("month")}
        >
          This Month
        </button>
      </div>

      <div class="report-box">
        <pre class="report-code-preview">{report}</pre>
      </div>

      <div class="report-actions-strip">
        <button
          class="btn-enhance-ai"
          onClick={enhance}
          disabled={enhancing}
        >
          {enhancing ? "Distilling with AI..." : "✦ Enhance with AI Gateway"}
        </button>
        <button class="btn-copy-md" onClick={copyMarkdown}>
          {copied ? "Copied ✓" : "Copy Markdown"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------
// Component: SettingsView (AI Gateway + Multi-Model Endpoint)
// ---------------------------------------------------------
function SettingsView() {
  const [cfg, setCfg] = useState<AiSettings | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [saved, setSaved] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string>("");

  useEffect(() => {
    invoke<AiSettings>("get_settings")
      .then(setCfg)
      .catch((e) => console.error(e));
  }, []);

  function saveSettings() {
    if (!cfg) return;
    invoke("set_ai_settings", { settings: cfg })
      .then(() => {
        setSaved(true);
        setTimeout(() => setSaved(false), 1600);
      })
      .catch((e) => console.error(e));
  }

  async function testConnection() {
    if (!cfg) return;
    setTesting(true);
    setTestResult("");
    try {
      const res = await invoke<string[]>("list_ai_models", { settings: cfg });
      setModels(res);
      setTestResult(`✓ Connected successfully (${res.length} models detected)`);
    } catch (e) {
      setTestResult(`✗ Connection failed: ${e}`);
    } finally {
      setTesting(false);
    }
  }

  if (!cfg) return <div class="loading-state">Loading settings...</div>;

  return (
    <div class="settings-view">
      <header class="section-header">
        <h2>App & AI Configuration</h2>
        <span class="section-sub">
          Manage local-first agent capture and AI Gateway connectivity
        </span>
      </header>

      {/* Agent Capture Section */}
      <section class="settings-section">
        <h3>Connected Coding Agents</h3>
        <div class="agent-status-row">
          <div class="agent-info">
            <span class="agent-name">OpenCode</span>
            <span class="agent-desc">Local SQLite database watcher</span>
          </div>
          <span class="status-badge active">Auto-Detected</span>
        </div>
        <div class="agent-status-row">
          <div class="agent-info">
            <span class="agent-name">Claude Code</span>
            <span class="agent-desc">Transcript watcher (~/.claude/projects)</span>
          </div>
          <span class="status-badge active">Auto-Detected</span>
        </div>
      </section>

      {/* AI Gateway Section */}
      <section class="settings-section">
        <div class="setting-toggle-header">
          <div>
            <h3>AI Report Enhancements</h3>
            <p class="section-sub">
              Distill coding sessions and generate executive summaries
            </p>
          </div>
          <label class="switch-toggle">
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
            <span class="slider" />
          </label>
        </div>

        {cfg.enabled && (
          <div class="ai-fields">
            <div class="form-group">
              <label class="form-label">Provider Type</label>
              <select
                class="form-input"
                value={cfg.provider}
                onChange={(e) =>
                  setCfg({
                    ...cfg,
                    provider: (e.target as HTMLSelectElement).value,
                  })
                }
              >
                <option value="gateway">
                  Custom AI Gateway (OpenAI-compatible)
                </option>
                <option value="ollama">Ollama (Local / Private)</option>
              </select>
            </div>

            <div class="form-group">
              <label class="form-label">
                {cfg.provider === "gateway"
                  ? "Gateway Endpoint Base URL"
                  : "Ollama URL"}
              </label>
              <input
                type="text"
                class="form-input"
                placeholder={
                  cfg.provider === "gateway"
                    ? "https://generative-ai-platform-development.ifoodcorp.com.br/api/v2"
                    : "http://localhost:11434"
                }
                value={cfg.url}
                onChange={(e) =>
                  setCfg({
                    ...cfg,
                    url: (e.target as HTMLInputElement).value,
                  })
                }
              />
            </div>

            {cfg.provider === "gateway" && (
              <>
                <div class="form-group">
                  <label class="form-label">Authentication Method</label>
                  <select
                    class="form-input"
                    value={cfg.auth_mode || "token_file"}
                    onChange={(e) =>
                      setCfg({
                        ...cfg,
                        auth_mode: (e.target as HTMLSelectElement).value,
                      })
                    }
                  >
                    <option value="token_file">
                      Dynamic Token File (Zero restart, auto-rotated by daemon)
                    </option>
                    <option value="api_key">
                      Manual Static API Key / Bearer Token
                    </option>
                  </select>
                </div>

                {cfg.auth_mode !== "api_key" ? (
                  <div class="form-group">
                    <label class="form-label">Token File Path</label>
                    <input
                      type="text"
                      class="form-input"
                      placeholder="~/.config/tompero/requester_token"
                      value={cfg.token_file || "~/.config/tompero/requester_token"}
                      onChange={(e) =>
                        setCfg({
                          ...cfg,
                          token_file: (e.target as HTMLInputElement).value,
                        })
                      }
                    />
                    <span class="field-hint">
                      Reads the live requester token from disk on every call. No restarts needed when tokens rotate.
                    </span>
                  </div>
                ) : (
                  <div class="form-group">
                    <label class="form-label">API Key / Bearer Token</label>
                    <input
                      type="password"
                      class="form-input"
                      placeholder="Paste your gateway authentication key"
                      value={cfg.api_key || ""}
                      onChange={(e) =>
                        setCfg({
                          ...cfg,
                          api_key: (e.target as HTMLInputElement).value,
                        })
                      }
                    />
                  </div>
                )}
              </>
            )}

            <div class="form-group">
              <label class="form-label">Model Identifier</label>
              <input
                type="text"
                list="model-suggestions"
                class="form-input"
                placeholder="e.g. claude-3-7-sonnet, gpt-4o, llama3.3"
                value={cfg.model}
                onChange={(e) =>
                  setCfg({
                    ...cfg,
                    model: (e.target as HTMLInputElement).value,
                  })
                }
              />
              {models.length > 0 && (
                <datalist id="model-suggestions">
                  {models.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </div>

            <div class="form-group">
              <label class="form-label">Reasoning Effort (Optional)</label>
              <select
                class="form-input"
                value={cfg.reasoning || "none"}
                onChange={(e) =>
                  setCfg({
                    ...cfg,
                    reasoning: (e.target as HTMLSelectElement).value,
                  })
                }
              >
                <option value="none">Off (No thinking tokens)</option>
                <option value="low">Low</option>
                <option value="medium">Medium</option>
                <option value="high">High</option>
                <option value="xhigh">Extra High (xhigh)</option>
              </select>
            </div>

            <div class="ai-actions-bar">
              <button
                class="btn-test"
                onClick={testConnection}
                disabled={testing}
              >
                {testing ? "Testing..." : "Test Connection"}
              </button>
              {testResult && (
                <span
                  class={`test-status ${
                    testResult.startsWith("✓") ? "ok" : "err"
                  }`}
                >
                  {testResult}
                </span>
              )}
            </div>
          </div>
        )}
      </section>

      <div class="settings-save-bar">
        <button class="btn-save" onClick={saveSettings}>
          {saved ? "Saved ✓" : "Save Changes"}
        </button>
      </div>
    </div>
  );
}

export default App;
