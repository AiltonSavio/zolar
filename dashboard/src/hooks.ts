import { useEffect, useRef, useState } from 'react';
import type { Config } from './config';
import type { EscrowEvent } from './model/claims';
import type { EscrowView } from './model/escrow';
import {
  Rpc,
  fetchEscrows,
  fetchEvent,
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

export interface EscrowsData {
  escrows: EscrowView[];
  decimals: Record<string, number>;
}

export function useEscrows(rpc: Rpc, config: Config): Polled<EscrowsData> {
  const decimals = useRef<Record<string, number>>({});
  return usePoll(async () => {
    const escrows = await fetchEscrows(rpc, config.escrowProgram);
    const missing = [...new Set(escrows.map((e) => e.mint))].filter((m) => !(m in decimals.current));
    Object.assign(decimals.current, await fetchMintDecimals(rpc, missing));
    return { escrows, decimals: { ...decimals.current } };
  }, config.escrowPollMs);
}

const FIRST_LOAD = 8;
const KEEP = 40;

/** Escrow program transactions as they land, newest first. Failed transactions are left out. */
export function useEvents(rpc: Rpc, config: Config): Polled<EscrowEvent[]> {
  const events = useRef<EscrowEvent[]>([]);
  const newest = useRef<string | undefined>(undefined);
  return usePoll(async () => {
    const sigs = await fetchNewSignatures(rpc, config.escrowProgram, newest.current, newest.current ? 25 : FIRST_LOAD);
    const fresh: EscrowEvent[] = [];
    for (const s of sigs.filter((x) => x.err == null)) {
      if (fresh.length > 0) await new Promise((r) => setTimeout(r, 400));
      const ev = await fetchEvent(rpc, s.signature, config.escrowProgram);
      if (ev) fresh.push(ev);
    }
    if (sigs[0]) newest.current = sigs[0].signature;
    const known = new Set(events.current.map((e) => e.signature));
    events.current = [...fresh.filter((e) => !known.has(e.signature)), ...events.current].slice(0, KEEP);
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
