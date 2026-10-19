//! The Final Block Tree: an append-only incremental Merkle tree of fixed depth.
//!
//! The hash is a parameter so the logic is tested on the host with a stand-in hash and runs on-chain
//! with Poseidon (`sol_poseidon`). `filled[level]` holds the left sibling still waiting for a right
//! neighbour at that level, as in the usual incremental tree.

use header_relay_interface::TREE_DEPTH;

pub type Node = [u8; 32];

/// Appends `leaf` at position `index` (the number of leaves so far) and returns the new root.
/// `zeros[level]` is the root of an empty subtree of that height; `zeros[0]` is the empty leaf.
pub fn append<E>(
    filled: &mut [Node; TREE_DEPTH],
    zeros: &[Node; TREE_DEPTH],
    index: u64,
    leaf: Node,
    mut hash: impl FnMut(&Node, &Node) -> Result<Node, E>,
) -> Result<Node, E> {
    let (mut node, mut at) = (leaf, index);
    for level in 0..TREE_DEPTH {
        node = if at & 1 == 0 {
            filled[level] = node;
            hash(&node, &zeros[level])?
        } else {
            hash(&filled[level], &node)?
        };
        at >>= 1;
    }
    Ok(node)
}

/// Root of the empty tree.
pub fn empty_root<E>(
    zeros: &[Node; TREE_DEPTH],
    mut hash: impl FnMut(&Node, &Node) -> Result<Node, E>,
) -> Result<Node, E> {
    hash(&zeros[TREE_DEPTH - 1], &zeros[TREE_DEPTH - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for Poseidon: order-sensitive and collision-free enough for structure tests.
    fn toy(a: &Node, b: &Node) -> Result<Node, ()> {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = a[i]
                .wrapping_mul(31)
                .wrapping_add(b[(i + 7) % 32].wrapping_mul(17))
                .wrapping_add(i as u8)
                .rotate_left(3);
        }
        Ok(out)
    }

    fn zeros() -> [Node; TREE_DEPTH] {
        let mut z = [[0u8; 32]; TREE_DEPTH];
        z[0] = toy(&[0; 32], &[0; 32]).unwrap();
        for i in 1..TREE_DEPTH {
            z[i] = toy(&z[i - 1], &z[i - 1]).unwrap();
        }
        z
    }

    /// Recomputes the root of all leaves by building every level, padding with empty subtrees.
    fn naive_root(leaves: &[Node], zeros: &[Node; TREE_DEPTH]) -> Node {
        let mut level: Vec<Node> = leaves.to_vec();
        for z in zeros {
            if level.len() % 2 == 1 {
                level.push(*z);
            }
            level = level
                .chunks(2)
                .map(|p| toy(&p[0], &p[1]).unwrap())
                .collect();
            if level.is_empty() {
                level.push(toy(z, z).unwrap());
            }
        }
        level[0]
    }

    fn leaf(i: u64) -> Node {
        let mut l = [0u8; 32];
        l[24..].copy_from_slice(&(i + 1).to_be_bytes());
        l
    }

    #[test]
    fn the_empty_tree_has_the_root_of_empty_subtrees() {
        let z = zeros();
        assert_eq!(empty_root(&z, toy).unwrap(), naive_root(&[], &z));
    }

    #[test]
    fn incremental_roots_equal_roots_recomputed_from_all_leaves() {
        let z = zeros();
        let mut filled = [[0u8; 32]; TREE_DEPTH];
        let mut leaves = Vec::new();
        for i in 0..70u64 {
            leaves.push(leaf(i));
            let root = append(&mut filled, &z, i, leaf(i), toy).unwrap();
            assert_eq!(root, naive_root(&leaves, &z), "after {} leaves", i + 1);
        }
    }

    #[test]
    fn a_hash_error_is_passed_up() {
        let z = zeros();
        let mut filled = [[0u8; 32]; TREE_DEPTH];
        let r = append(&mut filled, &z, 0, leaf(0), |_, _| Err("syscall failed"));
        assert_eq!(r, Err("syscall failed"));
    }
}
