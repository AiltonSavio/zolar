pragma circom 2.1.0;

// Zolar Private Claim (ADR 0002).
//
// Proves, without revealing them, that the holder of a Payment Commitment knows a transaction ID and
// salt behind it, that the transaction is in the SHA-256d Merkle tree of a block, and that the block
// is a leaf of a Final Block Tree with the given root, at a height inside the Escrow's window.
//
// Public inputs, in this order (they are the verifying key's IC points 1..4):
//   root        Final Block Tree root (the Escrow checks it is one of the relay's recent roots)
//   commitment  Poseidon(txid_hi, txid_lo, salt), read from the Escrow
//   lower       lowest allowed block height, computed by the Escrow from created_tip and Finality Depth
//   expiry      highest allowed block height (the Escrow's expiry_height)
//
// Byte conventions match the Header Relay and the Escrow: the transaction ID and every Merkle node are
// in internal byte order; a 256-bit value is a byte string given MSB-first per byte as 256 bits (the
// bit order of circomlib's Sha256); the two limbs are the first and last 16 bytes read big-endian.

include "circomlib/circuits/sha256/sha256.circom";
include "circomlib/circuits/poseidon.circom";
include "circomlib/circuits/bitify.circom";
include "circomlib/circuits/comparators.circom";

// SHA256(SHA256(left || right)): one node of a Bitcoin-style Merkle tree.
template Sha256dNode() {
    signal input left[256];
    signal input right[256];
    signal output out[256];
    component first = Sha256(512);
    component second = Sha256(256);
    for (var i = 0; i < 256; i++) {
        first.in[i] <== left[i];
        first.in[256 + i] <== right[i];
    }
    for (var i = 0; i < 256; i++) {
        second.in[i] <== first.out[i];
    }
    for (var i = 0; i < 256; i++) {
        out[i] <== second.out[i];
    }
}

// One transaction-tree level. `active` = 0 passes the node through (a shorter tree); `isRight` = 1
// means the node is the right child. Both selectors and every sibling bit are constrained boolean.
template TxLevel() {
    signal input cur[256];
    signal input sibling[256];
    signal input isRight;
    signal input active;
    signal output next[256];

    isRight * (isRight - 1) === 0;
    active * (active - 1) === 0;

    component node = Sha256dNode();
    signal toLeft[256];
    signal toRight[256];
    signal pick[256];
    for (var i = 0; i < 256; i++) {
        sibling[i] * (sibling[i] - 1) === 0;
        toLeft[i] <== isRight * (sibling[i] - cur[i]);
        toRight[i] <== isRight * (cur[i] - sibling[i]);
        node.left[i] <== cur[i] + toLeft[i];
        node.right[i] <== sibling[i] + toRight[i];
    }
    for (var i = 0; i < 256; i++) {
        pick[i] <== active * (node.out[i] - cur[i]);
        next[i] <== cur[i] + pick[i];
    }
}

// 256 bits (a byte string, MSB first per byte) to two 128-bit big-endian limbs.
template Limbs() {
    signal input bits[256];
    signal output hi;
    signal output lo;
    component high = Bits2Num(128);
    component low = Bits2Num(128);
    for (var i = 0; i < 128; i++) {
        high.in[i] <== bits[127 - i];
        low.in[i] <== bits[255 - i];
    }
    hi <== high.out;
    lo <== low.out;
}

template PrivateClaim(TX_DEPTH, TREE_DEPTH) {
    // Public inputs.
    signal input root;
    signal input commitment;
    signal input lower;
    signal input expiry;

    // Private inputs.
    signal input txid[256];
    signal input salt;
    signal input txSibling[TX_DEPTH][256];
    signal input txIsRight[TX_DEPTH];
    signal input txActive[TX_DEPTH];
    signal input height;
    signal input leafIndex[TREE_DEPTH];
    signal input treeSibling[TREE_DEPTH];

    // The transaction ID is a 256-bit string.
    for (var i = 0; i < 256; i++) {
        txid[i] * (txid[i] - 1) === 0;
    }

    // Payment Commitment.
    component txLimbs = Limbs();
    for (var i = 0; i < 256; i++) {
        txLimbs.bits[i] <== txid[i];
    }
    component commit = Poseidon(3);
    commit.inputs[0] <== txLimbs.hi;
    commit.inputs[1] <== txLimbs.lo;
    commit.inputs[2] <== salt;
    commit.out === commitment;

    // Transaction Merkle path to the block's root. The active levels are a prefix.
    component level[TX_DEPTH];
    for (var l = 0; l < TX_DEPTH; l++) {
        level[l] = TxLevel();
        for (var i = 0; i < 256; i++) {
            if (l == 0) {
                level[l].cur[i] <== txid[i];
            } else {
                level[l].cur[i] <== level[l - 1].next[i];
            }
            level[l].sibling[i] <== txSibling[l][i];
        }
        level[l].isRight <== txIsRight[l];
        level[l].active <== txActive[l];
        if (l > 0) {
            txActive[l] * (1 - txActive[l - 1]) === 0;
        }
    }
    component blockRoot = Limbs();
    for (var i = 0; i < 256; i++) {
        if (TX_DEPTH == 0) {
            blockRoot.bits[i] <== txid[i];
        } else {
            blockRoot.bits[i] <== level[TX_DEPTH - 1].next[i];
        }
    }

    // Block height: a 32-bit number inside [lower, expiry].
    component heightBits = Num2Bits(32);
    heightBits.in <== height;
    component lowerBits = Num2Bits(32);
    lowerBits.in <== lower;
    component expiryBits = Num2Bits(32);
    expiryBits.in <== expiry;
    component aboveLower = LessEqThan(32);
    aboveLower.in[0] <== lower;
    aboveLower.in[1] <== height;
    aboveLower.out === 1;
    component belowExpiry = LessEqThan(32);
    belowExpiry.in[0] <== height;
    belowExpiry.in[1] <== expiry;
    belowExpiry.out === 1;

    // Final Block Tree: the block's leaf up to the public root. The leaf contains the height, so the
    // position cannot be chosen freely: a leaf for this height exists at one place only.
    component leaf = Poseidon(3);
    leaf.inputs[0] <== height;
    leaf.inputs[1] <== blockRoot.hi;
    leaf.inputs[2] <== blockRoot.lo;

    component node[TREE_DEPTH];
    signal toLeft[TREE_DEPTH];
    signal toRight[TREE_DEPTH];
    signal cur[TREE_DEPTH + 1];
    cur[0] <== leaf.out;
    for (var l = 0; l < TREE_DEPTH; l++) {
        leafIndex[l] * (leafIndex[l] - 1) === 0;
        toLeft[l] <== leafIndex[l] * (treeSibling[l] - cur[l]);
        toRight[l] <== leafIndex[l] * (cur[l] - treeSibling[l]);
        node[l] = Poseidon(2);
        node[l].inputs[0] <== cur[l] + toLeft[l];
        node[l].inputs[1] <== treeSibling[l] + toRight[l];
        cur[l + 1] <== node[l].out;
    }
    cur[TREE_DEPTH] === root;
}
