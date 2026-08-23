import type { BenchmarkResult } from "./types";

export const BENCHMARK_HISTORY_KEY = "agentkernel_benchmark_history_v1";
export const LEGACY_BENCHMARK_KEY = "agentkernel_last_benchmark";
export const BENCHMARK_HISTORY_VERSION = 1;
export const MAX_BENCHMARK_HISTORY = 50;

interface BenchmarkHistoryStorage {
  version: number;
  servers: Record<string, BenchmarkResult[]>;
}

function getStorage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

export function normalizeBenchmarkResult(value: unknown): BenchmarkResult | null {
  if (!value || typeof value !== "object") return null;
  const result = value as Record<string, unknown>;

  if (
    !isFiniteNumber(result.create_ms) ||
    !isFiniteNumber(result.exec_ms) ||
    !isFiniteNumber(result.destroy_ms) ||
    !isFiniteNumber(result.total_ms) ||
    !isNonEmptyString(result.image) ||
    !isNonEmptyString(result.timestamp)
  ) {
    return null;
  }

  return {
    create_ms: result.create_ms,
    exec_ms: result.exec_ms,
    destroy_ms: result.destroy_ms,
    total_ms: result.total_ms,
    image: result.image,
    backend: isNonEmptyString(result.backend) ? result.backend : "unknown",
    started_at: typeof result.started_at === "string" ? result.started_at : undefined,
    finished_at: typeof result.finished_at === "string" ? result.finished_at : undefined,
    timestamp: result.timestamp,
  };
}

function timestampValue(result: BenchmarkResult): number {
  const timestamp = Date.parse(result.timestamp);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function newestFirst(results: BenchmarkResult[]): BenchmarkResult[] {
  return [...results]
    .sort((a, b) => timestampValue(b) - timestampValue(a))
    .slice(0, MAX_BENCHMARK_HISTORY);
}

function emptyStorage(): BenchmarkHistoryStorage {
  return { version: BENCHMARK_HISTORY_VERSION, servers: {} };
}

function parseStorage(raw: string | null): BenchmarkHistoryStorage | null {
  if (!raw) return null;

  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const parsedValue = parsed as Record<string, unknown>;
    if (parsedValue.version !== BENCHMARK_HISTORY_VERSION || !parsedValue.servers || typeof parsedValue.servers !== "object") {
      return null;
    }

    const servers: Record<string, BenchmarkResult[]> = {};
    for (const [key, serverHistory] of Object.entries(parsedValue.servers as Record<string, unknown>)) {
      if (!Array.isArray(serverHistory)) continue;
      servers[key] = newestFirst(
        serverHistory
          .map(normalizeBenchmarkResult)
          .filter((result): result is BenchmarkResult => result !== null),
      );
    }
    return { version: BENCHMARK_HISTORY_VERSION, servers };
  } catch {
    return null;
  }
}

function writeStorage(storage: Storage, value: BenchmarkHistoryStorage): boolean {
  try {
    storage.setItem(BENCHMARK_HISTORY_KEY, JSON.stringify(value));
    return true;
  } catch {
    return false;
  }
}

function readStorage(storage: Storage, key: string): string | null {
  try {
    return storage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Build a stable partition identifier from non-secret server configuration.
 * The API key is intentionally not accepted or included here.
 */
export function benchmarkServerKey(name: string, url: string): string {
  return encodeURIComponent(`${name}\n${url}`);
}

export function loadBenchmarkHistory(serverKey: string): BenchmarkResult[] {
  const storage = getStorage();
  if (!storage) return [];

  const current = parseStorage(readStorage(storage, BENCHMARK_HISTORY_KEY));
  if (current && Object.prototype.hasOwnProperty.call(current.servers, serverKey)) {
    return current.servers[serverKey] ?? [];
  }

  // A legacy value was global because it predates multi-server history. When
  // present, migrate it into the currently active server's partition.
  const legacy = normalizeBenchmarkResult(parseJson(readStorage(storage, LEGACY_BENCHMARK_KEY)));
  if (!legacy) return [];

  const migrated = current ?? emptyStorage();
  migrated.servers[serverKey] = [legacy];
  if (writeStorage(storage, migrated)) {
    try {
      storage.removeItem(LEGACY_BENCHMARK_KEY);
    } catch {
      // Keeping the legacy value is harmless if storage cleanup is unavailable.
    }
  }
  return [legacy];
}

function parseJson(raw: string | null): unknown {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

export function appendBenchmarkHistory(
  serverKey: string,
  result: BenchmarkResult,
): BenchmarkResult[] {
  const storage = getStorage();
  if (!storage) return [result];

  // Make appending safe even if the user runs a benchmark before the initial
  // history-loading effect has migrated the legacy value.
  const existing = loadBenchmarkHistory(serverKey);
  const current = parseStorage(readStorage(storage, BENCHMARK_HISTORY_KEY)) ?? emptyStorage();
  const next = newestFirst([result, ...existing]);
  current.servers[serverKey] = next;
  writeStorage(storage, current);
  return next;
}
