import { getBase64Encoder } from '@solana/kit';
import { classifyTransaction, type EscrowEvent, type RpcTransaction } from './model/claims';
import { toEscrowView, type EscrowView } from './model/escrow';
import { toRelayView, type RelayView } from './model/relay';

const MAX_MULTIPLE_ACCOUNTS = 100;
const ESCROW_ACCOUNT_SIZE = 300;
const ESCROW_DISCRIMINATOR_B58 = '2'; // base58 of the single byte 0x01
// A hung request would keep the page showing "Live" over stale data.
const REQUEST_TIMEOUT_MS = 20_000;

export class Rpc {
  constructor(private readonly url: string) {}

  async call<T>(method: string, params: unknown[]): Promise<T> {
    for (let attempt = 1; ; attempt++) {
      const res = await fetch(this.url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
        signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      });
      const body = (await res.json().catch(() => ({}))) as { result?: T; error?: { code: number; message: string } };
      const limited = res.status === 429 || body.error?.code === 429;
      if (limited && attempt <= 5) {
        await new Promise((r) => setTimeout(r, 1500 * attempt));
        continue;
      }
      if (!res.ok) throw new Error(`${method}: HTTP ${res.status}`);
      if (body.error) throw new Error(`${method}: ${body.error.message}`);
      return body.result as T;
    }
  }
}

interface RawAccount {
  data: [string, 'base64'];
  owner: string;
}

const b64 = getBase64Encoder();
const bytes = (a: RawAccount) => Uint8Array.from(b64.encode(a.data[0]));

export type RelayReading = { view: RelayView } | { error: string };

export async function fetchRelays(
  rpc: Rpc,
  relayProgram: string,
  addresses: string[],
): Promise<RelayReading[]> {
  const { value } = await rpc.call<{ value: (RawAccount | null)[] }>('getMultipleAccounts', [
    addresses,
    { encoding: 'base64', commitment: 'confirmed' },
  ]);
  return value.map((acc): RelayReading => {
    if (!acc) return { error: 'Relay account not found' };
    if (acc.owner !== relayProgram) return { error: 'Account is not owned by the Header Relay program' };
    try {
      return { view: toRelayView(bytes(acc)) };
    } catch (e) {
      return { error: `Cannot decode Relay: ${(e as Error).message}` };
    }
  });
}

export async function fetchEscrows(rpc: Rpc, escrowProgram: string): Promise<EscrowView[]> {
  const accounts = await rpc.call<{ pubkey: string; account: RawAccount }[]>('getProgramAccounts', [
    escrowProgram,
    {
      encoding: 'base64',
      commitment: 'confirmed',
      filters: [
        { dataSize: ESCROW_ACCOUNT_SIZE },
        { memcmp: { offset: 0, bytes: ESCROW_DISCRIMINATOR_B58, encoding: 'base58' } },
      ],
    },
  ]);
  return accounts.map((a) => toEscrowView(a.pubkey, bytes(a.account)));
}

export async function fetchMintDecimals(rpc: Rpc, mints: string[]): Promise<Record<string, number>> {
  const out: Record<string, number> = {};
  for (let i = 0; i < mints.length; i += MAX_MULTIPLE_ACCOUNTS) {
    const batch = mints.slice(i, i + MAX_MULTIPLE_ACCOUNTS);
    const { value } = await rpc.call<{
      value: ({ data: { parsed?: { info?: { decimals?: number } } } } | null)[];
    }>('getMultipleAccounts', [batch, { encoding: 'jsonParsed', commitment: 'confirmed' }]);
    value.forEach((acc, j) => {
      const decimals = acc?.data?.parsed?.info?.decimals;
      const mint = batch[j];
      if (mint && typeof decimals === 'number') out[mint] = decimals;
    });
  }
  return out;
}

/** Signatures of the program's transactions newer than `until` (paging back from the newest), newest first. */
export async function fetchNewSignatures(
  rpc: Rpc,
  program: string,
  until: string | undefined,
  pageSize: number,
  maxPages: number,
): Promise<{ signature: string; err: unknown }[]> {
  const all: { signature: string; err: unknown }[] = [];
  let before: string | undefined;
  for (let page = 0; page < maxPages; page++) {
    const sigs = await rpc.call<{ signature: string; err: unknown }[]>('getSignaturesForAddress', [
      program,
      { limit: pageSize, commitment: 'confirmed', ...(until ? { until } : {}), ...(before ? { before } : {}) },
    ]);
    all.push(...sigs);
    if (sigs.length < pageSize || !until) break;
    before = sigs[sigs.length - 1]?.signature;
  }
  return all;
}

export async function fetchEvents(rpc: Rpc, signature: string, escrowProgram: string): Promise<EscrowEvent[]> {
  const tx = await rpc.call<RpcTransaction | null>('getTransaction', [
    signature,
    { encoding: 'json', commitment: 'confirmed', maxSupportedTransactionVersion: 0 },
  ]);
  if (!tx) throw new Error(`getTransaction ${signature}: not available yet`);
  return classifyTransaction(tx, escrowProgram);
}
