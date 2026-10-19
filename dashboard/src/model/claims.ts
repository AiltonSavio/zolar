import { getBase58Encoder } from '@solana/kit';
import {
  CREATE_ESCROW_DISCRIMINATOR,
  DEPOSIT_BOND_DISCRIMINATOR,
  PRIVATE_CLAIM_DISCRIMINATOR,
  REFUND_DISCRIMINATOR,
  REQUEST_REFUND_DISCRIMINATOR,
  REVEAL_CLAIM_DISCRIMINATOR,
} from '@escrow-client';

export type EventKind =
  | 'created'
  | 'reveal-claim'
  | 'private-claim'
  | 'bond-deposited'
  | 'refund-requested'
  | 'refund';

/**
 * What the dashboard knows about an Escrow transaction. Deliberately no
 * instruction payload: a Reveal Claim carries the Zcash txid there, and
 * the dashboard must not display or derive it.
 */
export interface EscrowEvent {
  kind: EventKind;
  signature: string;
  slot: number;
  blockTime: number | null;
  escrow: string;
  failed: boolean;
  amount: bigint | null;
  mint: string | null;
}

/** The part of a `getTransaction` (json encoding) response the dashboard reads. */
export interface RpcTransaction {
  slot: number;
  blockTime: number | null;
  transaction: {
    signatures: string[];
    message: {
      accountKeys: string[];
      instructions: { programIdIndex: number; accounts: number[]; data: string }[];
    };
  };
  meta: {
    err: unknown;
    preTokenBalances?: { owner?: string; mint: string; uiTokenAmount: { amount: string } }[] | null;
  } | null;
}

const KINDS: Record<number, EventKind> = {
  [CREATE_ESCROW_DISCRIMINATOR]: 'created',
  [REVEAL_CLAIM_DISCRIMINATOR]: 'reveal-claim',
  [DEPOSIT_BOND_DISCRIMINATOR]: 'bond-deposited',
  [REFUND_DISCRIMINATOR]: 'refund',
  [PRIVATE_CLAIM_DISCRIMINATOR]: 'private-claim',
  [REQUEST_REFUND_DISCRIMINATOR]: 'refund-requested',
};

// The Escrow's position in each instruction's account list (the first
// account for the others, after the payee or payer for these two).
const ESCROW_ACCOUNT_INDEX: Partial<Record<EventKind, number>> = { created: 1, 'bond-deposited': 1 };
const escrowIndex = (kind: EventKind) => ESCROW_ACCOUNT_INDEX[kind] ?? 0;

const base58 = getBase58Encoder();

export function classifyTransaction(tx: RpcTransaction, escrowProgram: string): EscrowEvent | null {
  const { accountKeys, instructions } = tx.transaction.message;
  const ix = instructions.find((i) => accountKeys[i.programIdIndex] === escrowProgram);
  const signature = tx.transaction.signatures[0];
  if (!ix || !signature) return null;
  // Only the instruction tag (first byte) is read; the payload is never decoded.
  const kind = KINDS[base58.encode(ix.data)[0] ?? -1];
  if (!kind) return null;
  const escrow = accountKeys[ix.accounts[escrowIndex(kind)] ?? -1];
  if (!escrow) return null;
  const held = tx.meta?.preTokenBalances?.find((b) => b.owner === escrow);
  return {
    kind,
    signature,
    slot: tx.slot,
    blockTime: tx.blockTime,
    escrow,
    failed: tx.meta?.err != null,
    amount: held ? BigInt(held.uiTokenAmount.amount) : null,
    mint: held?.mint ?? null,
  };
}
