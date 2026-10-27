# Vending JS client

`src/generated` is copied byte-for-byte from Zolar's vending program branch at
`54bfbb07c42850d69820151a24c3d9c9cc29af89`. Codama owns these files; change the
program branch's `clients/codama/vending.mjs` and regenerate instead of editing
them. The program branch supplies the matching instruction/account interface.

```sh
pnpm --dir clients/vending-js install --frozen-lockfile
pnpm --dir clients/vending-js build
pnpm --dir sdk install --frozen-lockfile
```

The package builds declarations with TypeScript and bundles its entry point as
ESM with external Solana dependencies. This keeps the generated files unchanged
and lets the SDK retain its stricter optional-property checks. When integrating
the program branch, preserve this package metadata and build configuration;
its generated source is identical. Build this repository-local dependency before
installing the SDK or shop on a clean checkout.
