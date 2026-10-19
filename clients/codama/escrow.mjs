// Codama definition of the Escrow program. Layouts and instruction data must match
// programs/escrow/src (state.rs, processor.rs); the LiteSVM tests in crates/escrow-tests drive
// the program only through the generated client, so any drift fails there.
import {
  accountNode,
  accountValueNode,
  argumentValueNode,
  arrayTypeNode,
  bytesTypeNode,
  constantPdaSeedNodeFromString,
  errorNode,
  fieldDiscriminatorNode,
  fixedSizeTypeNode,
  instructionAccountNode,
  instructionArgumentNode,
  instructionNode,
  numberTypeNode,
  numberValueNode,
  pdaNode,
  pdaSeedValueNode,
  pdaValueNode,
  prefixedCountNode,
  programNode,
  publicKeyTypeNode,
  publicKeyValueNode,
  rootNode,
  structFieldTypeNode,
  structTypeNode,
  variablePdaSeedNode,
} from 'codama';

export const ESCROW_PROGRAM_ID = 'HMew6N4iWjmWMbJBoK6WfQRucBt8ZfWpMadd9mpMtbML';
const SYSTEM_PROGRAM = '11111111111111111111111111111111';
const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';

const u8 = numberTypeNode('u8');
const u32 = numberTypeNode('u32');
const u64 = numberTypeNode('u64');
const i64 = numberTypeNode('i64');
const bytes = (n) => fixedSizeTypeNode(bytesTypeNode(), n);
const field = (name, type, docs = []) => structFieldTypeNode({ name, type, docs });

const escrowAccount = accountNode({
  name: 'escrow',
  docs: ['SPL tokens a Payee locked for a Payer, released by a Claim. 300 bytes, PDA ["escrow", payee, commitment].'],
  discriminators: [fieldDiscriminatorNode('discriminator')],
  pda: { kind: 'pdaLinkNode', name: 'escrow' },
  data: structTypeNode([
    structFieldTypeNode({
      name: 'discriminator',
      type: u8,
      defaultValue: numberValueNode(1),
      defaultValueStrategy: 'omitted',
    }),
    field('version', u8),
    field('state', u8, ['0 open; 1 refund requested (a Claim still settles until the Refund closes the account).']),
    field('bump', u8),
    field('payee', publicKeyTypeNode(), ['Created the Escrow; gets the rent back and, with refunds, the tokens.']),
    field('payout', publicKeyTypeNode(), ['The Payer wallet that receives the tokens.']),
    field('mint', publicKeyTypeNode()),
    field('vault', publicKeyTypeNode(), ['Token account holding the tokens, owned by the Escrow PDA.']),
    field('relay', publicKeyTypeNode(), ['The Header Relay this Escrow trusts.']),
    field('commitment', bytes(32), ['Payment Commitment: Poseidon(txid_hi, txid_lo, salt).']),
    field('amount', u64),
    field('bondRequired', u64, ['Tokens of the mint the Payer must lock before a Claim can settle.']),
    field('bondDeposited', u64, ['Equals bondRequired once the Payer deposited; the Bond sits in the vault.']),
    field('createdAt', i64, ['Solana unix time of creation.']),
    field('createdTip', u32, ['Relay tip height at creation; a Claim must prove a block above it.']),
    field('expiryHeight', u32, ['Zcash height by which the payment must be mined.']),
    field('claimMode', u8, ['0 a Reveal Claim or a Private Claim may settle; 1 only a Private Claim may.']),
    field('refundRequestedAt', i64, ['Solana unix time of RequestRefund; 0 while state is 0.']),
    field('reserved', bytes(55)),
  ]),
});

const pdas = [
  pdaNode({
    name: 'escrow',
    seeds: [
      constantPdaSeedNodeFromString('utf8', 'escrow'),
      variablePdaSeedNode('payee', publicKeyTypeNode()),
      variablePdaSeedNode('commitment', bytes(32)),
    ],
  }),
];

const disc = (value) =>
  instructionArgumentNode({
    name: 'discriminator',
    type: u8,
    defaultValue: numberValueNode(value),
    defaultValueStrategy: 'omitted',
  });

const program = (name, address) =>
  instructionAccountNode({
    name,
    isWritable: false,
    isSigner: false,
    defaultValue: publicKeyValueNode(address, name),
  });

const instructions = [
  instructionNode({
    name: 'createEscrow',
    docs: [
      'Locks `amount` tokens from the payee in the vault for the Payer wallet `payout`. The vault is a fresh, empty token account of the mint whose owner is the Escrow PDA.',
      'The expiry must lie in (relay tip, relay tip + 20].',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'payee', isSigner: true, isWritable: true }),
      instructionAccountNode({
        name: 'escrow',
        isWritable: true,
        isSigner: false,
        defaultValue: pdaValueNode('escrow', [
          pdaSeedValueNode('payee', accountValueNode('payee')),
          pdaSeedValueNode('commitment', argumentValueNode('commitment')),
        ]),
      }),
      instructionAccountNode({ name: 'payeeToken', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'vault', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
      program('tokenProgram', TOKEN_PROGRAM),
      program('systemProgram', SYSTEM_PROGRAM),
    ],
    arguments: [
      disc(0),
      instructionArgumentNode({ name: 'commitment', type: bytes(32) }),
      instructionArgumentNode({ name: 'payout', type: publicKeyTypeNode() }),
      instructionArgumentNode({ name: 'amount', type: u64 }),
      instructionArgumentNode({ name: 'bondRequired', type: u64 }),
      instructionArgumentNode({ name: 'expiryHeight', type: u32 }),
      instructionArgumentNode({ name: 'claimMode', type: u8, docs: ['0: a Reveal Claim or a Private Claim may settle. 1: only a Private Claim may, so the transaction ID is never published.'] }),
    ],
  }),
  instructionNode({
    name: 'revealClaim',
    docs: [
      'Releases the Escrow to its Payer wallet. Anyone may submit it; the tokens go to `payout` whoever signs.',
      'txPath holds the SHA-256d siblings from the transaction to the block Merkle root (position txIndex); treePath the Poseidon siblings of the block leaf (index blockHeight - checkpointHeight - 1) in the Final Block Tree, with the empty-subtree levels left off; root is one of the relay\'s recent roots.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'escrow', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
      instructionAccountNode({ name: 'vault', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payoutToken', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payee', isWritable: true, isSigner: false }),
      program('tokenProgram', TOKEN_PROGRAM),
    ],
    arguments: [
      disc(1),
      instructionArgumentNode({ name: 'txid', type: bytes(32), docs: ['Internal byte order (the order of the block Merkle tree, not the display order).'] }),
      instructionArgumentNode({ name: 'salt', type: bytes(32) }),
      instructionArgumentNode({ name: 'blockHeight', type: u32 }),
      instructionArgumentNode({ name: 'txIndex', type: u32 }),
      instructionArgumentNode({ name: 'root', type: bytes(32) }),
      instructionArgumentNode({ name: 'treePath', type: arrayTypeNode(bytes(32), prefixedCountNode(u8)), docs: ['Poseidon siblings from the leaf up; at most 20. Levels not sent are the empty-subtree constants.'] }),
      instructionArgumentNode({ name: 'txPath', type: arrayTypeNode(bytes(32), prefixedCountNode(u8)) }),
    ],
  }),
  instructionNode({
    name: 'depositBond',
    docs: [
      'The Payer wallet (`payout`) locks exactly `bondRequired` tokens of the mint in the vault, once. The Bond returns to the Payer on a Claim and goes to the Payee on a refund.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'payer', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'escrow', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payerToken', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'vault', isWritable: true, isSigner: false }),
      program('tokenProgram', TOKEN_PROGRAM),
    ],
    arguments: [disc(2)],
  }),
  instructionNode({
    name: 'refund',
    docs: [
      'Second step of a refund: pays the whole vault (tokens and Bond) to the Payee and closes the Escrow, once the claim window since RequestRefund has passed and the refund conditions still hold (a Halt after the request sends the Payee back to the 24 hour backstop). Anyone may submit it.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'escrow', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
      instructionAccountNode({ name: 'vault', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payeeToken', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payee', isWritable: true, isSigner: false }),
      program('tokenProgram', TOKEN_PROGRAM),
    ],
    arguments: [disc(3)],
  }),
  instructionNode({
    name: 'requestRefund',
    docs: [
      'First step of a refund, open to anyone: starts the claim window. Allowed once the trusted relay (not Halted) has a tip above expiryHeight + Finality Depth, or 24 hours after creation whatever the relay says. A Claim still settles during and after the window until the Refund closes the Escrow.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'escrow', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
    ],
    arguments: [disc(5)],
  }),
  instructionNode({
    name: 'privateClaim',
    docs: [
      'Releases the Escrow to its Payer wallet against a Groth16 proof (circuits/src/private_claim.circom) that the committed transaction is in a final block inside the Escrow\'s height window. The transaction ID, salt and block never appear on Solana. Anyone may submit it; the tokens go to `payout` whoever signs. Settles in the same states as a Reveal Claim and needs the Bond deposited.',
      'The public inputs are root (one of the relay\'s recent roots, named here), the Escrow\'s commitment, the lowest allowed height (created tip + 1 - Finality Depth) and the Escrow\'s expiry height, the last three read by the program.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'escrow', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
      instructionAccountNode({ name: 'vault', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payoutToken', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'payee', isWritable: true, isSigner: false }),
      program('tokenProgram', TOKEN_PROGRAM),
    ],
    arguments: [
      disc(4),
      instructionArgumentNode({ name: 'proofA', type: bytes(64), docs: ['Proof point A already negated (y -> q - y), big-endian x || y.'] }),
      instructionArgumentNode({ name: 'proofB', type: bytes(128), docs: ['x_c1 || x_c0 || y_c1 || y_c0, each 32 bytes big-endian.'] }),
      instructionArgumentNode({ name: 'proofC', type: bytes(64) }),
      instructionArgumentNode({ name: 'root', type: bytes(32), docs: ['A Final Block Tree root in the relay\'s recent history.'] }),
    ],
  }),
];

const errors = [
  ['wrongAccount', 'An account is not the one the Escrow expects'],
  ['badExpiry', 'Expiry is not in (relay tip, relay tip + 20]'],
  ['zeroAmount', 'Amount is zero'],
  ['commitmentOutOfField', 'Commitment is not below the BN254 modulus'],
  ['badTokenAccount', 'A token account has the wrong mint, owner, state, delegate or close authority'],
  ['notOpen', 'The Escrow is not open'],
  ['relayHalted', 'The Header Relay is halted'],
  ['commitmentMismatch', 'Transaction ID and salt do not hash to the Payment Commitment'],
  ['blockOutOfRange', 'The block is too far below the relay tip at creation, or above the expiry height'],
  ['badInclusionProof', 'A Merkle path is longer than allowed'],
  ['unknownRoot', 'The Final Block Tree path does not lead to a recent root that already holds the block'],
  ['relayNotReadable', 'The relay account is not a sealed Header Relay'],
  ['poseidonFailed', 'The Poseidon syscall rejected its input'],
  ['bondNotDeposited', 'The Bond the Escrow requires is not deposited'],
  ['claimModeNotAllowed', 'The claim mode is not supported, or the Escrow does not allow a Reveal Claim'],
  ['noBondRequired', 'The Escrow requires no Bond'],
  ['bondAlreadyDeposited', 'The Bond is already deposited'],
  ['refundNotYet', 'Neither the relay tip nor the 24 hour backstop allows a refund yet'],
  ['refundNotRequested', 'No refund was requested'],
  ['claimWindowOpen', 'The claim window since the refund request has not passed'],
  ['notPayer', 'The Bond must be signed and paid by the Payer wallet'],
  ['invalidProof', 'The Groth16 proof does not verify against the Escrow\'s public inputs'],
].map(([name, message], i) => errorNode({ name, code: i + 1, message }));

export const root = rootNode(
  programNode({
    name: 'escrow',
    publicKey: ESCROW_PROGRAM_ID,
    version: '0.1.0',
    origin: 'shank',
    accounts: [escrowAccount],
    instructions,
    pdas,
    errors,
  }),
);
