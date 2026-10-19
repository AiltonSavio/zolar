import { useEffect, useRef, useState } from 'react';
import type { Config } from './config';
import type { EscrowEvent } from './model/claims';
import type { EscrowView } from './model/escrow';
import {
  Rpc,
  fetchEscrows,
  fetchEvents,
  fetchMintDecimals,
  fetchNewSignatures,
  fetchRelays,
  type RelayReading,
} from './rpc';

export interface Polled<T> {
  data: T | undefined;
  error: string | undefined;
  updatedAt: number | undefined;
}

/** Runs `task` now and again `ms` after each run finishes; pauses while the tab is hidden. */
function usePoll<T>(task: () => Promise<T>, ms: number): Polled<T> {
  const [state, setState] = useState<Polled<T>>({ data: undefined, error: undefined, updatedAt: undefined });
  const taskRef = useRef(task);
  useEffect(() => {
    taskRef.current = task;
  });
  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const run = async () => {
      if (!document.hidden) {
        try {
          const data = await taskRef.current();
          if (!stopped) setState({ data, error: undefined, updatedAt: Date.now() });
        } catch (e) {
          if (!stopped) setState((s) => ({ ...s, error: (e as Error).message }));
        }
      }
      if (!stopped) timer = setTimeout(run, ms);
    };
    void run();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [ms]);
  return state;
}

export function useRpc(config: Config): Rpc {
  const [rpc] = useState(() => new Rpc(config.rpcUrl));
  return rpc;
}

export function useRelays(rpc: Rpc, config: Config): Polled<RelayReading[]> {
  const addresses = config.relays.map((r) => r.address);
  return usePoll(() => fetchRelays(rpc, config.relayProgram, addresses), config.relayPollMs);
}

export function useEscrows(rpc: Rpc, config: Config): Polled<EscrowView[]> {
  return usePoll(() => fetchEscrows(rpc, config.escrowProgram), config.escrowPollMs);
}

/** Decimals of every mint seen so far; each mint is looked up once. */
export function useMintDecimals(rpc: Rpc, mints: string[]): Record<string, number> {
  const [decimals, setDecimals] = useState<Record<string, number>>({});
  const asked = useRef(new Set<string>());
  const wanted = [...new Set(mints)].sort().join(',');
  useEffect(() => {
    const missing = wanted.split(',').filter((m) => m && !asked.current.has(m));
    if (missing.length === 0) return;
    missing.forEach((m) => asked.current.add(m));
    fetchMintDecimals(rpc, missing)
      .then((found) => setDecimals((d) => ({ ...d, ...found })))
      .catch(() => missing.forEach((m) => asked.current.delete(m)));
  }, [rpc, wanted]);
  return decimals;
}

const FIRST_LOAD = 8;
const PAGE = 25;
const MAX_PAGES = 4;
const KEEP = 40;
const MAX_ATTEMPTS = 5;

/**
 * Escrow program transactions as they land, newest first. Failed transactions are left out.
 * A transaction the RPC cannot return yet is retried on later polls (up to MAX_ATTEMPTS) without
 * holding back the others.
 */
export function useEvents(rpc: Rpc, config: Config): Polled<EscrowEvent[]> {
  const events = useRef<EscrowEvent[]>([]);
  const newest = useRef<string | undefined>(undefined);
  const retry = useRef(new Map<string, number>());
  return usePoll(async () => {
    const sigs = newest.current
      ? await fetchNewSignatures(rpc, config.escrowProgram, newest.current, PAGE, MAX_PAGES)
      : await fetchNewSignatures(rpc, config.escrowProgram, undefined, FIRST_LOAD, 1);
    if (sigs[0]) newest.current = sigs[0].signature;
    const todo = [...sigs.filter((x) => x.err == null).map((x) => x.signature)];
    for (const sig of retry.current.keys()) if (!todo.includes(sig)) todo.push(sig);

    const fresh: EscrowEvent[] = [];
    let first = true;
    for (const sig of todo) {
      if (!first) await new Promise((r) => setTimeout(r, 400));
      first = false;
      try {
        fresh.push(...(await fetchEvents(rpc, sig, config.escrowProgram)));
        retry.current.delete(sig);
      } catch {
        const attempts = (retry.current.get(sig) ?? 0) + 1;
        if (attempts >= MAX_ATTEMPTS) retry.current.delete(sig);
        else retry.current.set(sig, attempts);
      }
    }
    const key = (e: EscrowEvent) => `${e.signature}:${e.index}`;
    const known = new Set(events.current.map(key));
    const merged = [...fresh.filter((e) => !known.has(key(e))), ...events.current];
    events.current = merged.sort((a, b) => b.slot - a.slot || a.index - b.index).slice(0, KEEP);
    return events.current;
  }, config.escrowPollMs);
}

/** Re-renders every second so relative times stay current. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return now;
}
