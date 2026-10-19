/**
 * Devnet demo helper: creates an Escrow with a random Payment Commitment, so the
 * dashboard has something to show without a Zcash payment. It can never be
 * claimed (nobody knows a txid behind the commitment) and is taken back with
 * `zolar-payee refund --escrow <address> --wait`.
 *
 *   pnpm demo:escrow -- --keypair <payee key file> --payout <wallet> \
 *     [--mint M] [--amount 5000000] [--bond 0] [--private-only] [--expiry-ahead 2]
 *
 * The key file is read by path and never printed.
 */
import { randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';
import {
  address,
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createTransactionMessage,
  getAddressEncoder,
  getBase64EncodedWireTransaction,
  getProgramDerivedAddress,
  getSignatureFromTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
  type Address,
  AccountRole,
  type AccountMeta,
  type AccountSignerMeta,
  type Instruction,
} from '@solana/kit';
import { findEscrowPda, getCreateEscrowInstructionAsync } from '@escrow-client';
import { fetchRelay } from '@relay-client';

const TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ATA_PROGRAM = address('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
const SYSTEM_PROGRAM = address('11111111111111111111111111111111');

const { values: a } = parseArgs({
  args: process.argv.slice(2).filter((x) => x !== '--'),
  options: {
    keypair: { type: 'string' },
    payout: { type: 'string' },
    mint: { type: 'string', default: '6hcTJNrnzSfL3QNj3uQRsFuC9WHAD48ejMov7Nw7ZyqN' },
    relay: { type: 'string', default: 'C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS' },
    rpc: { type: 'string', default: 'https://api.devnet.solana.com' },
    amount: { type: 'string', default: '5000000' },
    bond: { type: 'string', default: '0' },
    'expiry-ahead': { type: 'string', default: '2' },
    'private-only': { type: 'boolean', default: false },
  },
});
if (!a.keypair || !a.payout) throw new Error('--keypair and --payout are required');

async function ata(owner: Address, mint: Address): Promise<Address> {
  const enc = getAddressEncoder();
  const [pda] = await getProgramDerivedAddress({
    programAddress: ATA_PROGRAM,
    seeds: [enc.encode(owner), enc.encode(TOKEN_PROGRAM), enc.encode(mint)],
  });
  return pda;
}

const rpc = createSolanaRpc(a.rpc);
const payee = await createKeyPairSignerFromBytes(Uint8Array.from(JSON.parse(readFileSync(a.keypair, 'utf8')) as number[]));
const mint = address(a.mint);
const relay = address(a.relay);
const commitment = randomBytes(32);
// Zero the top byte so the value is always below the field modulus.
commitment[0] = 0;

const relayAccount = await fetchRelay(rpc, relay);
const expiryHeight = relayAccount.data.tipHeight + Number(a['expiry-ahead']);

const [escrow] = await findEscrowPda({ payee: payee.address, commitment });
const vault = await ata(escrow, mint);
const createVault: Instruction<string, readonly (AccountMeta | AccountSignerMeta)[]> = {
  programAddress: ATA_PROGRAM,
  accounts: [
    { address: payee.address, role: AccountRole.WRITABLE_SIGNER, signer: payee },
    { address: vault, role: AccountRole.WRITABLE },
    { address: escrow, role: AccountRole.READONLY },
    { address: mint, role: AccountRole.READONLY },
    { address: SYSTEM_PROGRAM, role: AccountRole.READONLY },
    { address: TOKEN_PROGRAM, role: AccountRole.READONLY },
  ],
  data: Uint8Array.of(1), // CreateIdempotent
};
const create = await getCreateEscrowInstructionAsync({
  payee,
  payeeToken: await ata(payee.address, mint),
  vault,
  relay,
  commitment,
  payout: address(a.payout),
  amount: BigInt(a.amount),
  bondRequired: BigInt(a.bond),
  expiryHeight,
  claimMode: a['private-only'] ? 1 : 0,
});

const { value: blockhash } = await rpc.getLatestBlockhash().send();
const signed = await pipe(
  createTransactionMessage({ version: 0 }),
  (m) => setTransactionMessageFeePayerSigner(payee, m),
  (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
  (m) => appendTransactionMessageInstructions([createVault, create], m),
  (m) => signTransactionMessageWithSigners(m),
);
const signature = getSignatureFromTransaction(signed);
await rpc.sendTransaction(getBase64EncodedWireTransaction(signed), { encoding: 'base64' }).send();
for (let i = 0; i < 40; i++) {
  const { value } = await rpc.getSignatureStatuses([signature]).send();
  const s = value[0];
  if (s?.err) throw new Error(`transaction failed: ${JSON.stringify(s.err, (_, v) => (typeof v === 'bigint' ? String(v) : v))}`);
  if (s?.confirmationStatus === 'confirmed' || s?.confirmationStatus === 'finalized') break;
  await new Promise((r) => setTimeout(r, 1000));
}
console.log(`Escrow created ${escrow}`);
console.log(`expiry height ${expiryHeight} (relay tip ${relayAccount.data.tipHeight})`);
console.log(`signature ${signature}`);
