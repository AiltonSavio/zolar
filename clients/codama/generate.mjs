import { createFromRoot } from 'codama';
import { renderVisitor as renderJs } from '@codama/renderers-js';
import { renderVisitor as renderRust } from '@codama/renderers-rust';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { root } from './program.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const codama = createFromRoot(root);
codama.accept(renderRust(path.join(here, '../rust'), { formatCode: true }));
codama.accept(renderJs(path.join(here, '../js'), { formatCode: false }));
