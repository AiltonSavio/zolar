import type { VendingPayment, VendingProof } from '@zolar/sdk';
export type { VendingChain, VendingConnection, VendingSession, VendingSnapshot } from '@zolar/sdk';
export interface VendingProver {
  findPayment(machine: string, nonce: Uint8Array): Promise<VendingPayment | null>;
  prove(machine: string, nonce: Uint8Array, txid: string): Promise<VendingProof>;
}
