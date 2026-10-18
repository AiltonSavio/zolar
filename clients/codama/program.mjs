// Codama definition of the Header Relay program. The program is a hand-written Pinocchio
// program with no IDL, so this file is the interface description the clients are generated
// from. Layouts and instruction data must match programs/header-relay/src (state.rs,
// processor.rs); the LiteSVM tests drive the program only through the generated client, so
// any drift fails there.
import {
  accountNode,
  argumentValueNode,
  accountValueNode,
  arrayTypeNode,
  bytesTypeNode,
  constantPdaSeedNodeFromString,
  definedTypeLinkNode,
  definedTypeNode,
  errorNode,
  fieldDiscriminatorNode,
  fixedCountNode,
  fixedSizeTypeNode,
  instructionAccountNode,
  instructionArgumentNode,
  instructionNode,
  numberTypeNode,
  numberValueNode,
  pdaNode,
  pdaSeedValueNode,
  pdaValueNode,
  programNode,
  publicKeyTypeNode,
  publicKeyValueNode,
  rootNode,
  sizePrefixTypeNode,
  structFieldTypeNode,
  structTypeNode,
  variablePdaSeedNode,
} from 'codama';

export const PROGRAM_ID = '6F4o7Fphgz5MS4BZrqbE4NyaLUKiAprDNdYTxif4sg9D';
const SYSTEM_PROGRAM = '11111111111111111111111111111111';

const u8 = numberTypeNode('u8');
const u16 = numberTypeNode('u16');
const u32 = numberTypeNode('u32');
const u64 = numberTypeNode('u64');
const u128 = numberTypeNode('u128');
const bytes = (n) => fixedSizeTypeNode(bytesTypeNode(), n);
const field = (name, type, docs = []) => structFieldTypeNode({ name, type, docs });
const discriminatorField = (value) =>
  structFieldTypeNode({
    name: 'discriminator',
    type: u8,
    defaultValue: numberValueNode(value),
    defaultValueStrategy: 'omitted',
  });

const CONTEXT_LEN = 113;

const checkpoint = definedTypeNode({
  name: 'checkpoint',
  docs: [
    'The immutable starting point of a Header Relay. The SHA-256 of its 956 serialised bytes is the Relay address seed.',
    'times and bits hold the newest 113 blocks up to and including the checkpoint block, oldest first.',
  ],
  type: structTypeNode([
    field('version', u8),
    field('network', u8, ['0 mainnet, 1 testnet.']),
    field('finalityDepth', u16),
    field('nu7Height', u32, ['Height at which NU7 difficulty rules start on this network.']),
    field('height', u32, ['Height of the checkpoint block.']),
    field('nonce', u64, ['Lets one deployer create several Relays from the same checkpoint data.']),
    field('hash', bytes(32), ['Checkpoint block hash, internal byte order.']),
    field('times', arrayTypeNode(u32, fixedCountNode(CONTEXT_LEN))),
    field('bits', arrayTypeNode(u32, fixedCountNode(CONTEXT_LEN))),
  ]),
});

const headerSlot = definedTypeNode({
  name: 'headerSlot',
  docs: [
    'One accepted header in the Store. Work and height are measured from the checkpoint block (work 0).',
    'The parent is valid only while the slot at parentSlot still has sequence number parentSeq (slot 65535 is the checkpoint block).',
  ],
  type: structTypeNode([
    field('hash', bytes(32)),
    field('merkleRoot', bytes(32)),
    field('work', u128),
    field('height', u32),
    field('time', u32),
    field('bits', u32),
    field('seq', u32),
    field('parentSlot', u16),
    field('parentSeq', u32),
  ]),
});

const relayAccount = accountNode({
  name: 'relay',
  discriminators: [fieldDiscriminatorNode('discriminator')],
  pda: definedPda('relay'),
  data: structTypeNode([
    discriminatorField(1),
    field('version', u8),
    field('state', u8, ['0 pending (checkpoint being written), 1 active.']),
    field('halted', u8),
    field('creator', publicKeyTypeNode()),
    field('tipHeight', u32, ['Height of the heaviest known header.']),
    field('tipSlot', u16),
    field('tipSeq', u32),
    field('tipWork', u128, ['Cumulative work of the tip since the checkpoint block.']),
    field('tipHash', bytes(32)),
    field('checkpoint', definedTypeLinkNode('checkpoint')),
    field('reserved', bytes(128), ['Zeroed space for fields added by later tickets.']),
  ]),
});

const storeAccount = accountNode({
  name: 'store',
  docs: ['Followed by `capacity` HeaderSlot records (102 bytes each); `count` headers were ever stored.'],
  discriminators: [fieldDiscriminatorNode('discriminator')],
  pda: definedPda('store'),
  data: structTypeNode([
    discriminatorField(2),
    field('version', u8),
    field('relay', publicKeyTypeNode()),
    field('capacity', u16),
    field('count', u32),
  ]),
});

const bufferAccount = accountNode({
  name: 'headerBuffer',
  discriminators: [fieldDiscriminatorNode('discriminator')],
  pda: definedPda('headerBuffer'),
  data: structTypeNode([
    discriminatorField(3),
    field('version', u8),
    field('accepted', u8),
    field('relay', publicKeyTypeNode()),
    field('authority', publicKeyTypeNode()),
    field('header', bytes(1487)),
    field('verifier', bytes(4488), ['Equihash verifier state, see zcash-header equihash module.']),
  ]),
});

function definedPda(name) {
  return { kind: 'pdaLinkNode', name };
}

const pdas = [
  pdaNode({
    name: 'relay',
    seeds: [
      constantPdaSeedNodeFromString('utf8', 'relay'),
      variablePdaSeedNode('checkpointDigest', bytes(32), 'SHA-256 of the serialised Checkpoint.'),
    ],
  }),
  pdaNode({
    name: 'store',
    seeds: [
      constantPdaSeedNodeFromString('utf8', 'store'),
      variablePdaSeedNode('relay', publicKeyTypeNode()),
    ],
  }),
  pdaNode({
    name: 'headerBuffer',
    seeds: [
      constantPdaSeedNodeFromString('utf8', 'buffer'),
      variablePdaSeedNode('relay', publicKeyTypeNode()),
      variablePdaSeedNode('authority', publicKeyTypeNode()),
      variablePdaSeedNode('nonce', u8),
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

const systemProgram = instructionAccountNode({
  name: 'systemProgram',
  isWritable: false,
  isSigner: false,
  defaultValue: publicKeyValueNode(SYSTEM_PROGRAM, 'systemProgram'),
});

const relayPda = (arg) =>
  pdaValueNode('relay', [pdaSeedValueNode('checkpointDigest', argumentValueNode(arg))]);

const instructions = [
  instructionNode({
    name: 'createRelay',
    docs: [
      'Creates the Relay and Store accounts for a Checkpoint whose digest is given. The Relay stays pending until the checkpoint bytes are written and sealed.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'payer', isSigner: true, isWritable: true }),
      instructionAccountNode({
        name: 'relay',
        isWritable: true,
        isSigner: false,
        defaultValue: relayPda('checkpointDigest'),
      }),
      instructionAccountNode({
        name: 'store',
        isWritable: true,
        isSigner: false,
        defaultValue: pdaValueNode('store', [pdaSeedValueNode('relay', accountValueNode('relay'))]),
      }),
      systemProgram,
    ],
    arguments: [disc(0), instructionArgumentNode({ name: 'checkpointDigest', type: bytes(32) })],
  }),
  instructionNode({
    name: 'writeCheckpoint',
    docs: ['Writes serialised Checkpoint bytes at an offset. Only the creator, only while pending.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'creator', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'relay', isWritable: true, isSigner: false }),
    ],
    arguments: [
      disc(1),
      instructionArgumentNode({ name: 'offset', type: u16 }),
      instructionArgumentNode({ name: 'bytes', type: sizePrefixTypeNode(bytesTypeNode(), u32) }),
    ],
  }),
  instructionNode({
    name: 'sealRelay',
    docs: [
      'Checks the SHA-256 of the written checkpoint equals the Relay address seed, then activates the Relay. Anyone may call it.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [instructionAccountNode({ name: 'relay', isWritable: true, isSigner: false })],
    arguments: [disc(2)],
  }),
  instructionNode({
    name: 'growStore',
    docs: ['Adds room for 100 more header slots. Allowed only before the Store has wrapped.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'payer', isSigner: true, isWritable: true }),
      instructionAccountNode({ name: 'store', isWritable: true, isSigner: false }),
      systemProgram,
    ],
    arguments: [disc(3)],
  }),
  instructionNode({
    name: 'createBuffer',
    docs: ['Creates the authority\'s header buffer for a Relay. The authority pays the rent and gets it back on close.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: true }),
      instructionAccountNode({ name: 'relay', isWritable: false, isSigner: false }),
      instructionAccountNode({
        name: 'buffer',
        isWritable: true,
        isSigner: false,
        defaultValue: pdaValueNode('headerBuffer', [
          pdaSeedValueNode('relay', accountValueNode('relay')),
          pdaSeedValueNode('authority', accountValueNode('authority')),
          pdaSeedValueNode('nonce', argumentValueNode('nonce')),
        ]),
      }),
      systemProgram,
    ],
    arguments: [disc(4), instructionArgumentNode({ name: 'nonce', type: u8 })],
  }),
  instructionNode({
    name: 'writeBuffer',
    docs: ['Writes header bytes at an offset. Only before verification begins.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
    ],
    arguments: [
      disc(5),
      instructionArgumentNode({ name: 'offset', type: u16 }),
      instructionArgumentNode({ name: 'bytes', type: sizePrefixTypeNode(bytesTypeNode(), u32) }),
    ],
  }),
  instructionNode({
    name: 'beginVerify',
    docs: ['Freezes the header and computes the BLAKE2b midstate.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
    ],
    arguments: [disc(6)],
  }),
  instructionNode({
    name: 'stepVerify',
    docs: ['Hashes and merges the next `leaves` of the Equihash solution (512 in total).'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
    ],
    arguments: [disc(7), instructionArgumentNode({ name: 'leaves', type: u16 })],
  }),
  instructionNode({
    name: 'acceptHeader',
    docs: [
      'After the Equihash solution verified: checks the hash against the target and that the parent is known, then stores the header.',
    ],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'relay', isWritable: true, isSigner: false }),
      instructionAccountNode({ name: 'store', isWritable: true, isSigner: false }),
    ],
    arguments: [disc(8)],
  }),
  instructionNode({
    name: 'resetBuffer',
    docs: ['Clears the verifier state so the buffer can take another header.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: false }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
    ],
    arguments: [disc(9)],
  }),
  instructionNode({
    name: 'closeBuffer',
    docs: ['Closes the buffer and returns all its lamports to the authority.'],
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts: [
      instructionAccountNode({ name: 'authority', isSigner: true, isWritable: true }),
      instructionAccountNode({ name: 'buffer', isWritable: true, isSigner: false }),
    ],
    arguments: [disc(10)],
  }),
];

const errors = [
  ['malformedHeader', 'Header is not 1487 bytes or its solution prefix is wrong'],
  ['malformedState', 'Verifier state has the wrong size'],
  ['wrongStage', 'Verification step called out of order'],
  ['collision', 'Equihash subtrees do not collide'],
  ['outOfOrder', 'Equihash indices are out of order'],
  ['duplicateIndex', 'Equihash index repeated'],
  ['nonZeroRoot', 'Equihash root hash is not zero'],
  ['headerFrozen', 'Header cannot change once verification began'],
  ['invalidBits', 'nBits is not a valid target'],
  ['targetAboveLimit', 'Target is easier than the network proof-of-work limit'],
  ['hashAboveTarget', 'Header hash is above its target'],
  ['unknownParent', 'Parent header is not known to the Relay'],
  ['duplicateHeader', 'Header is already stored'],
  ['notVerified', 'Equihash solution has not been verified'],
  ['relayNotActive', 'Relay checkpoint is not sealed'],
  ['relayHalted', 'Relay is halted'],
  ['checkpointDigestMismatch', 'Checkpoint bytes do not hash to the Relay address seed'],
  ['invalidCheckpoint', 'Checkpoint fields are invalid'],
  ['wrongAccount', 'Account is not the expected Relay, Store or buffer'],
  ['alreadyAccepted', 'Buffer already accepted its header'],
  ['storeNotGrowable', 'Store already wrapped or reached its maximum size'],
  ['relayAlreadySealed', 'Relay checkpoint is already sealed'],
].map(([name, message], i) => errorNode({ name, code: i + 1, message }));

export const root = rootNode(
  programNode({
    name: 'headerRelay',
    publicKey: PROGRAM_ID,
    version: '0.1.0',
    origin: 'shank',
    accounts: [relayAccount, storeAccount, bufferAccount],
    instructions,
    definedTypes: [checkpoint, headerSlot],
    pdas,
    errors,
  }),
);
