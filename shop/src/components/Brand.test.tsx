import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { Brand } from './Brand';

afterEach(cleanup);

it('uses the approved Font 1 wordmark in both brand sizes', () => {
  // Source: "Font 1 · Barlow Semi Condensed ExtraBold" on the Zolar checkout mockups canvas
  // (t16/zolar-lockup-font1-approved.png), cropped to the wordmark with white made transparent and the Z–o gap cut from 83px to 9px.
  const artwork = readFileSync('public/zolar-wordmark.png');
  expect(createHash('sha256').update(artwork).digest('hex')).toBe(
    '8bd7678bd71c632c1a59f38c32e798dc6097b46ca62a52bdc28cb2615389c341',
  );
  for (const size of ['large', 'small'] as const) {
    render(<Brand size={size} />);
    expect(screen.getByRole('img', { name: 'zolar' }).getAttribute('src')).toBe('/zolar-wordmark.png?v=font-1-9px');
    cleanup();
  }
});
