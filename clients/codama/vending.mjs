import {
  accountNode,
  booleanTypeNode,
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
  programNode,
  publicKeyTypeNode,
  publicKeyValueNode,
  rootNode,
  structFieldTypeNode,
  structTypeNode,
  variablePdaSeedNode,
} from 'codama';
export const VENDING_PROGRAM_ID =
  '8XqWKHQiesY5o2eDKyewb9ZHwAZ5gdDMtWGuJogvQ5X5';
const u8 = numberTypeNode('u8'),
  u16 = numberTypeNode('u16'),
  u32 = numberTypeNode('u32'),
  u64 = numberTypeNode('u64'),
  i64 = numberTypeNode('i64');
const bytes = (n) => fixedSizeTypeNode(bytesTypeNode(), n);
const f = (name, type) => structFieldTypeNode({ name, type });
const disc = (value) => ({
  name: 'discriminator',
  type: u8,
  defaultValue: numberValueNode(value),
  defaultValueStrategy: 'omitted',
});
const account = (name, discriminator, fields) =>
  accountNode({
    name,
    discriminators: [fieldDiscriminatorNode('discriminator')],
    data: structTypeNode([
      structFieldTypeNode(disc(discriminator)),
      f('version', u8),
      ...fields,
    ]),
  });
const accounts = [
  account('machine', 1, [
    f('bump', u8),
    f('mintBump', u8),
    f('payee', publicKeyTypeNode()),
    f('seed', bytes(32)),
    f('mint', publicKeyTypeNode()),
    f('relay', publicKeyTypeNode()),
    f('priceZat', u64),
    f('available', u32),
    f('stock', u32),
    f('diversifier', bytes(11)),
    f('svkHash', bytes(32)),
    f('vkeyHash', bytes(32)),
    f('reservationBlocks', u32),
    f('svk', bytes(64)),
  ]),
  account('reservation', 2, [
    f('bump', u8),
    f('machine', publicKeyTypeNode()),
    f('nonce', bytes(32)),
    f('payout', publicKeyTypeNode()),
    f('priceZat', u64),
    f('expiryHeight', u32),
    f('createdAt', i64),
    f('releaseRequested', booleanTypeNode()),
    f('releaseRequestedAt', i64),
  ]),
  account('paymentTag', 3, []),
];
const pdas = [
  [
    'machine',
    'machine',
    [
      ['payee', publicKeyTypeNode()],
      ['seed', bytes(32)],
    ],
  ],
  ['mintAuthority', 'mint', [['machine', publicKeyTypeNode()]]],
  [
    'reservation',
    'reservation',
    [
      ['machine', publicKeyTypeNode()],
      ['nonce', bytes(32)],
    ],
  ],
  [
    'paymentTag',
    'tag',
    [
      ['machine', publicKeyTypeNode()],
      ['paymentTag', bytes(32)],
    ],
  ],
].map(([name, prefix, seeds]) =>
  pdaNode({
    name,
    seeds: [
      constantPdaSeedNodeFromString('utf8', prefix),
      ...seeds.map(([n, t]) => variablePdaSeedNode(n, t)),
    ],
  }),
);
const a = (name, isWritable = false, isSigner = false) =>
  instructionAccountNode({ name, isWritable, isSigner });
const prog = (name, key) =>
  instructionAccountNode({
    name,
    isWritable: false,
    isSigner: false,
    defaultValue: publicKeyValueNode(key),
  });
const token = () =>
  prog('tokenProgram', 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const system = () => prog('systemProgram', '11111111111111111111111111111111');
const ata = () =>
  prog(
    'associatedTokenProgram',
    'ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL',
  );
const ix = (name, tag, accounts, args = []) =>
  instructionNode({
    name,
    discriminators: [fieldDiscriminatorNode('discriminator')],
    accounts,
    arguments: [
      instructionArgumentNode(disc(tag)),
      ...args.map(([name, type]) => instructionArgumentNode({ name, type })),
    ],
  });
// Proof has a u16 little-endian byte length; the public values are exactly 249 bytes.
import { sizePrefixTypeNode } from 'codama';
const instructions = [
  ix(
    'initMachine',
    0,
    [
      a('payee', true, true),
      a('machine', true),
      a('mint', true, true),
      a('mintAuthority'),
      a('relay'),
      token(),
      system(),
    ],
    [
      ['seed', bytes(32)],
      ['priceZat', u64],
      ['stock', u32],
      ['diversifier', bytes(11)],
      ['svk', bytes(64)],
      ['relayAddress', publicKeyTypeNode()],
      ['vkeyHash', bytes(32)],
      ['reservationBlocks', u32],
    ],
  ),
  ix(
    'reserve',
    1,
    [
      a('payer', true, true),
      a('machine', true),
      a('reservation', true),
      a('relay'),
      system(),
    ],
    [['nonce', bytes(32)]],
  ),
  ix(
    'claim',
    2,
    [
      a('submitter', true, true),
      a('machine', true),
      a('reservation', true),
      a('relay'),
      a('payout', true),
      a('mint', true),
      a('mintAuthority'),
      a('payoutToken', true),
      a('paymentTag', true),
      token(),
      ata(),
      system(),
    ],
    [
      ['proof', sizePrefixTypeNode(bytesTypeNode(), u16)],
      ['publicValues', bytes(249)],
    ],
  ),
  ix('requestRelease', 4, [
    a('submitter', false, true),
    a('machine'),
    a('reservation', true),
    a('relay'),
  ]),
  ix('releaseReservation', 3, [
    a('submitter', false, true),
    a('machine', true),
    a('reservation', true),
    a('relay'),
    a('payout', true),
  ]),
];
const names = [
  'WrongAccount',
  'BadConfig',
  'SoldOut',
  'InvalidProof',
  'BadPublicValues',
  'WrongMachine',
  'WrongDiversifier',
  'WrongViewingKey',
  'WrongBranch',
  'UnknownRoot',
  'RelayHalted',
  'RelayNotReadable',
  'Underpaid',
  'PaymentExpired',
  'TagUsed',
  'MissingReservation',
  'ReleaseTooEarly',
  'ArithmeticOverflow',
  'BadTokenAccount',
  'AlreadyInitialized',
  'WrongPayout',
  'ReleaseAlreadyRequested',
  'ReleaseNotRequested',
  'ClaimWindowOpen',
];
const errors = names.map((name, i) =>
  errorNode({
    name: name[0].toLowerCase() + name.slice(1),
    code: i + 1,
    message: name,
  }),
);
export const root = rootNode(
  programNode({
    name: 'vending',
    publicKey: VENDING_PROGRAM_ID,
    version: '0.1.0',
    origin: 'shank',
    accounts,
    instructions,
    pdas,
    errors,
  }),
);
