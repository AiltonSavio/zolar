import { createFromRoot } from 'codama';
import { renderVisitor as renderJs } from '@codama/renderers-js';
import { renderVisitor as renderRust } from '@codama/renderers-rust';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { root as headerRelay } from './program.mjs';
import { root as escrow } from './escrow.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const render = (root, rustDir, jsDir) => {
  const codama = createFromRoot(root);
  codama.accept(renderRust(path.join(here, rustDir), { formatCode: true }));
  codama.accept(renderJs(path.join(here, jsDir), { formatCode: false }));
};
render(headerRelay, '../rust', '../js');
render(escrow, '../escrow-rust', '../escrow-js');
