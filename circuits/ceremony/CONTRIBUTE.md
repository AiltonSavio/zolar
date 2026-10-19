# Contribute to the Zolar key ceremony

You add one piece of randomness to the key that checks Zolar's private payments. If **you** keep it secret,
and then delete it, nobody can forge a payment proof, whatever the other contributors do. You do not need to
understand the cryptography, and nothing you do can break the key: a bad or malicious contribution is detected
and rejected by the coordinator.

## What you need

- A Linux, macOS or WSL machine you trust. Your own laptop is fine; a freshly rebooted one is better.
- **Node 22** and **pnpm** (`corepack enable` gives you pnpm).
- **4 GB of free memory**, **2 GB of free disk**, and a connection that can download about 410 MB and upload about 410 MB.
- **About 20 minutes** at the keyboard for your turn (most of it is the machine working; about 6 minutes on a
  fast 8-core laptop, 15 to 20 on an old one). We schedule one contributor at a time. **Your slot is 2 hours
  from the moment the coordinator sends you the link**, so you do not have to rush; tell the coordinator if you
  will miss it and the turn moves to the next person.

## Steps

1. Get the tool (once):

   ```sh
   git clone <repo url> zolar && cd zolar
   pnpm --dir circuits install --frozen-lockfile
   ```

2. The coordinator sends you your exact command. It looks like this; the long hash is how the tool checks
   it downloaded the right previous file:

   ```sh
   node circuits/ceremony/ceremony.mjs contribute \
     --in https://github.com/<org>/zolar/releases/download/ceremony-r2/claim_0002.zkey \
     --sha256 <64 hex characters> --name "Your name or handle"
   ```

   It downloads the file, asks you to **mash the keyboard and type any random text** (it is mixed into the
   computer's own randomness and never saved), then works for a few minutes and prints:

   ```
   contribution hash (publish this):
     <128 hex characters>
   file sha256: <64 hex characters>
   ```

   If memory is tight, set `ZOLAR_THREADS=2` before the command; it is slower but lighter.

3. **Send the file** `my_contribution.zkey` (it is public data, not a secret) to the coordinator: upload it
   anywhere that gives a download link (your own GitHub release, Google Drive or Dropbox "anyone with the link",
   `wormhole send`, a file host) and send the link and the `file sha256` to the coordinator.

4. **Publish your contribution hash under your own name**, somewhere you control and that cannot be edited
   silently: a post on X or Mastodon, a GitHub gist, a signed message. Include your name, the contribution
   hash, the file sha256 and the line "Zolar ceremony contribution". This is what lets anyone, you included,
   check later that the coordinator did not drop or swap your contribution.

5. **Get rid of your randomness.** It exists only while the command runs (in memory, plus the keyboard text you
   typed). When it finishes, close the terminal and, if you can, reboot. Delete `previous.zkey` and your
   copy of `my_contribution.zkey` once the coordinator has confirmed receipt; keep the `.receipt.txt` (it is
   public). The ceremony is safe if *at least one* contributor did this honestly, so do not worry about
   perfection: just do not save, share or write down what you typed, and do not run it inside a VM
   that gets snapshotted or on a machine that hibernates mid-run.

6. When the ceremony is finished, run `node circuits/ceremony/ceremony.mjs verify` (see [README.md](README.md))
   or just compare your published hash with the line for your name in `circuits/ceremony/transcript.json`.

## If something goes wrong

- *sha256 mismatch*: the link or hash is wrong; ask the coordinator. Never contribute on a file whose hash you
  were not given.
- *Out of memory / killed*: close your browser, or run with `ZOLAR_THREADS=2`. You can retry from step 2; a
  failed run leaves nothing behind that matters.
- You may contribute once. If your run crashed after printing a hash, do not reuse that file: tell the coordinator.
