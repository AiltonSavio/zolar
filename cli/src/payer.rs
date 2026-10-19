//! The Payer's side: build the PCZT, check the Escrow before paying, pay, wait for finality, claim.

use std::{
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use escrow_client::{
    accounts::Escrow,
    instructions::{DepositBondBuilder, PrivateClaimBuilder, RevealClaimBuilder},
};
use solana_address::Address;
use solana_keypair::Keypair;
use solana_signer::Signer;
use zolar_claim::{
    FinalBlockTree, block_leaf,
    private_claim::{ClaimTree, Witness, circuit_input, claim_window_lower},
    tx_merkle_path,
};
use zolar_pczt::summary;
use zolar_relayer::{chain::Cluster, p2p, rpc::HttpRpc};

use crate::{
    blocks::{Found, fetch_block_txids, fetch_headers, find_payment, header_hash, merkle_root},
    devtool::Devtool,
    internal_txid,
    prover::Prover,
    rpc::finalized_account_within,
    solana::{
        RelayState, associated_token_account, create_ata_idempotent, load_keypair,
        mint_freeze_authority, parse_token_account, read_account, read_escrow, read_relay, send,
    },
};

const POLL: Duration = Duration::from_secs(10);
/// Mainnet needs Finality Depth 20 blocks at about 75 to 90 s each, before any slow block.
const FINALITY_WAIT: Duration = Duration::from_secs(90 * 60);

pub struct Chains {
    pub rpc_url: String,
    pub explorer_url: String,
    pub relay: Address,
    pub peers: Vec<String>,
    pub network: p2p::Network,
}

/// Builds the unsigned PCZT: the Payee's address, a value, the challenge memo, and an expiry
/// `expiry_blocks` above the relay tip so the Escrow's 20 block limit holds.
pub fn create_pczt(
    chains: &Chains,
    devtool: &Devtool,
    address: &str,
    value: u64,
    memo: &str,
    expiry_blocks: u32,
    out: &Path,
) -> Result<()> {
    ensure!(
        (1..=20).contains(&expiry_blocks),
        "--expiry-blocks must be 1 to 20: the Escrow refuses an expiry further ahead"
    );
    let mut rpc = HttpRpc::new(&chains.rpc_url);
    let relay = read_relay(&mut rpc, &chains.relay)?;
    let tip = relay.view()?.tip_height();
    println!("syncing the wallet");
    devtool.sync()?;
    let expiry = tip + expiry_blocks;
    devtool.create(address, value, memo, expiry, out)?;
    let s = summary(&std::fs::read(out)?)?;
    println!("PCZT written to {}", out.display());
    println!("transaction ID : {}", s.txid);
    println!("expiry height  : {} (relay tip {tip})", s.expiry_height);
    Ok(())
}

/// What the Payer learned about an Escrow it is willing to pay into.
pub struct Checked {
    pub escrow: Escrow,
    pub escrow_address: Address,
    pub txid: String,
    pub expiry_height: u32,
}

pub struct Check<'a> {
    pub chains: &'a Chains,
    pub escrow_address: Address,
    pub payer_wallet: Address,
    pub pczt: &'a Path,
    pub salt: [u8; 32],
    pub accept_freeze_authority: bool,
    /// The token the Payer agreed to be paid in. The Payee's Escrow must hold exactly this mint.
    pub expected_mint: Address,
    /// The least tokens (smallest unit) the Payer agreed to be paid.
    pub min_amount: u64,
    /// The most the Payer agreed to lock as a Bond.
    pub max_bond: u64,
    /// How long to wait for the Escrow to be finalized on Solana.
    pub finalized_wait: Duration,
}

/// The terms of the trade as the Payer agreed them, against what the Payee locked.
pub fn check_terms(
    escrow: &Escrow,
    payer_wallet: &Address,
    relay: &Address,
    expected_mint: &Address,
    min_amount: u64,
    max_bond: u64,
) -> Result<()> {
    ensure!(
        escrow.state == 0,
        "the Escrow is not open (a refund may be under way)"
    );
    ensure!(
        &escrow.relay == relay,
        "the Escrow trusts relay {}, not the expected {relay}",
        escrow.relay
    );
    ensure!(
        &escrow.payout == payer_wallet,
        "the Escrow pays {}, not this Payer wallet {payer_wallet}",
        escrow.payout
    );
    ensure!(
        &escrow.mint == expected_mint,
        "the Escrow holds mint {}, not the agreed {expected_mint}",
        escrow.mint
    );
    ensure!(
        escrow.amount >= min_amount,
        "the Escrow locks {} tokens, fewer than the agreed {min_amount}",
        escrow.amount
    );
    ensure!(
        escrow.bond_required <= max_bond,
        "the Escrow asks for a Bond of {}, more than the agreed {max_bond}",
        escrow.bond_required
    );
    Ok(())
}

/// Waits until the Bond is in a finalized Escrow: once a Zcash payment is out, a rolled back deposit
/// could not be repeated after a refund request.
fn bond_is_finalized(chains: &Chains, escrow_address: Address, required: u64) -> Result<()> {
    let started = Instant::now();
    loop {
        let data =
            finalized_account_within(&chains.rpc_url, &escrow_address, Duration::from_secs(120))?;
        if read_escrow(&data)?.bond_deposited >= required {
            println!("the Bond is finalized on Solana");
            return Ok(());
        }
        ensure!(
            started.elapsed() < Duration::from_secs(120),
            "the Bond is not finalized on Solana after 2 minutes; do not pay yet"
        );
        sleep(Duration::from_secs(5));
    }
}

/// Everything a Payer verifies before signing. Read-only.
pub fn check_escrow(c: &Check) -> Result<Checked> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let s = summary(&std::fs::read(c.pczt)?)?;
    let url = &c.chains.rpc_url;
    let data = finalized_account_within(url, &c.escrow_address, c.finalized_wait)?;
    let escrow = read_escrow(&data)?;

    // Only the Escrow program can create the account at this address, so its contents are its own.
    ensure!(
        Escrow::find_pda(&escrow.payee, escrow.commitment).0 == c.escrow_address,
        "{} is not an Escrow account of the Escrow program",
        c.escrow_address
    );
    check_terms(
        &escrow,
        &c.payer_wallet,
        &c.chains.relay,
        &c.expected_mint,
        c.min_amount,
        c.max_bond,
    )?;
    let commitment = zolar_claim::payment_commitment(&internal_txid(&s.txid)?, &c.salt)?;
    ensure!(
        commitment == escrow.commitment,
        "the Escrow's Payment Commitment does not match this PCZT's transaction ID and the salt"
    );
    ensure!(
        escrow.expiry_height == s.expiry_height,
        "the Escrow expires at {} but the transaction at {}: a payment mined in between would be lost",
        escrow.expiry_height,
        s.expiry_height
    );

    let vault = parse_token_account(&finalized_account_within(
        url,
        &escrow.vault,
        c.finalized_wait,
    )?)?;
    ensure!(
        vault.owner == c.escrow_address && vault.mint == escrow.mint,
        "the vault is not a token account of the Escrow for its mint"
    );
    ensure!(
        vault.amount >= escrow.amount && !vault.has_delegate && !vault.has_close_authority,
        "the vault does not hold the tokens or has a delegate or close authority"
    );

    let mint = finalized_account_within(url, &escrow.mint, c.finalized_wait)?;
    match mint_freeze_authority(&mint)? {
        Some(authority) if !c.accept_freeze_authority => bail!(
            "the mint has a freeze authority ({authority}); it could freeze the vault after you pay. \
             Pass --accept-freeze-authority if you trust it"
        ),
        Some(authority) => println!("warning: the mint has a freeze authority ({authority})"),
        None => println!("the mint has no freeze authority"),
    }

    let relay = read_relay(&mut rpc, &escrow.relay)?;
    let view = relay.view()?;
    view.ensure_running()
        .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
    ensure!(
        view.tip_height() < s.expiry_height,
        "the relay tip {} is already at the expiry {}",
        view.tip_height(),
        s.expiry_height
    );

    println!("Escrow {} checked", c.escrow_address);
    println!("  mint            : {}", escrow.mint);
    println!(
        "  amount          : {} (vault holds {})",
        escrow.amount, vault.amount
    );
    println!("  payout          : {}", escrow.payout);
    println!(
        "  Bond            : {} required, {} deposited",
        escrow.bond_required, escrow.bond_deposited
    );
    println!("  transaction ID  : {}", s.txid);
    println!(
        "  expiry          : {} (relay tip {}, Finality Depth {})",
        s.expiry_height,
        view.tip_height(),
        view.finality_depth()
    );
    Ok(Checked {
        txid: s.txid,
        expiry_height: s.expiry_height,
        escrow,
        escrow_address: c.escrow_address,
    })
}

/// Locks the Bond the Escrow requires, if it is not locked yet. A Payer calls this before broadcasting:
/// a Reveal Claim cannot settle without it, and it returns with the tokens.
pub fn deposit_bond(chains: &Chains, keypair: &Path, escrow_address: Address) -> Result<()> {
    let mut rpc = HttpRpc::new(&chains.rpc_url);
    let payer = load_keypair(keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &escrow_address)?)?;
    ensure!(
        escrow.payout == payer.pubkey(),
        "the Escrow pays {}, not this keypair; only that wallet can lock the Bond",
        escrow.payout
    );
    if escrow.bond_required == 0 {
        println!("the Escrow requires no Bond");
        return Ok(());
    }
    if escrow.bond_deposited >= escrow.bond_required {
        println!("the Bond of {} is already locked", escrow.bond_deposited);
        return bond_is_finalized(chains, escrow_address, escrow.bond_required);
    }
    let source = associated_token_account(&payer.pubkey(), &escrow.mint);
    let held = parse_token_account(&read_account(&mut rpc, &source)?)
        .context("the Payer has no token account for the Escrow's mint, so no tokens for the Bond")?
        .amount;
    ensure!(
        held >= escrow.bond_required,
        "the Payer holds {held} tokens, fewer than the {} Bond",
        escrow.bond_required
    );
    let ix = DepositBondBuilder::new()
        .payer(payer.pubkey())
        .escrow(escrow_address)
        .payer_token(source)
        .vault(escrow.vault)
        .instruction();
    let signature = send(&mut rpc, &[ix], &payer)?;
    println!(
        "Bond locked         : {signature} ({} tokens)",
        escrow.bond_required
    );
    bond_is_finalized(chains, escrow_address, escrow.bond_required)
}

/// Proves, signs and broadcasts the checked PCZT. It does not look the payment up anywhere: asking an
/// explorer for the transaction would tell it which payment the next Claim belongs to. `claim` and
/// `private-claim` find the block themselves from Zcash P2P blocks.
pub fn pay(devtool: &Devtool, checked: &Checked, pczt: &Path, work: &Path) -> Result<()> {
    std::fs::create_dir_all(work)?;
    let proven = work.join("proven.pczt");
    let signed = work.join("signed.pczt");
    let combined = work.join("combined.pczt");
    println!("proving");
    devtool.prove(pczt, &proven)?;
    println!("signing");
    devtool.sign(&proven, &signed)?;
    devtool.combine(&proven, &signed, &combined)?;
    ensure!(
        summary(&std::fs::read(&combined)?)?.txid == checked.txid,
        "the signed transaction has another ID than the committed one"
    );
    println!("broadcasting {}", checked.txid);
    devtool.send(&combined)?;
    Ok(())
}

/// The Final Block Tree rebuilt from Zcash P2P headers, proven equal to the relay's.
pub struct Rebuilt {
    pub headers: Vec<p2p::RawHeader>,
    pub tree: FinalBlockTree,
    pub root: [u8; 32],
}

/// Rebuilds the relay's Final Block Tree from the headers of the blocks it holds as final and checks
/// the root against the relay's own. A mismatch means the peers' chain is not the relay's.
pub fn rebuild_tree(chains: &Chains, relay: &RelayState) -> Result<Rebuilt> {
    let view = relay.view()?;
    let checkpoint = relay.account()?.checkpoint;
    let size = view.leaf_count() as usize;
    let headers = fetch_headers(checkpoint.hash, size, &chains.peers, chains.network)?;
    let leaves = headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            Ok(block_leaf(
                checkpoint.height + 1 + i as u32,
                &merkle_root(h),
            )?)
        })
        .collect::<Result<Vec<_>>>()?;
    let tree = FinalBlockTree::new(leaves);
    let root = tree.root(size);
    ensure!(
        view.root_at(size as u32) == Some(root),
        "the tree rebuilt from Zcash P2P headers does not equal the relay's root: the peers' chain \
         differs from the relay's"
    );
    println!(
        "rebuilt {size} leaves from P2P headers; the root {} equals the relay's",
        hex::encode(root)
    );
    Ok(Rebuilt {
        headers,
        tree,
        root,
    })
}

pub struct Claim<'a> {
    pub chains: &'a Chains,
    pub keypair: PathBuf,
    pub escrow_address: Address,
    pub txid: &'a str,
    pub salt: [u8; 32],
}

/// The payment's block, found and tied to the relay's Final Block Tree.
struct Located {
    height: u32,
    /// Position of the block's leaf in the tree (`height - checkpoint_height - 1`).
    leaf: usize,
    /// Leaves of the tree the relay's root was checked against.
    size: usize,
    tree: FinalBlockTree,
    root: [u8; 32],
    txids: Vec<[u8; 32]>,
    tx_index: usize,
}

/// Waits for the payment's block to enter the Final Block Tree and finds it. The Payer asks Zcash P2P
/// peers for the blocks of the Escrow's height window one after another, in order, each by hash, and
/// looks for its own transaction ID in them locally: no explorer or other third party is told which
/// transaction or which height matters. Every block is checked against the header's Merkle root, the
/// headers against the relay's own root.
fn locate_payment(
    chains: &Chains,
    rpc: &mut HttpRpc,
    escrow: &Escrow,
    txid: &[u8; 32],
) -> Result<Located> {
    let started = Instant::now();
    let mut next_height: Option<u32> = None;
    let mut cached: Option<(u32, Rebuilt)> = None;
    loop {
        let relay = read_relay(rpc, &chains.relay)?;
        let view = relay.view()?;
        view.ensure_running()
            .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
        let checkpoint = relay.account()?.checkpoint;
        let size = view.leaf_count();
        let lower = claim_window_lower(escrow.created_tip, view.finality_depth());
        let first = next_height.unwrap_or(lower.max(checkpoint.height + 1));
        let last = (checkpoint.height + size).min(escrow.expiry_height);
        if size > 0 && first <= last {
            if cached.as_ref().map(|(n, _)| *n) != Some(size) {
                cached = Some((size, rebuild_tree(chains, &relay)?));
            }
            let (_, rebuilt) = cached.as_ref().expect("just set");
            let leaf_of = |h: u32| (h - checkpoint.height - 1) as usize;
            let hashes: Vec<[u8; 32]> = (first..=last)
                .map(|h| header_hash(&rebuilt.headers[leaf_of(h)]))
                .collect();
            let scan = find_payment(&hashes, txid, |hash| {
                fetch_block_txids(*hash, &chains.peers, chains.network)
            });
            if let Some(Found {
                candidate,
                txids,
                tx_index,
            }) = scan.found
            {
                let height = first + candidate as u32;
                let (size, rebuilt) = cached.take().expect("just set");
                println!("the payment is in a final block; building the proof");
                return Ok(Located {
                    height,
                    leaf: leaf_of(height),
                    size: size as usize,
                    tree: rebuilt.tree,
                    root: rebuilt.root,
                    txids,
                    tx_index,
                });
            }
            next_height = Some(first + scan.scanned as u32);
            if let Some(why) = scan.failure {
                println!("could not read the next block from Zcash peers ({why}); will retry");
            }
        }
        ensure!(
            next_height.is_none_or(|n| n <= escrow.expiry_height),
            "the payment is in none of the final blocks up to the Escrow's expiry height {}; it was \
             not mined in time",
            escrow.expiry_height
        );
        println!(
            "waiting for finality: relay tip {}, final up to {}, looked through {}",
            view.tip_height(),
            view.finalized_height(),
            next_height.map_or(0, |n| n - 1)
        );
        ensure!(
            started.elapsed() < FINALITY_WAIT,
            "the payment did not appear in a final block in {} minutes",
            FINALITY_WAIT.as_secs() / 60
        );
        sleep(POLL);
    }
}

fn payout_account(rpc: &mut HttpRpc, payer: &Keypair, escrow: &Escrow) -> Result<Address> {
    let payout_token = associated_token_account(&payer.pubkey(), &escrow.mint);
    // The Payer's token account is made in its own transaction: with it the claim would not fit one.
    if rpc
        .account(&payout_token)
        .map_err(anyhow::Error::msg)?
        .is_none()
    {
        let ata = create_ata_idempotent(&payer.pubkey(), &payer.pubkey(), &escrow.mint);
        let created = send(rpc, &[ata], payer)?;
        println!("Payer token account : {payout_token} ({created})");
    }
    Ok(payout_token)
}

/// Waits for the payment's block to be final, finds it, builds the Inclusion Proof and the tree path,
/// checks both against the relay's own root, and sends the Reveal Claim. The Reveal Claim publishes
/// the transaction ID and salt on Solana; use [`private_claim`] to keep them off the chain.
pub fn reveal_claim(c: &Claim) -> Result<String> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let payer = load_keypair(&c.keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &c.escrow_address)?)?;
    let me = internal_txid(c.txid)?;
    let found = locate_payment(c.chains, &mut rpc, &escrow, &me)?;
    let payout_token = payout_account(&mut rpc, &payer, &escrow)?;
    let claim = RevealClaimBuilder::new()
        .escrow(c.escrow_address)
        .relay(c.chains.relay)
        .vault(escrow.vault)
        .payout_token(payout_token)
        .payee(escrow.payee)
        .txid(me)
        .salt(c.salt)
        .block_height(found.height)
        .tx_index(found.tx_index as u32)
        .root(found.root)
        .tree_path(
            found
                .tree
                .compress(found.tree.path(found.leaf, found.size))
                .into(),
        )
        .tx_path(tx_merkle_path(&found.txids, found.tx_index).into())
        .instruction();
    let signature = send(&mut rpc, &[claim], &payer)?;
    println!("Reveal Claim        : {signature}");
    Ok(signature)
}

/// The relay keeps its last 128 roots. A proof takes a minute; if the root it names has aged out by the
/// time it is ready, or the program says so, the Private Claim starts again from a fresh root.
#[derive(Debug)]
pub struct RootAgedOut;

impl std::fmt::Display for RootAgedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the root fell out of the relay's history of 128 roots")
    }
}

impl std::error::Error for RootAgedOut {}

/// Runs `attempt` until it stops failing with [`RootAgedOut`], at most `attempts` times.
pub fn retry_aged_root<T>(attempts: u32, mut attempt: impl FnMut() -> Result<T>) -> Result<T> {
    let mut left = attempts;
    loop {
        match attempt() {
            Err(e) if e.downcast_ref::<RootAgedOut>().is_some() && left > 1 => {
                left -= 1;
                println!("{e}; proving again against a fresh root ({left} attempts left)");
            }
            other => return other,
        }
    }
}

pub struct PrivateClaimArgs<'a> {
    pub claim: Claim<'a>,
    pub prover: &'a Prover,
    /// Directory for the proof's temporary files (the witness is deleted after proving).
    pub work_dir: PathBuf,
    pub max_attempts: u32,
}

/// Sends a Private Claim: finds the payment's block, builds the witness and proves it on this
/// machine, then submits only the 288 bytes of proof and root. The transaction ID, salt, block height
/// and Merkle paths stay in this process.
pub fn private_claim(a: &PrivateClaimArgs) -> Result<String> {
    let c = &a.claim;
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let payer = load_keypair(&c.keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &c.escrow_address)?)?;
    ensure!(
        escrow.payout == payer.pubkey(),
        "the Escrow pays {}, not this keypair",
        escrow.payout
    );
    let me = internal_txid(c.txid)?;
    ensure!(
        zolar_claim::payment_commitment(&me, &c.salt)? == escrow.commitment,
        "this transaction ID and salt do not match the Escrow's Payment Commitment"
    );
    // Found once: blocks do not change. Only the root may need refreshing.
    let found = locate_payment(c.chains, &mut rpc, &escrow, &me)?;
    let payout_token = payout_account(&mut rpc, &payer, &escrow)?;

    retry_aged_root(a.max_attempts, || {
        let relay = read_relay(&mut rpc, &c.chains.relay)?;
        let view = relay.view()?;
        view.ensure_running()
            .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
        // The newest root the tree can be rebuilt for; it stays among the last 128 for about two hours.
        let size = view.leaf_count() as usize;
        let rebuilt;
        let tree = if size == found.size {
            &found.tree
        } else {
            rebuilt = rebuild_tree(c.chains, &relay)?;
            &rebuilt.tree
        };
        let setup = ProofSetup {
            tree,
            size,
            finality_depth: view.finality_depth(),
        };
        prove_and_send(a, &mut rpc, &payer, &escrow, &payout_token, &found, &setup)
    })
}

struct ProofSetup<'a> {
    tree: &'a FinalBlockTree,
    size: usize,
    finality_depth: u16,
}

fn prove_and_send(
    a: &PrivateClaimArgs,
    rpc: &mut HttpRpc,
    payer: &Keypair,
    escrow: &Escrow,
    payout_token: &Address,
    found: &Located,
    setup: &ProofSetup,
) -> Result<String> {
    let c = &a.claim;
    let me = internal_txid(c.txid)?;
    let input = circuit_input(&Witness {
        txids: &found.txids,
        index: found.tx_index,
        salt: c.salt,
        height: found.height,
        tx_depth: a.prover.depth,
        tree: ClaimTree {
            tree: setup.tree,
            size: setup.size,
            leaf: found.leaf,
        },
        lower: claim_window_lower(escrow.created_tip, setup.finality_depth),
        expiry: escrow.expiry_height,
    })
    .map_err(|e| {
        anyhow::anyhow!(
            "cannot build the Private Claim witness: {e} (a block with more than 2^{} transactions \
             needs a Reveal Claim, which publishes the transaction ID)",
            a.prover.depth
        )
    })?;
    debug_assert_eq!(
        zolar_claim::payment_commitment(&me, &c.salt)?,
        input.public.commitment
    );
    println!("proving on this machine (no network is used)");
    let proof = a.prover.prove(&input, &a.work_dir)?;
    println!("proof ready in {:.1} s", proof.took.as_secs_f64());

    // The relay moved on while proving: is the root still one of its last 128?
    let relay = read_relay(rpc, &c.chains.relay)?;
    if !relay.view()?.is_known_root(&input.public.root) {
        return Err(RootAgedOut.into());
    }
    let claim = PrivateClaimBuilder::new()
        .escrow(c.escrow_address)
        .relay(c.chains.relay)
        .vault(escrow.vault)
        .payout_token(*payout_token)
        .payee(escrow.payee)
        .proof_a(proof.bytes.a)
        .proof_b(proof.bytes.b)
        .proof_c(proof.bytes.c)
        .root(input.public.root)
        .instruction();
    match send(rpc, &[claim], payer) {
        Ok(signature) => {
            println!("Private Claim       : {signature}");
            Ok(signature)
        }
        Err(e) if e.to_string().contains("UnknownRoot") => Err(RootAgedOut.into()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escrow() -> Escrow {
        Escrow {
            state: 0,
            payee: Address::new_from_array([1; 32]),
            payout: Address::new_from_array([2; 32]),
            mint: Address::new_from_array([3; 32]),
            relay: Address::new_from_array([4; 32]),
            amount: 5_000_000,
            ..Escrow::from_bytes(&sample()).unwrap()
        }
    }

    fn sample() -> Vec<u8> {
        let mut d = vec![0u8; 300];
        d[0] = 1;
        d[1] = 1;
        d
    }

    #[test]
    fn an_aged_out_root_is_proved_again_and_other_errors_are_not_retried() {
        let mut calls = 0;
        let out = retry_aged_root(3, || {
            calls += 1;
            if calls < 3 {
                Err(RootAgedOut.into())
            } else {
                Ok(calls)
            }
        });
        assert_eq!(out.unwrap(), 3);

        let mut calls = 0;
        let out: Result<()> = retry_aged_root(3, || {
            calls += 1;
            Err(RootAgedOut.into())
        });
        assert_eq!(calls, 3, "gives up after the attempts");
        assert!(out.unwrap_err().downcast_ref::<RootAgedOut>().is_some());

        let mut calls = 0;
        let out: Result<()> = retry_aged_root(3, || {
            calls += 1;
            bail!("the proof was rejected")
        });
        assert_eq!(calls, 1);
        assert!(out.is_err());
    }

    #[test]
    fn the_payer_pays_only_into_the_agreed_mint_and_amount() {
        let e = escrow();
        let (payer, relay, mint) = (e.payout, e.relay, e.mint);
        check_terms(&e, &payer, &relay, &mint, 5_000_000, 0).unwrap();
        check_terms(&e, &payer, &relay, &mint, 1, 0).unwrap();
        // A worthless token locked against the right payout and relay.
        let other = Address::new_from_array([9; 32]);
        assert!(check_terms(&e, &payer, &relay, &other, 1, 0).is_err());
        // Underfunded.
        assert!(check_terms(&e, &payer, &relay, &mint, 5_000_001, 0).is_err());
        // Another payout wallet, another relay, a required Bond, a closed Escrow.
        assert!(check_terms(&e, &other, &relay, &mint, 1, 0).is_err());
        assert!(check_terms(&e, &payer, &other, &mint, 1, 0).is_err());
        // A Bond above the agreed maximum is refused, one within it accepted.
        let with_bond = Escrow {
            bond_required: 100,
            ..e.clone()
        };
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 99).is_err());
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 100).is_ok());
        assert!(check_terms(&with_bond, &payer, &relay, &mint, 1, 0).is_err());
        let closed = Escrow { state: 1, ..e };
        assert!(check_terms(&closed, &payer, &relay, &mint, 1, 0).is_err());
    }
}
