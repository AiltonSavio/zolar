import { createFromRoot } from 'codama';
import { renderVisitor as renderJs } from '@codama/renderers-js';
import { renderVisitor as renderRust } from '@codama/renderers-rust';
import { fileURLToPath } from 'node:url';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { root as headerRelay } from './program.mjs';
import { root as escrow } from './escrow.mjs';
import { root as vending } from './vending.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const render = async (root, rustDir, jsDir) => {
  const codama = createFromRoot(root);
  await codama.accept(renderRust(path.join(here, rustDir), { formatCode: true }));
  await codama.accept(renderJs(path.join(here, jsDir), { formatCode: false }));
  const generated = path.join(here, jsDir, 'src/generated');
  for (const name of readdirSync(generated, { recursive: true })) {
    if (!name.endsWith('.ts')) continue;
    const file = path.join(generated, name);
    const original = readFileSync(file, 'utf8');
    const normalized = original.replace(/[ \t]+$/gm, '');
    if (normalized !== original) writeFileSync(file, normalized);
  }
};
await render(headerRelay, '../rust', '../js');
await render(escrow, '../escrow-rust', '../escrow-js');

await render(vending, '../vending-rust', '../vending-js');
