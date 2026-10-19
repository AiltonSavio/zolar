//! The Payer's side: build the PCZT, check the Escrow before paying, pay, wait for finality, claim.

use std::{
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use escrow_client::{accounts::Escrow, instructions::RevealClaimBuilder};
use solana_address::Address;
use solana_signer::Signer;
use zolar_claim::{FinalBlockTree, block_leaf, tx_merkle_path};
use zolar_pczt::summary;
use zolar_relayer::rpc::HttpRpc;

use crate::{
    blocks::{fetch_headers, merkle_root, verified_block_txids},
    devtool::Devtool,
    explorer::Explorer,
    internal_txid,
    solana::{
        associated_token_account, create_ata_idempotent, load_keypair, mint_freeze_authority,
        parse_token_account, read_account, read_escrow, read_relay, send,
    },
};

const POLL: Duration = Duration::from_secs(10);

pub struct Chains {
    pub rpc_url: String,
    pub explorer_url: String,
    pub relay: Address,
    pub peers: Vec<String>,
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
}

/// Everything a Payer verifies before signing. Read-only.
pub fn check_escrow(c: &Check) -> Result<Checked> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let s = summary(&std::fs::read(c.pczt)?)?;
    let data = read_account(&mut rpc, &c.escrow_address)?;
    let escrow = read_escrow(&data)?;

    // Only the Escrow program can create the account at this address, so its contents are its own.
    ensure!(
        Escrow::find_pda(&escrow.payee, escrow.commitment).0 == c.escrow_address,
        "{} is not an Escrow account of the Escrow program",
        c.escrow_address
    );
    ensure!(escrow.state == 0, "the Escrow is not open");
    ensure!(
        escrow.relay == c.chains.relay,
        "the Escrow trusts relay {}, not the expected {}",
        escrow.relay,
        c.chains.relay
    );
    ensure!(
        escrow.payout == c.payer_wallet,
        "the Escrow pays {}, not this Payer wallet {}",
        escrow.payout,
        c.payer_wallet
    );
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
    ensure!(
        escrow.bond_required == 0,
        "the Escrow asks for a Bond; this version cannot lock one"
    );

    let vault = parse_token_account(&read_account(&mut rpc, &escrow.vault)?)?;
    ensure!(
        vault.owner == c.escrow_address && vault.mint == escrow.mint,
        "the vault is not a token account of the Escrow for its mint"
    );
    ensure!(
        vault.amount >= escrow.amount && !vault.has_delegate && !vault.has_close_authority,
        "the vault does not hold the tokens or has a delegate or close authority"
    );

    let mint = read_account(&mut rpc, &escrow.mint)?;
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

/// Proves, signs and broadcasts the checked PCZT, then waits until the explorer shows it mined.
/// Returns the block height.
pub fn pay(
    chains: &Chains,
    devtool: &Devtool,
    checked: &Checked,
    pczt: &Path,
    work: &Path,
) -> Result<u32> {
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
    wait_until_mined(chains, &checked.txid)
}

pub fn wait_until_mined(chains: &Chains, txid: &str) -> Result<u32> {
    let explorer = Explorer::new(&chains.explorer_url);
    let started = Instant::now();
    loop {
        if let Some(mined) = explorer.mined(txid)? {
            println!("mined in block {}", mined.block_height);
            return Ok(mined.block_height);
        }
        ensure!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "transaction {txid} was not mined in 30 minutes"
        );
        sleep(POLL);
    }
}

pub struct Claim<'a> {
    pub chains: &'a Chains,
    pub keypair: PathBuf,
    pub escrow_address: Address,
    pub txid: &'a str,
    pub block_height: u32,
    pub salt: [u8; 32],
}

/// Waits until the block is in the Final Block Tree, builds the Inclusion Proof and the tree path from
/// data outside Solana, checks both against the relay's own root, and sends the Reveal Claim.
pub fn reveal_claim(c: &Claim) -> Result<String> {
    let mut rpc = HttpRpc::new(&c.chains.rpc_url);
    let payer = load_keypair(&c.keypair)?;
    let escrow = read_escrow(&read_account(&mut rpc, &c.escrow_address)?)?;

    let started = Instant::now();
    let relay = loop {
        let relay = read_relay(&mut rpc, &c.chains.relay)?;
        let view = relay.view()?;
        view.ensure_running()
            .map_err(|e| anyhow::anyhow!("the relay is halted: {e:?}"))?;
        if view.finalized_height() >= c.block_height {
            break relay;
        }
        println!(
            "waiting for finality: block {}, relay tip {}, final up to {}",
            c.block_height,
            view.tip_height(),
            view.finalized_height()
        );
        ensure!(
            started.elapsed() < Duration::from_secs(30 * 60),
            "the block did not become final in 30 minutes"
        );
        sleep(POLL);
    };
    let view = relay.view()?;
    let checkpoint = relay.account()?.checkpoint;
    let size = view.leaf_count() as usize;
    let index = (c.block_height - checkpoint.height - 1) as usize;
    ensure!(index < size, "the block is not in the tree yet");
    println!("final block tree: {size} leaves, the block is leaf {index}");

    let headers = fetch_headers(checkpoint.hash, size, &c.chains.peers)?;
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
    println!("rebuilt root equals the relay's root {}", hex::encode(root));

    let txids = verified_block_txids(
        &Explorer::new(&c.chains.explorer_url),
        c.block_height,
        &merkle_root(&headers[index]),
    )?;
    let me = internal_txid(c.txid)?;
    let tx_index = txids
        .iter()
        .position(|t| *t == me)
        .with_context(|| format!("{} is not in block {}", c.txid, c.block_height))?;
    println!(
        "Inclusion Proof: transaction {tx_index} of {} in block {}",
        txids.len(),
        c.block_height
    );

    let payout_token = associated_token_account(&payer.pubkey(), &escrow.mint);
    let claim = RevealClaimBuilder::new()
        .escrow(c.escrow_address)
        .relay(c.chains.relay)
        .vault(escrow.vault)
        .payout_token(payout_token)
        .payee(escrow.payee)
        .txid(me)
        .salt(c.salt)
        .block_height(c.block_height)
        .tx_index(tx_index as u32)
        .root(root)
        .tree_path(tree.compress(tree.path(index, size)).into())
        .tx_path(tx_merkle_path(&txids, tx_index).into())
        .instruction();
    let ata = create_ata_idempotent(&payer.pubkey(), &payer.pubkey(), &escrow.mint);
    let signature = send(&mut rpc, &[ata, claim], &payer)?;
    println!("Reveal Claim        : {signature}");
    Ok(signature)
}
