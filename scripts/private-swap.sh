#!/usr/bin/env bash
# One real swap settled by a Private Claim (Zcash testnet TAZ for devnet tokens), then the txid check.
# Keys and wallets are passed by path through the environment; nothing secret is read here.
#
# Required environment:
#   ZOLAR_DEVTOOL ZOLAR_ZCASH_WALLET ZOLAR_ZCASH_IDENTITY   the Payer's zcash-devtool (patched) and wallet
#   ZOLAR_PAYER_KEYPAIR ZOLAR_PAYEE_KEYPAIR                 Solana keypair files
#   PAYEE_UA_FILE PAYEE_UFVK_FILE                           the Payee's testnet address and viewing key
#   MINT                                                    token mint (the Payee must hold AMOUNT of it)
#   ZOLAR_DATA                                              circuit artifacts (see circuits/README.md)
# Optional: BIN (directory of zolar-payer and zolar-payee, default cli/target/release), AMOUNT (5000000),
#   VALUE (zatoshis, 100000), RUN (output directory), PRIVATE_ONLY=0 to allow the Reveal Claim as well.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${BIN:-$ROOT/cli/target/release}"
AMOUNT="${AMOUNT:-5000000}"
VALUE="${VALUE:-100000}"
RUN="${RUN:-$(mktemp -d)}"
mkdir -p "$RUN"
MEMO="zolar private swap $(head -c 6 /dev/urandom | od -An -tx1 | tr -d ' \n')"
PAYER_WALLET=$(solana-keygen pubkey "$ZOLAR_PAYER_KEYPAIR")
PRIVATE_FLAG=--private-only; [ "${PRIVATE_ONLY:-1}" = 0 ] && PRIVATE_FLAG=
echo "run dir $RUN, memo '$MEMO'"

date +%T; echo "== 1. Payer builds the PCZT"
"$BIN/zolar-payer" create-pczt --address "$(cat "$PAYEE_UA_FILE")" --value "$VALUE" --memo "$MEMO" --expiry-blocks 18 --output "$RUN/created.pczt" | tee "$RUN/create-pczt.log"
date +%T; echo "== 2. Payee creates the Escrow"
"$BIN/zolar-payee" create-escrow --pczt "$RUN/created.pczt" --ufvk-file "$PAYEE_UFVK_FILE" --expect-memo "$MEMO" --min-zats "$VALUE" --payout "$PAYER_WALLET" --mint "$MINT" --amount "$AMOUNT" --salt-out "$RUN/salt.txt" $PRIVATE_FLAG | tee "$RUN/escrow.log"
ESCROW=$(grep '^Escrow created' "$RUN/escrow.log" | awk '{print $NF}')
date +%T; echo "== 3. Payer checks, pays"
"$BIN/zolar-payer" pay --mint "$MINT" --min-amount "$AMOUNT" --escrow "$ESCROW" --pczt "$RUN/created.pczt" --salt-file "$RUN/salt.txt" --work-dir "$RUN/work" | tee "$RUN/pay.log"
date +%T; echo "== 4. Payer waits for finality, proves locally and sends the Private Claim"
"$BIN/zolar-payer" private-claim --escrow "$ESCROW" --pczt "$RUN/created.pczt" --salt-file "$RUN/salt.txt" --work-dir "$RUN/proof" | tee "$RUN/claim.log"
date +%T; echo "== 5. The Zcash transaction ID must appear nowhere on Solana"
TXID=$(grep '^transaction ID' "$RUN/create-pczt.log" | awk '{print $NF}')
CREATE_SIG=$(grep '^signature' "$RUN/escrow.log" | awk '{print $NF}')
CLAIM_SIG=$(grep '^Private Claim' "$RUN/claim.log" | awk '{print $NF}')
"$ROOT/scripts/check-txid-absent.sh" "$TXID" --address "$ESCROW" "$CREATE_SIG" "$CLAIM_SIG" | tee "$RUN/check.log"
echo "txid $TXID escrow $ESCROW create $CREATE_SIG claim $CLAIM_SIG" | tee "$RUN/summary.txt"
