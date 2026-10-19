import type { ReadonlyUint8Array } from '@solana/kit';
import { getRelayDecoder } from '@relay-client';
import { toExplorerHash, toHex } from './hex';

const ROOT_HISTORY = 128;
const RELAY_STATE_ACTIVE = 1;

export type Halt =
  | { halted: false }
  | { halted: true; reason: string; haltHeight: number; haltHash: string };

export interface RelayView {
  sealed: boolean;
  network: 'mainnet' | 'testnet' | 'unknown';
  tipHeight: number;
  tipHash: string;
  chainWork: bigint;
  finalityDepth: number;
  /** Leaves in the Final Block Tree. */
  treeSize: number;
  finalizedHeight: number;
  latestRoot: string;
  halt: Halt;
}

const HALT_REASONS: Record<number, string> = {
  1: 'Deep reorg: the heaviest chain reorganised past a final block',
  2: 'The Final Block Tree is full',
};

export function toRelayView(data: ReadonlyUint8Array): RelayView {
  const r = getRelayDecoder().decode(data);
  const halt: Halt = r.halted
    ? {
        halted: true,
        reason: HALT_REASONS[r.halted] ?? `Halted (reason ${r.halted})`,
        haltHeight: r.haltHeight,
        haltHash: toExplorerHash(r.haltHash),
      }
    : { halted: false };
  const root = r.roots[r.leafCount % ROOT_HISTORY];
  return {
    sealed: r.state === RELAY_STATE_ACTIVE,
    network: r.checkpoint.network === 0 ? 'mainnet' : r.checkpoint.network === 1 ? 'testnet' : 'unknown',
    tipHeight: r.tipHeight,
    tipHash: toExplorerHash(r.tipHash),
    chainWork: r.tipWork,
    finalityDepth: r.checkpoint.finalityDepth,
    treeSize: r.leafCount,
    finalizedHeight: r.finalizedHeight,
    latestRoot: root ? toHex(root) : '',
    halt,
  };
}
