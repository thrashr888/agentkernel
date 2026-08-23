import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTerm } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import {
  AlertCircle,
  Clock3,
  FileTerminal,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Terminal as TerminalIcon,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import type { SessionEvent, SessionSummary } from "@/lib/types";

interface CastHeader {
  version: number;
  width: number;
  height: number;
  duration?: number;
  title?: string;
  command?: string;
}

interface ParsedCast {
  header: CastHeader;
  events: SessionEvent[];
}

function parseCast(cast: string): ParsedCast {
  const lines = cast.split(/\r?\n/).filter((line) => line.trim().length > 0);
  if (lines.length === 0) throw new Error("The recording is empty.");

  const header = JSON.parse(lines[0]) as CastHeader;
  if (header.version !== 2 || !Number.isFinite(header.width) || !Number.isFinite(header.height)) {
    throw new Error("The recording is not a valid asciicast v2 file.");
  }

  const events: SessionEvent[] = [];
  for (const line of lines.slice(1)) {
    const parsed = JSON.parse(line) as unknown;
    if (!Array.isArray(parsed) || parsed.length !== 3) continue;
    const [time, kind, data] = parsed;
    if (typeof time !== "number" || typeof kind !== "string" || typeof data !== "string") continue;
    if (kind === "o" || kind === "i") {
      events.push({ time, event_type: kind === "o" ? "output" : "input", data });
    }
  }
  return { header, events };
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDuration(seconds?: number) {
  if (!seconds || seconds <= 0) return "0s";
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`;
}

function formatTimestamp(timestamp?: number) {
  if (!timestamp) return "Unknown time";
  return new Date(timestamp * 1000).toLocaleString();
}

function ReplayPlayer({ cast, fallbackDuration }: { cast: string; fallbackDuration?: number }) {
  const [parsed, setParsed] = useState<ParsedCast | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const terminalContainer = useRef<HTMLDivElement>(null);
  const terminal = useRef<XTerm | null>(null);
  const events = useRef<SessionEvent[]>([]);
  const elapsedRef = useRef(0);
  const renderedTime = useRef(Number.NEGATIVE_INFINITY);
  const renderedEvent = useRef(0);
  const animationFrame = useRef<number | null>(null);

  useEffect(() => {
    try {
      setParsed(parseCast(cast));
      setParseError(null);
      elapsedRef.current = 0;
      setElapsed(0);
      setPlaying(false);
    } catch (error) {
      setParsed(null);
      setParseError(error instanceof Error ? error.message : "Unable to read recording.");
    }
  }, [cast]);

  const duration = parsed?.header.duration ?? fallbackDuration ??
    (parsed?.events.length ? parsed.events[parsed.events.length - 1].time : 0);

  const renderAt = useCallback((time: number, reset = false) => {
    const instance = terminal.current;
    if (!instance) return;
    if (reset || time < renderedTime.current) {
      instance.reset();
      renderedEvent.current = 0;
    }
    const output: string[] = [];
    while (
      renderedEvent.current < events.current.length
      && events.current[renderedEvent.current].time <= time
    ) {
      const event = events.current[renderedEvent.current];
      if (event.event_type === "output") output.push(event.data);
      renderedEvent.current += 1;
    }
    if (output.length > 0) instance.write(output.join(""));
    renderedTime.current = time;
    elapsedRef.current = time;
    setElapsed(time);
  }, []);

  useEffect(() => {
    const container = terminalContainer.current;
    if (!parsed || !container) return;
    events.current = parsed.events;
    renderedTime.current = Number.NEGATIVE_INFINITY;
    renderedEvent.current = 0;
    const instance = new XTerm({
      cols: parsed.header.width,
      rows: parsed.header.height,
      convertEol: true,
      cursorBlink: false,
      disableStdin: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
      fontSize: 12,
      theme: { background: "#0a0a0a", foreground: "#f5f5f5" },
    });
    const fit = new FitAddon();
    instance.loadAddon(fit);
    instance.open(container);
    fit.fit();
    terminal.current = instance;
    renderAt(0, true);
    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(container);
    return () => {
      observer.disconnect();
      instance.dispose();
      terminal.current = null;
    };
  }, [parsed, renderAt]);

  useEffect(() => {
    if (!playing || !parsed) return;
    const startedAt = performance.now() - (elapsedRef.current * 1000) / speed;
    const tick = (now: number) => {
      const next = Math.min(duration, ((now - startedAt) / 1000) * speed);
      renderAt(next);
      if (next >= duration) {
        animationFrame.current = null;
        setPlaying(false);
      } else {
        animationFrame.current = window.requestAnimationFrame(tick);
      }
    };
    animationFrame.current = window.requestAnimationFrame(tick);
    return () => {
      if (animationFrame.current !== null) window.cancelAnimationFrame(animationFrame.current);
      animationFrame.current = null;
    };
  }, [duration, parsed, playing, renderAt, speed]);

  if (parseError) {
    return <Card><CardContent className="flex items-center gap-3 pt-6 text-sm text-destructive"><AlertCircle className="h-4 w-4 shrink-0" />{parseError}</CardContent></Card>;
  }
  if (!parsed) return <Skeleton className="h-[420px] rounded-lg" />;

  return (
    <Card className="overflow-hidden">
      <CardHeader className="border-b bg-muted/20 pb-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base"><TerminalIcon className="h-4 w-4" />Terminal replay</CardTitle>
            <p className="mt-1 text-xs text-muted-foreground">{parsed.header.width} × {parsed.header.height} · {parsed.events.length} events</p>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => { setPlaying(false); renderAt(0); }}><RotateCcw className="mr-1.5 h-3.5 w-3.5" /> Restart</Button>
            <Button size="sm" onClick={() => setPlaying(!playing)} disabled={duration <= 0}>{playing ? <Pause className="mr-1.5 h-3.5 w-3.5" /> : <Play className="mr-1.5 h-3.5 w-3.5" />}{playing ? "Pause" : "Play"}</Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-4 p-4">
        <div className="rounded-md border border-neutral-800 bg-neutral-950 p-4 shadow-inner">
          <div ref={terminalContainer} className="h-[320px] max-h-[520px] overflow-auto" aria-label="Asciicast terminal replay" />
          {elapsed === 0 && <p className="pointer-events-none -mt-8 px-2 text-xs text-neutral-600">Press Play to start the recording.</p>}
        </div>
        <div className="flex items-center gap-3">
          <span className="w-12 text-right font-mono text-xs text-muted-foreground">{formatDuration(elapsed)}</span>
          <input aria-label="Replay position" className="h-1.5 flex-1 accent-primary" type="range" min={0} max={Math.max(duration, 0.1)} step={0.01} value={Math.min(elapsed, duration)} onChange={(event) => { setPlaying(false); renderAt(Number(event.target.value)); }} />
          <span className="w-12 font-mono text-xs text-muted-foreground">{formatDuration(duration)}</span>
          <select aria-label="Replay speed" className="rounded border bg-background px-2 py-1 text-xs" value={speed} onChange={(event) => setSpeed(Number(event.target.value))}>
            {[0.5, 1, 2, 4].map((value) => <option key={value} value={value}>{value}×</option>)}
          </select>
        </div>
      </CardContent>
    </Card>
  );
}

function RecordingListItem({ recording, selected, onSelect }: { recording: SessionSummary; selected: boolean; onSelect: () => void }) {
  return (
    <button className={`w-full rounded-md border p-3 text-left transition-colors ${selected ? "border-primary bg-accent" : "hover:bg-accent/50"}`} onClick={onSelect}>
      <div className="flex items-start justify-between gap-2"><span className="min-w-0 truncate font-medium">{recording.title || recording.id}</span><Badge variant="outline" className="shrink-0">{formatDuration(recording.duration)}</Badge></div>
      <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">{recording.filename}</p>
      <div className="mt-2 flex items-center gap-3 text-[11px] text-muted-foreground"><span>{formatTimestamp(recording.timestamp)}</span><span>{recording.event_count} events</span><span>{formatBytes(recording.size_bytes)}</span></div>
    </button>
  );
}

export function Sessions() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const { data: sessions, isLoading, error, refetch } = useQuery({ queryKey: ["sessions"], queryFn: () => api.listSessions(), refetchInterval: 5000 });
  const sorted = useMemo(() => sessions ? [...sessions].sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0)) : [], [sessions]);
  const selected = sorted.find((recording) => recording.id === selectedId) ?? sorted[0];
  const castQuery = useQuery({ queryKey: ["session-cast", selected?.id], queryFn: () => api.getSessionCast(selected!.id), enabled: Boolean(selected) });

  if (isLoading) return <div className="space-y-6"><Skeleton className="h-10 w-48" /><Skeleton className="h-[420px] rounded-lg" /></div>;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between"><div><h1 className="text-3xl font-bold tracking-tight">Sessions</h1><p className="text-muted-foreground">Replay recorded terminal sessions from asciicast files.</p></div><Button variant="outline" size="sm" onClick={() => refetch()}><RefreshCw className="mr-2 h-4 w-4" /> Refresh</Button></div>
      {error ? <Card><CardContent className="flex items-center gap-3 pt-6 text-sm text-destructive"><AlertCircle className="h-4 w-4" /> Failed to load recordings: {error instanceof Error ? error.message : String(error)}</CardContent></Card> : sorted.length === 0 ? <Card><CardContent className="flex flex-col items-center justify-center py-16 text-center"><FileTerminal className="mb-4 h-12 w-12 text-muted-foreground/30" /><p className="text-muted-foreground">No session recordings yet.</p><p className="mt-1 text-xs text-muted-foreground">Use <code>agentkernel attach --record</code> or <code>agentkernel ssh --record</code> to create one.</p></CardContent></Card> : (
        <div className="grid gap-6 xl:grid-cols-[minmax(260px,340px)_1fr]">
          <Card className="h-fit"><CardHeader className="pb-3"><CardTitle className="text-base">Recordings <span className="font-normal text-muted-foreground">({sorted.length})</span></CardTitle></CardHeader><CardContent className="space-y-2">{sorted.map((recording) => <RecordingListItem key={recording.id} recording={recording} selected={recording.id === selected?.id} onSelect={() => setSelectedId(recording.id)} />)}</CardContent></Card>
          <div className="space-y-4">
            {selected && <Card><CardContent className="grid gap-3 pt-6 sm:grid-cols-3"><div><p className="text-xs text-muted-foreground">Recording</p><p className="truncate font-medium">{selected.title || selected.id}</p></div><div><p className="text-xs text-muted-foreground">Command</p><p className="truncate font-mono text-xs">{selected.command || "Unknown command"}</p></div><div><p className="text-xs text-muted-foreground">Recorded</p><p className="flex items-center gap-1 text-sm"><Clock3 className="h-3.5 w-3.5 text-muted-foreground" />{formatTimestamp(selected.timestamp)}</p></div></CardContent></Card>}
            {castQuery.error ? <Card><CardContent className="pt-6 text-sm text-destructive">Failed to load this recording: {castQuery.error instanceof Error ? castQuery.error.message : String(castQuery.error)}</CardContent></Card> : castQuery.data ? <ReplayPlayer cast={castQuery.data} fallbackDuration={selected.duration} /> : <Skeleton className="h-[480px] rounded-lg" />}
          </div>
        </div>
      )}
    </div>
  );
}
